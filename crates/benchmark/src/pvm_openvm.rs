//! Unified PVM -> OpenVM semantic gate and three-way proof benchmark.
//!
//! The public surface of this module intentionally uses the benchmark's stable names rather than
//! the historical M4/M4.1 milestone names. The bounded fixture set and all M4 public-statement
//! semantics are reused unchanged. Proving is isolated in one child process per implementation.

use std::{env, fs, path::Path, process::Command, time::Instant};

use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use zk_jam_openvm_backend::{
    native_pvm::NativePvmLowerer, M2Benchmark, M2Input, M4ExpectedStatement, M4ProofArtifact,
    M4PublicValuesV1, OpenVmBackend, OpenVmProgramArtifact,
};
use zk_jam_translation::{
    execute_reference, input_commitment, program_commitment, translate, workload_program,
    ExecutionInputV1,
};

use crate::{
    environment_report,
    m4::{build_pvm_openvm_generated_program, m4_case_specs, M4ProgramId},
    read_json, write_json, EnvironmentReport,
};

pub const PVM_OPENVM_SCHEMA_VERSION: &str = "pvm-openvm-benchmark-v1";
const PVM_OPENVM_IMPLEMENTATIONS: [&str; 3] = [
    "direct_openvm_guest",
    "generated_guest",
    "direct_pvm_lowering",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PvmOpenVmSemanticCase {
    pub workload: String,
    pub case: String,
    pub input: [u32; 2],
    pub reference_output_hex: String,
    pub direct_openvm_guest_output_hex: String,
    pub generated_guest_output_hex: String,
    pub direct_pvm_lowering_output_hex: String,
    pub direct_openvm_guest_match: bool,
    pub generated_guest_match: bool,
    pub direct_pvm_lowering_match: bool,
    pub public_values_len: usize,
    pub reserved_bytes_zero: bool,
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PvmOpenVmSemanticGate {
    pub complete: bool,
    pub cases: Vec<PvmOpenVmSemanticCase>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PvmOpenVmSide {
    pub build_ns: Option<u128>,
    pub transpile_ns: Option<u128>,
    pub lowering_ns: Option<u128>,
    pub app_keygen_ns: Option<u128>,
    pub agg_keygen_ns: Option<u128>,
    pub keygen_ns: Option<u128>,
    pub execute_ns: Option<u128>,
    pub prove_ns: Option<u128>,
    pub verify_ns: Option<u128>,
    pub proof_bytes: Option<usize>,
    pub peak_rss_bytes: Option<u64>,
    pub executable_bytes: Option<usize>,
    pub serialized_executable_bytes: Option<usize>,
    pub public_values_len: Option<usize>,
    pub output_hex: Option<String>,
    pub proof_verified: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PvmOpenVmRatioSet {
    pub execute: Option<f64>,
    pub prove: Option<f64>,
    pub keygen: Option<f64>,
    pub verify: Option<f64>,
    pub proof_size: Option<f64>,
    pub peak_rss: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PvmOpenVmRatios {
    pub generated_over_direct: PvmOpenVmRatioSet,
    pub direct_pvm_lowering_over_direct: PvmOpenVmRatioSet,
    pub direct_pvm_lowering_over_generated: PvmOpenVmRatioSet,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PvmOpenVmWorkload {
    pub workload: String,
    pub input: [u32; 2],
    pub reference_output_hex: String,
    pub source_pvm_instruction_count: usize,
    pub generated_ir_instruction_count: usize,
    pub translation_ns: Option<u128>,
    pub emission_ns: Option<u128>,
    pub lowered_openvm_instruction_count: Option<usize>,
    pub direct_openvm_guest: PvmOpenVmSide,
    pub generated_guest: PvmOpenVmSide,
    pub direct_pvm_lowering: PvmOpenVmSide,
    pub ratios: PvmOpenVmRatios,
    pub semantics_match: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PvmOpenVmBenchmarkReport {
    pub schema_version: String,
    pub environment: EnvironmentReport,
    pub semantic_gate: PvmOpenVmSemanticGate,
    pub workloads: Vec<PvmOpenVmWorkload>,
    pub comparison_status: String,
    pub comparison_complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PvmOpenVmWorkerOutput {
    pub implementation: String,
    pub side: PvmOpenVmSide,
    pub source_pvm_instruction_count: usize,
    pub generated_ir_instruction_count: usize,
    pub translation_ns: Option<u128>,
    pub emission_ns: Option<u128>,
    pub lowered_openvm_instruction_count: Option<usize>,
}

fn representative_input(workload: M4ProgramId) -> [u32; 2] {
    match workload {
        M4ProgramId::Arithmetic => [7, 9],
        M4ProgramId::Branch => [21, 8],
        M4ProgramId::Memory16K => [0x1234_5678, 16 * 1024],
    }
}

fn public_name(workload: M4ProgramId) -> &'static str {
    match workload {
        M4ProgramId::Arithmetic => "arithmetic",
        M4ProgramId::Branch => "branch",
        M4ProgramId::Memory16K => "memory-16k",
    }
}

fn native_benchmark(workload: M4ProgramId) -> M2Benchmark {
    workload.native_benchmark()
}

fn parse_workload(name: &str) -> Result<M4ProgramId> {
    match name {
        "arithmetic" => Ok(M4ProgramId::Arithmetic),
        "branch" => Ok(M4ProgramId::Branch),
        "memory" | "memory-16k" | "memory-16384" => Ok(M4ProgramId::Memory16K),
        other => Err(eyre!("unknown PVM -> OpenVM workload: {other}")),
    }
}

fn execute_values(
    backend: &OpenVmBackend,
    artifact: &OpenVmProgramArtifact,
    input: [u32; 2],
) -> Result<M4PublicValuesV1> {
    let execution = backend.execute(artifact, M2Input::arithmetic(input[0], input[1]))?;
    M4PublicValuesV1::decode_openvm(&execution.public_output)
        .map_err(|error| eyre!("decode PVM -> OpenVM public values: {error}"))
}

fn expected_statement(
    workload: M4ProgramId,
    input: [u32; 2],
) -> Result<(
    zk_jam_refine_interface::PvmProgramV1,
    ExecutionInputV1,
    M4ExpectedStatement,
)> {
    let source = workload_program(workload.workload());
    let input_value = ExecutionInputV1::new(input.to_vec());
    let reference = execute_reference(&source, &input_value, workload.output_register())? as u32;
    let mut public_output = [0u8; 32];
    public_output[..4].copy_from_slice(&reference.to_le_bytes());
    Ok((
        source.clone(),
        input_value.clone(),
        M4ExpectedStatement {
            program_commitment: program_commitment(&source),
            input_commitment: input_commitment(&input_value),
            public_output,
        },
    ))
}

fn public_values_match(values: &M4PublicValuesV1, expected: &M4ExpectedStatement) -> bool {
    values.program_commitment == expected.program_commitment
        && values.input_commitment == expected.input_commitment
        && values.output == expected.public_output
        && values.output[4..].iter().all(|byte| *byte == 0)
}

pub fn run_pvm_openvm_preflight(output_root: &Path) -> Result<PvmOpenVmSemanticGate> {
    let backend = OpenVmBackend;
    let mut artifacts = Vec::new();
    for workload in M4ProgramId::ALL {
        let generated = build_pvm_openvm_generated_program(&backend, workload)?;
        let direct = backend.m4_native_program(native_benchmark(workload))?;
        let lowered =
            NativePvmLowerer::default().lower(&generated.program, workload.output_register())?;
        let direct_pvm = backend.program_from_vm_exe(
            native_benchmark(workload),
            lowered.exe,
            "Direct PVM Lowering: PvmProgramV1 -> OpenVM Instructions",
        )?;
        artifacts.push((workload, generated.artifact, direct, direct_pvm));
    }

    let mut cases = Vec::new();
    for case in m4_case_specs() {
        let (_, generated, direct, direct_pvm) = artifacts
            .iter()
            .find(|(workload, _, _, _)| *workload == case.program)
            .ok_or_else(|| eyre!("missing PVM -> OpenVM semantic-gate artifact"))?;
        let (_, _, expected) = expected_statement(case.program, case.input)?;
        let direct_values = execute_values(&backend, direct, case.input)?;
        let generated_values = execute_values(&backend, generated, case.input)?;
        let lowering_values = execute_values(&backend, direct_pvm, case.input)?;
        let reserved_bytes_zero = [&direct_values, &generated_values, &lowering_values]
            .iter()
            .all(|values| values.output[4..].iter().all(|byte| *byte == 0));
        let direct_match = public_values_match(&direct_values, &expected);
        let generated_match = public_values_match(&generated_values, &expected);
        let lowering_match = public_values_match(&lowering_values, &expected);
        let mut expected_hex = [0u8; 32];
        expected_hex.copy_from_slice(&expected.public_output);
        cases.push(PvmOpenVmSemanticCase {
            workload: public_name(case.program).to_string(),
            case: case.name.to_string(),
            input: case.input,
            reference_output_hex: hex(&expected_hex),
            direct_openvm_guest_output_hex: hex(&direct_values.output),
            generated_guest_output_hex: hex(&generated_values.output),
            direct_pvm_lowering_output_hex: hex(&lowering_values.output),
            direct_openvm_guest_match: direct_match,
            generated_guest_match: generated_match,
            direct_pvm_lowering_match: lowering_match,
            public_values_len: M4PublicValuesV1::OPENVM_LEN,
            reserved_bytes_zero,
            complete: direct_match && generated_match && lowering_match && reserved_bytes_zero,
        });
    }
    let gate = PvmOpenVmSemanticGate {
        complete: cases.len() == 6 && cases.iter().all(|case| case.complete),
        cases,
    };
    let result_dir = output_root.join(format!("pvm-openvm-preflight-{}", timestamp()));
    fs::create_dir_all(&result_dir)?;
    write_json(result_dir.join("pvm-openvm-preflight.json"), &gate)?;
    fs::write(
        result_dir.join("pvm-openvm-preflight.md"),
        render_gate(&gate),
    )?;
    if !gate.complete {
        return Err(eyre!("PVM -> OpenVM semantic gate failed"));
    }
    Ok(gate)
}

fn process_peak_rss_bytes() -> Option<u64> {
    fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmHWM:")?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
        .map(|kib| kib * 1024)
}

struct WorkerArtifact {
    artifact: OpenVmProgramArtifact,
    translation_ns: Option<u128>,
    emission_ns: Option<u128>,
    lowering_ns: Option<u128>,
    lowered_openvm_instruction_count: Option<usize>,
    source_pvm_instruction_count: usize,
    generated_ir_instruction_count: usize,
}

fn worker_artifact(implementation: &str, workload: M4ProgramId) -> Result<WorkerArtifact> {
    let backend = OpenVmBackend;
    let source = workload_program(workload.workload());
    match implementation {
        "direct_openvm_guest" => Ok(WorkerArtifact {
            artifact: backend.m4_native_program(native_benchmark(workload))?,
            translation_ns: None,
            emission_ns: None,
            lowering_ns: None,
            lowered_openvm_instruction_count: None,
            source_pvm_instruction_count: source.instruction_count(),
            generated_ir_instruction_count: 0,
        }),
        "generated_guest" => {
            let built = build_pvm_openvm_generated_program(&backend, workload)?;
            let count = built.translated.translated_instruction_count();
            Ok(WorkerArtifact {
                artifact: built.artifact,
                translation_ns: Some(built.translation_ns),
                emission_ns: Some(built.emission_ns),
                lowering_ns: None,
                lowered_openvm_instruction_count: None,
                source_pvm_instruction_count: source.instruction_count(),
                generated_ir_instruction_count: count,
            })
        }
        "direct_pvm_lowering" => {
            let started = Instant::now();
            let lowered = NativePvmLowerer::default().lower(&source, workload.output_register())?;
            let lowering_ns = started.elapsed().as_nanos();
            let count = lowered.openvm_instruction_count;
            let artifact = backend.program_from_vm_exe(
                native_benchmark(workload),
                lowered.exe,
                "Direct PVM Lowering: PvmProgramV1 -> OpenVM Instructions",
            )?;
            Ok(WorkerArtifact {
                artifact,
                translation_ns: None,
                emission_ns: None,
                lowering_ns: Some(lowering_ns),
                lowered_openvm_instruction_count: Some(count),
                source_pvm_instruction_count: source.instruction_count(),
                generated_ir_instruction_count: 0,
            })
        }
        other => Err(eyre!("unknown PVM -> OpenVM implementation: {other}")),
    }
}

pub fn run_pvm_openvm_worker(
    implementation: &str,
    workload_name: &str,
    input: [u32; 2],
    output: &Path,
) -> Result<()> {
    let workload = parse_workload(workload_name)?;
    let backend = OpenVmBackend;
    let worker_artifact = worker_artifact(implementation, workload)?;
    let WorkerArtifact {
        artifact,
        translation_ns,
        emission_ns,
        lowering_ns,
        lowered_openvm_instruction_count: lowered_count,
        source_pvm_instruction_count: source_count,
        generated_ir_instruction_count: ir_count,
    } = worker_artifact;
    let (_, _, expected) = expected_statement(workload, input)?;
    let prepared = backend.prepare(artifact)?;
    let input_value = M2Input::arithmetic(input[0], input[1]);
    let execute_started = Instant::now();
    let execution = backend.execute_prepared(&prepared, input_value)?;
    let execute_ns = execute_started.elapsed().as_nanos();
    let execution_values = M4PublicValuesV1::decode_openvm(&execution.public_output)?;
    let prove_started = Instant::now();
    let proof = backend.prove_prepared(&prepared, input_value)?;
    let prove_ns = prove_started.elapsed().as_nanos();
    let proof_values = M4PublicValuesV1::decode_openvm(&proof.public_output)?;
    let proof_artifact = M4ProofArtifact {
        schema_version: 1,
        program_commitment: expected.program_commitment,
        input_commitment: expected.input_commitment,
        public_output: expected.public_output,
        proof,
    };
    let verify_started = Instant::now();
    let verified = proof_artifact.verify_m4(&expected, input_value).is_ok();
    let verify_ns = verify_started.elapsed().as_nanos();
    let side = PvmOpenVmSide {
        build_ns: Some(prepared.program.build_time_ns),
        transpile_ns: Some(prepared.program.transpile_time_ns),
        lowering_ns,
        app_keygen_ns: Some(prepared.app_keygen_time_ns),
        agg_keygen_ns: Some(prepared.agg_keygen_time_ns),
        keygen_ns: Some(prepared.keygen_time_ns),
        execute_ns: Some(execute_ns),
        prove_ns: Some(prove_ns),
        verify_ns: Some(verify_ns),
        proof_bytes: Some(proof_artifact.proof.proof_payload_size_bytes()),
        peak_rss_bytes: process_peak_rss_bytes(),
        executable_bytes: Some(prepared.program.executable_bytes),
        serialized_executable_bytes: Some(prepared.program.serialized_executable_size_bytes),
        public_values_len: Some(execution.public_output.len()),
        output_hex: Some(hex(&proof_values.output)),
        proof_verified: verified
            && execution_values == proof_values
            && public_values_match(&proof_values, &expected),
        error: None,
    };
    write_json(
        output.to_path_buf(),
        &PvmOpenVmWorkerOutput {
            implementation: implementation.to_string(),
            side,
            source_pvm_instruction_count: source_count,
            generated_ir_instruction_count: ir_count,
            translation_ns,
            emission_ns,
            lowered_openvm_instruction_count: lowered_count,
        },
    )
}

fn failed_side(error: impl Into<String>) -> PvmOpenVmSide {
    PvmOpenVmSide {
        error: Some(error.into()),
        ..PvmOpenVmSide::default()
    }
}

fn collect_worker(
    output_root: &Path,
    workload: M4ProgramId,
    input: [u32; 2],
    implementation: &str,
) -> PvmOpenVmWorkerOutput {
    let worker_name = implementation.replace('_', "-");
    let output = output_root.join(format!(".{worker_name}-{}.json", workload.name()));
    let executable = match env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            return PvmOpenVmWorkerOutput {
                implementation: implementation.to_string(),
                side: failed_side(format!("locate worker executable: {error}")),
                source_pvm_instruction_count: 0,
                generated_ir_instruction_count: 0,
                translation_ns: None,
                emission_ns: None,
                lowered_openvm_instruction_count: None,
            }
        }
    };
    let a = input[0].to_string();
    let b = input[1].to_string();
    let output_text = output.to_string_lossy().to_string();
    let status = Command::new(executable)
        .args([
            "__pvm-openvm-worker",
            "--implementation",
            implementation,
            "--workload",
            public_name(workload),
            "--a",
            &a,
            "--b",
            &b,
            "--output",
            &output_text,
        ])
        .env("RUST_BACKTRACE", "1")
        .status();
    let result = match status {
        Ok(status) if status.success() => {
            read_json(&output).unwrap_or_else(|error| PvmOpenVmWorkerOutput {
                implementation: implementation.to_string(),
                side: failed_side(format!("invalid worker output: {error}")),
                source_pvm_instruction_count: 0,
                generated_ir_instruction_count: 0,
                translation_ns: None,
                emission_ns: None,
                lowered_openvm_instruction_count: None,
            })
        }
        Ok(status) => PvmOpenVmWorkerOutput {
            implementation: implementation.to_string(),
            side: failed_side(format!("worker exited with status {status}")),
            source_pvm_instruction_count: 0,
            generated_ir_instruction_count: 0,
            translation_ns: None,
            emission_ns: None,
            lowered_openvm_instruction_count: None,
        },
        Err(error) => PvmOpenVmWorkerOutput {
            implementation: implementation.to_string(),
            side: failed_side(format!("spawn worker: {error}")),
            source_pvm_instruction_count: 0,
            generated_ir_instruction_count: 0,
            translation_ns: None,
            emission_ns: None,
            lowered_openvm_instruction_count: None,
        },
    };
    let _ = fs::remove_file(output);
    result
}

fn ratio(numerator: Option<u128>, denominator: Option<u128>) -> Option<f64> {
    Some(numerator? as f64 / denominator? as f64)
}

fn ratio_usize(numerator: Option<usize>, denominator: Option<usize>) -> Option<f64> {
    Some(numerator? as f64 / denominator? as f64)
}

fn ratio_u64(numerator: Option<u64>, denominator: Option<u64>) -> Option<f64> {
    Some(numerator? as f64 / denominator? as f64)
}

fn ratio_set(numerator: &PvmOpenVmSide, denominator: &PvmOpenVmSide) -> PvmOpenVmRatioSet {
    PvmOpenVmRatioSet {
        execute: ratio(numerator.execute_ns, denominator.execute_ns),
        prove: ratio(numerator.prove_ns, denominator.prove_ns),
        keygen: ratio(numerator.keygen_ns, denominator.keygen_ns),
        verify: ratio(numerator.verify_ns, denominator.verify_ns),
        proof_size: ratio_usize(numerator.proof_bytes, denominator.proof_bytes),
        peak_rss: ratio_u64(numerator.peak_rss_bytes, denominator.peak_rss_bytes),
    }
}

fn unavailable_workload(
    workload: M4ProgramId,
    input: [u32; 2],
    error: String,
) -> PvmOpenVmWorkload {
    let source = workload_program(workload.workload());
    let (_, _, expected) = expected_statement(workload, input)
        .unwrap_or_else(|_| panic!("bounded PVM fixture must construct for unavailable report"));
    PvmOpenVmWorkload {
        workload: public_name(workload).to_string(),
        input,
        reference_output_hex: hex(&expected.public_output),
        source_pvm_instruction_count: source.instruction_count(),
        generated_ir_instruction_count: 0,
        translation_ns: None,
        emission_ns: None,
        lowered_openvm_instruction_count: None,
        direct_openvm_guest: failed_side(error.clone()),
        generated_guest: failed_side(error.clone()),
        direct_pvm_lowering: failed_side(error),
        ratios: PvmOpenVmRatios::default(),
        semantics_match: false,
    }
}

pub fn run_pvm_openvm_workload(
    output_root: &Path,
    semantic_gate_path: &Path,
    workload: M4ProgramId,
) -> Result<PvmOpenVmWorkload> {
    run_pvm_openvm_workload_filtered(output_root, semantic_gate_path, workload, None)
}

pub fn run_pvm_openvm_workload_filtered(
    output_root: &Path,
    semantic_gate_path: &Path,
    workload: M4ProgramId,
    only: Option<&str>,
) -> Result<PvmOpenVmWorkload> {
    let gate: PvmOpenVmSemanticGate = read_json(semantic_gate_path)?;
    if !gate.complete {
        return Err(eyre!(
            "PVM -> OpenVM benchmark requires a complete semantic gate"
        ));
    }
    if let Some(only) = only {
        if !PVM_OPENVM_IMPLEMENTATIONS.contains(&only) {
            return Err(eyre!("unknown PVM -> OpenVM implementation filter: {only}"));
        }
    }
    fs::create_dir_all(output_root)?;
    let input = representative_input(workload);
    let (source, _, expected) = expected_statement(workload, input)?;
    let generated_ir_instruction_count = translate(&source)?.translated_instruction_count();
    let mut workers = Vec::new();
    for implementation in PVM_OPENVM_IMPLEMENTATIONS {
        workers.push(if only.is_none() || only == Some(implementation) {
            collect_worker(output_root, workload, input, implementation)
        } else {
            PvmOpenVmWorkerOutput {
                implementation: implementation.to_string(),
                side: failed_side(format!("implementation not selected by --only={only:?}")),
                source_pvm_instruction_count: source.instruction_count(),
                generated_ir_instruction_count: 0,
                translation_ns: None,
                emission_ns: None,
                lowered_openvm_instruction_count: None,
            }
        });
    }
    let direct = &workers[0];
    let generated = &workers[1];
    let lowering = &workers[2];
    let semantics_match =
        workers
            .iter()
            .zip(PVM_OPENVM_IMPLEMENTATIONS)
            .all(|(worker, implementation)| {
                if only.is_some() && only != Some(implementation) {
                    return true;
                }
                worker.side.error.is_none()
                    && worker.side.proof_verified
                    && worker.side.output_hex.as_deref()
                        == Some(hex(&expected.public_output).as_str())
            });
    let report = PvmOpenVmWorkload {
        workload: public_name(workload).to_string(),
        input,
        reference_output_hex: hex(&expected.public_output),
        source_pvm_instruction_count: source.instruction_count(),
        generated_ir_instruction_count,
        translation_ns: generated.translation_ns,
        emission_ns: generated.emission_ns,
        lowered_openvm_instruction_count: lowering.lowered_openvm_instruction_count,
        direct_openvm_guest: direct.side.clone(),
        generated_guest: generated.side.clone(),
        direct_pvm_lowering: lowering.side.clone(),
        ratios: PvmOpenVmRatios {
            generated_over_direct: ratio_set(&generated.side, &direct.side),
            direct_pvm_lowering_over_direct: ratio_set(&lowering.side, &direct.side),
            direct_pvm_lowering_over_generated: ratio_set(&lowering.side, &generated.side),
        },
        semantics_match,
    };
    write_json(
        output_root.join(format!("pvm-openvm-{}.json", workload.name())),
        &report,
    )?;
    Ok(report)
}

pub fn aggregate_pvm_openvm(
    output_root: &Path,
    semantic_gate_path: &Path,
    partial_paths: [&Path; 3],
) -> Result<PvmOpenVmBenchmarkReport> {
    let semantic_gate: PvmOpenVmSemanticGate = read_json(semantic_gate_path)?;
    let workloads = M4ProgramId::ALL
        .into_iter()
        .zip(partial_paths)
        .map(|(workload, path)| {
            if path.exists() {
                read_json(path).unwrap_or_else(|error| {
                    unavailable_workload(
                        workload,
                        representative_input(workload),
                        format!("invalid or unreadable workload report: {error}"),
                    )
                })
            } else {
                unavailable_workload(
                    workload,
                    representative_input(workload),
                    "workload worker report unavailable".to_string(),
                )
            }
        })
        .collect::<Vec<_>>();
    let comparison_complete = semantic_gate.complete
        && workloads.len() == 3
        && workloads.iter().all(|workload| {
            workload.semantics_match
                && workload.direct_openvm_guest.proof_verified
                && workload.generated_guest.proof_verified
                && workload.direct_pvm_lowering.proof_verified
                && workload.direct_openvm_guest.error.is_none()
                && workload.generated_guest.error.is_none()
                && workload.direct_pvm_lowering.error.is_none()
        });
    let available = workloads
        .iter()
        .flat_map(|workload| {
            [
                &workload.direct_openvm_guest,
                &workload.generated_guest,
                &workload.direct_pvm_lowering,
            ]
        })
        .filter(|side| side.error.is_none())
        .count();
    let report = PvmOpenVmBenchmarkReport {
        schema_version: PVM_OPENVM_SCHEMA_VERSION.to_string(),
        environment: environment_report(&format!("pvm-openvm-{}", timestamp()), "cpu")?,
        semantic_gate,
        workloads,
        comparison_status: if comparison_complete {
            "complete"
        } else if available == 0 {
            "unavailable"
        } else {
            "partial"
        }
        .to_string(),
        comparison_complete,
    };
    let result_dir = output_root.join(format!("pvm-openvm-benchmark-{}", timestamp()));
    fs::create_dir_all(&result_dir)?;
    write_json(result_dir.join("pvm-openvm-benchmark.json"), &report)?;
    fs::write(
        result_dir.join("pvm-openvm-benchmark.csv"),
        render_csv(&report),
    )?;
    fs::write(
        result_dir.join("pvm-openvm-comparison.csv"),
        render_comparison_csv(&report),
    )?;
    fs::write(
        result_dir.join("pvm-openvm-benchmark.md"),
        render_markdown(&report),
    )?;
    Ok(report)
}

pub fn validate_pvm_openvm_report(report_path: &Path, schema_path: &Path) -> Result<()> {
    let report_value: serde_json::Value = serde_json::from_slice(&fs::read(report_path)?)?;
    let schema: serde_json::Value = serde_json::from_slice(&fs::read(schema_path)?)?;
    let compiled = jsonschema::validator_for(&schema)
        .map_err(|error| eyre!("compile PVM -> OpenVM schema: {error}"))?;
    let errors = compiled
        .iter_errors(&report_value)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(eyre!(
            "PVM -> OpenVM report failed schema validation:\n{}",
            errors.join("\n")
        ));
    }
    let report: PvmOpenVmBenchmarkReport = serde_json::from_value(report_value)?;
    let complete = report.semantic_gate.complete
        && report.workloads.len() == 3
        && report
            .workloads
            .iter()
            .all(|workload| workload.semantics_match);
    if report.comparison_complete != complete
        || (complete && report.comparison_status != "complete")
    {
        return Err(eyre!(
            "PVM -> OpenVM completion status does not match report semantics"
        ));
    }
    Ok(())
}

fn render_gate(gate: &PvmOpenVmSemanticGate) -> String {
    let mut output = format!(
        "# PVM → OpenVM Semantic Compatibility\n\n{} / 6 cases passed\n\n",
        gate.cases.iter().filter(|case| case.complete).count()
    );
    output.push_str("| Workload | Case | Direct OpenVM Guest | Generated Guest | Direct PVM Lowering |\n|---|---|---:|---:|---:|\n");
    for case in &gate.cases {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            case.workload,
            case.case,
            case.direct_openvm_guest_match,
            case.generated_guest_match,
            case.direct_pvm_lowering_match
        ));
    }
    output
}

fn render_csv(report: &PvmOpenVmBenchmarkReport) -> String {
    let mut output = String::from("workload,implementation,metric,value\n");
    for workload in &report.workloads {
        for (name, side) in [
            ("direct_openvm_guest", &workload.direct_openvm_guest),
            ("generated_guest", &workload.generated_guest),
            ("direct_pvm_lowering", &workload.direct_pvm_lowering),
        ] {
            for (metric, value) in side_metrics(side) {
                output.push_str(&format!(
                    "{},{},{},{}\n",
                    workload.workload,
                    name,
                    metric,
                    value.unwrap_or_default()
                ));
            }
        }
    }
    output
}

fn side_metrics(side: &PvmOpenVmSide) -> [(&'static str, Option<String>); 14] {
    [
        ("build_ns", side.build_ns.map(|value| value.to_string())),
        (
            "transpile_ns",
            side.transpile_ns.map(|value| value.to_string()),
        ),
        (
            "lowering_ns",
            side.lowering_ns.map(|value| value.to_string()),
        ),
        (
            "app_keygen_ns",
            side.app_keygen_ns.map(|value| value.to_string()),
        ),
        (
            "agg_keygen_ns",
            side.agg_keygen_ns.map(|value| value.to_string()),
        ),
        ("keygen_ns", side.keygen_ns.map(|value| value.to_string())),
        ("execute_ns", side.execute_ns.map(|value| value.to_string())),
        ("prove_ns", side.prove_ns.map(|value| value.to_string())),
        ("verify_ns", side.verify_ns.map(|value| value.to_string())),
        (
            "proof_bytes",
            side.proof_bytes.map(|value| value.to_string()),
        ),
        (
            "peak_rss_bytes",
            side.peak_rss_bytes.map(|value| value.to_string()),
        ),
        (
            "executable_bytes",
            side.executable_bytes.map(|value| value.to_string()),
        ),
        (
            "serialized_executable_bytes",
            side.serialized_executable_bytes
                .map(|value| value.to_string()),
        ),
        (
            "public_values_len",
            side.public_values_len.map(|value| value.to_string()),
        ),
    ]
}

fn render_comparison_csv(report: &PvmOpenVmBenchmarkReport) -> String {
    let mut output =
        String::from("workload,ratio_group,execute,prove,keygen,verify,proof_size,peak_rss\n");
    for workload in &report.workloads {
        for (name, ratios) in [
            (
                "generated_over_direct",
                &workload.ratios.generated_over_direct,
            ),
            (
                "direct_pvm_lowering_over_direct",
                &workload.ratios.direct_pvm_lowering_over_direct,
            ),
            (
                "direct_pvm_lowering_over_generated",
                &workload.ratios.direct_pvm_lowering_over_generated,
            ),
        ] {
            output.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                workload.workload,
                name,
                opt(ratios.execute),
                opt(ratios.prove),
                opt(ratios.keygen),
                opt(ratios.verify),
                opt(ratios.proof_size),
                opt(ratios.peak_rss)
            ));
        }
    }
    output
}

fn opt(value: Option<f64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn render_markdown(report: &PvmOpenVmBenchmarkReport) -> String {
    let passed = report
        .semantic_gate
        .cases
        .iter()
        .filter(|case| case.complete)
        .count();
    let mut output = format!("# PVM → OpenVM Benchmark\n\n## Environment\n\n- OpenVM: `{}` at `{}`\n- Runner: `{}/{}`\n\n## Semantic Compatibility\n\n{} / 6 cases passed\n\n## Implementations\n\n### Direct OpenVM Guest\n\n### Generated Guest\n\n### Direct PVM Lowering\n\n## Performance\n\n| Workload | Implementation | Execute | Prove | Keygen | Verify | Proof | RSS |\n|---|---|---:|---:|---:|---:|---:|---:|\n", report.environment.openvm_version, report.environment.openvm_revision, report.environment.os, report.environment.arch, passed);
    for workload in &report.workloads {
        for (name, side) in [
            ("Direct OpenVM Guest", &workload.direct_openvm_guest),
            ("Generated Guest", &workload.generated_guest),
            ("Direct PVM Lowering", &workload.direct_pvm_lowering),
        ] {
            output.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
                workload.workload,
                name,
                opt_u128(side.execute_ns),
                opt_u128(side.prove_ns),
                opt_u128(side.keygen_ns),
                opt_u128(side.verify_ns),
                opt_usize(side.proof_bytes),
                opt_u64(side.peak_rss_bytes)
            ));
        }
    }
    output.push_str("\n## Relative Performance\n\nRatios use Direct OpenVM Guest as the 1.0 baseline.\n\n## Proof Size\n\n## Memory Usage\n\n## Translation and Lowering Cost\n\n## Conclusions\n\nThis benchmark measures the cost of executing and proving the bounded PVM fixture programs through three OpenVM implementation strategies.\n");
    output
}

fn opt_u128(value: Option<u128>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}
fn opt_usize(value: Option<usize>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}
fn opt_u64(value: Option<u64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn timestamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
