use crate::{CostModelError, TraceWorkloadReport};
use zk_jam_openvm_backend::{M4PublicValuesV1, OpenVmBackend};
use zk_jam_translation::{
    execute_reference, input_commitment, program_commitment, ExecutionInputV1,
};

pub fn measure(
    workload: &crate::workload::CostWorkload,
    backend: &OpenVmBackend,
) -> Result<TraceWorkloadReport, CostModelError> {
    let lowered = zk_jam_openvm_backend::native_pvm::NativePvmLowerer::default()
        .lower(&workload.program, workload.output_register)?;
    let predicted_static_instruction_count = lowered.openvm_instruction_count;
    let artifact = backend.program_from_vm_exe(
        workload.benchmark.clone(),
        lowered.exe,
        "C: PVM -> NativePvmLowerer -> OpenVM VmExe",
    )?;
    let trace = backend.execute_metered(&artifact, workload.input)?;
    let reference = execute_reference(
        &workload.program,
        &ExecutionInputV1::new(vec![workload.input.a, workload.input.b]),
        workload.output_register,
    )?;
    let expected = expected_public_values(&workload.program, workload.input, reference);
    let public_values_match = trace.public_output == expected;
    if trace.public_output.len() != 128 || !public_values_match {
        return Err(CostModelError::Correctness(format!(
            "{} returned invalid 128-byte public values",
            workload.name
        )));
    }
    let heights = trace
        .segments
        .iter()
        .flat_map(|segment| segment.trace_heights.iter().copied())
        .fold(Vec::<u64>::new(), |mut result, height| {
            result.push(u64::from(height));
            result
        });
    let proof_work_v1 = (!heights.is_empty()).then(|| heights.iter().sum());
    Ok(TraceWorkloadReport {
        name: workload.name.to_string(),
        predicted_static_instruction_count,
        actual_lowered_instruction_count: predicted_static_instruction_count,
        executed_instruction_count: trace.executed_instruction_count,
        trace_heights: (!heights.is_empty()).then_some(heights),
        proof_work_v1,
        measurement_status: if proof_work_v1.is_some() {
            "complete".to_string()
        } else {
            "partial".to_string()
        },
        public_values_len: trace.public_output.len(),
        public_values_match,
        program_commitment: hex(&program_commitment(&workload.program)),
        input_commitment: hex(&input_commitment(
            &zk_jam_translation::ExecutionInputV1::new(vec![workload.input.a, workload.input.b]),
        )),
        reference_output: reference,
    })
}

pub fn measure_many(
    workloads: &[crate::workload::CostWorkload],
    backend: &OpenVmBackend,
) -> Result<Vec<TraceWorkloadReport>, CostModelError> {
    let mut artifacts = Vec::with_capacity(workloads.len());
    let mut predicted = Vec::with_capacity(workloads.len());
    for workload in workloads {
        let lowered = zk_jam_openvm_backend::native_pvm::NativePvmLowerer::default()
            .lower(&workload.program, workload.output_register)?;
        predicted.push(lowered.openvm_instruction_count);
        artifacts.push(backend.program_from_vm_exe(
            workload.benchmark.clone(),
            lowered.exe,
            "C: PVM -> NativePvmLowerer -> OpenVM VmExe",
        )?);
    }
    let inputs = artifacts
        .iter()
        .zip(workloads)
        .map(|(artifact, workload)| (artifact, workload.input))
        .collect::<Vec<_>>();
    let traces = backend.execute_metered_batch(&inputs)?;
    workloads
        .iter()
        .zip(traces)
        .zip(predicted)
        .map(|((workload, trace), predicted)| {
            let reference = execute_reference(
                &workload.program,
                &ExecutionInputV1::new(vec![workload.input.a, workload.input.b]),
                workload.output_register,
            )?;
            let expected = expected_public_values(&workload.program, workload.input, reference);
            if trace.public_output.len() != 128 || trace.public_output != expected {
                return Err(CostModelError::Correctness(format!(
                    "{} returned invalid 128-byte public values",
                    workload.name
                )));
            }
            let heights = trace
                .segments
                .iter()
                .flat_map(|segment| segment.trace_heights.iter().copied())
                .map(u64::from)
                .collect::<Vec<_>>();
            let proof_work_v1 = (!heights.is_empty()).then(|| heights.iter().sum());
            Ok(TraceWorkloadReport {
                name: workload.name.to_string(),
                predicted_static_instruction_count: predicted,
                actual_lowered_instruction_count: predicted,
                executed_instruction_count: trace.executed_instruction_count,
                trace_heights: (!heights.is_empty()).then_some(heights),
                proof_work_v1,
                measurement_status: if proof_work_v1.is_some() {
                    "complete".to_string()
                } else {
                    "partial".to_string()
                },
                public_values_len: trace.public_output.len(),
                public_values_match: true,
                program_commitment: hex(&program_commitment(&workload.program)),
                input_commitment: hex(&input_commitment(&ExecutionInputV1::new(vec![
                    workload.input.a,
                    workload.input.b,
                ]))),
                reference_output: reference,
            })
        })
        .collect()
}

fn expected_public_values(
    program: &zk_jam_refine_interface::PvmProgramV1,
    input: zk_jam_openvm_backend::M2Input,
    output: u64,
) -> Vec<u8> {
    M4PublicValuesV1 {
        program_commitment: program_commitment(program),
        input_commitment: input_commitment(&ExecutionInputV1::new(vec![input.a, input.b])),
        output: {
            let mut value = [0u8; 32];
            value[..4].copy_from_slice(&(output as u32).to_le_bytes());
            value
        },
    }
    .encode_openvm()
    .to_vec()
}

fn hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}
