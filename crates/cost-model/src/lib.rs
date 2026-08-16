//! A+B PVM → OpenVM cost model.  This crate intentionally contains no proving in its static or
//! trace paths; the optional GPU calibration entry point is the only path allowed to call proof.

use std::{collections::BTreeMap, fs, path::Path, process::Command, time::Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use zk_jam_openvm_backend::{native_pvm::NativePvmLowerer, OpenVmBackend};
use zk_jam_translation::{
    execute_reference, input_commitment, program_commitment, ExecutionInputV1,
};

pub mod aggregate;
pub mod lowering_cost;
pub mod pvm_gas;
pub mod schema;
pub mod trace_model;
pub mod workload;

pub const COST_MODEL_VERSION: &str = "pvm-openvm-cost-model-v1";

#[derive(Debug, Error)]
pub enum CostModelError {
    #[error("cost model I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("cost model JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("cost model backend error: {0}")]
    Backend(#[from] eyre::Report),
    #[error("cost model lowering error: {0}")]
    Lowering(#[from] zk_jam_openvm_backend::native_pvm::NativePvmError),
    #[error("cost model translation error: {0}")]
    Translation(#[from] zk_jam_translation::TranslationError),
    #[error("cost model PVM gas error: {0}")]
    Gas(#[from] pvm_gas::PvmGasError),
    #[error("cost model correctness failure: {0}")]
    Correctness(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Environment {
    pub zk_jam_revision: String,
    pub jambda_revision: String,
    pub openvm_revision: String,
    pub rust_toolchain: String,
    pub cost_model_version: String,
    pub translation_version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StaticWorkloadReport {
    pub name: String,
    pub pattern: String,
    pub pvm_instruction_count: usize,
    pub pvm_gas: pvm_gas::PvmGasReport,
    pub openvm_static: lowering_cost::LoweringCostReport,
    pub static_instruction_ratio: f64,
    pub program_commitment: String,
    pub input_commitment: String,
    pub reference_output: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StaticReport {
    pub schema_version: String,
    pub environment: Environment,
    pub workloads: Vec<StaticWorkloadReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraceWorkloadReport {
    pub name: String,
    pub predicted_static_instruction_count: usize,
    pub actual_lowered_instruction_count: usize,
    pub executed_instruction_count: u64,
    pub trace_heights: Option<Vec<u64>>,
    pub proof_work_v1: Option<u64>,
    pub measurement_status: String,
    pub public_values_len: usize,
    pub public_values_match: bool,
    pub program_commitment: String,
    pub input_commitment: String,
    pub reference_output: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraceReport {
    pub schema_version: String,
    pub environment: Environment,
    pub proof_work_definition: String,
    pub workloads: Vec<TraceWorkloadReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CombinedWorkloadReport {
    pub name: String,
    pub pvm_gas: u64,
    pub pvm_core_instructions: usize,
    pub proof_envelope_instructions: usize,
    pub total_static_instructions: usize,
    pub executed_instruction_count: u64,
    pub proof_work_v1: Option<u64>,
    pub static_instruction_ratio: f64,
    pub proof_work_per_pvm_gas: Option<f64>,
    pub core_alpha: Option<f64>,
    pub total_alpha: Option<f64>,
    pub measurement_status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CombinedReport {
    pub schema_version: String,
    pub environment: Environment,
    pub proof_work_definition: String,
    pub workloads: Vec<CombinedWorkloadReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GpuCalibrationSample {
    pub workload: String,
    pub sample: usize,
    pub pvm_gas: u64,
    pub proof_work: u64,
    pub prove_ns: u128,
    pub verify_ns: u128,
    pub proof_bytes: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CalibrationReport {
    pub t0_seconds: f64,
    pub k_seconds_per_work: f64,
    pub throughput_work_per_second: f64,
    pub r_squared: f64,
    pub max_relative_error: f64,
    pub calibration_status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GpuCalibrationReport {
    pub schema_version: String,
    pub environment: Environment,
    pub hardware: BTreeMap<String, String>,
    pub samples: Vec<GpuCalibrationSample>,
    pub fit: Option<CalibrationReport>,
}

pub fn analyze_static() -> Result<StaticReport, CostModelError> {
    let mut reports = Vec::new();
    for workload in workload::all() {
        let lowered =
            NativePvmLowerer::default().lower(&workload.program, workload.output_register)?;
        let gas = pvm_gas::analyze(&workload.program)?;
        let reference = execute_reference(
            &workload.program,
            &ExecutionInputV1::new(vec![workload.input.a, workload.input.b]),
            workload.output_register,
        )?;
        let openvm_static = lowering_cost::analyze(&lowered);
        if !lowering_cost::validate_count(&lowered) {
            return Err(CostModelError::Correctness(format!(
                "{} static count mismatch",
                workload.name
            )));
        }
        if openvm_static.total.total_instructions != lowered.openvm_instruction_count {
            return Err(CostModelError::Correctness(format!(
                "{} predicted static count mismatch",
                workload.name
            )));
        }
        reports.push(StaticWorkloadReport {
            name: workload.name.to_string(),
            pattern: workload.pattern.to_string(),
            pvm_instruction_count: workload.program.instruction_count(),
            static_instruction_ratio: openvm_static.total.total_instructions as f64
                / gas.total_gas.max(1) as f64,
            pvm_gas: gas,
            program_commitment: hex(&program_commitment(&workload.program)),
            input_commitment: hex(&input_commitment(&ExecutionInputV1::new(vec![
                workload.input.a,
                workload.input.b,
            ]))),
            reference_output: reference,
            openvm_static,
        });
    }
    Ok(StaticReport {
        schema_version: schema::SCHEMA_VERSION.to_string(),
        environment: environment(),
        workloads: reports,
    })
}

pub fn validate_trace() -> Result<TraceReport, CostModelError> {
    validate_trace_for(None)
}

pub fn validate_trace_for(workload_name: Option<&str>) -> Result<TraceReport, CostModelError> {
    let backend = OpenVmBackend;
    let selected = workload::all()
        .into_iter()
        .filter(|workload| workload_name.is_none() || Some(workload.name) == workload_name)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(CostModelError::Correctness(
            "no matching trace workload".into(),
        ));
    }
    let workloads = trace_model::measure_many(&selected, &backend)?;
    Ok(TraceReport {
        schema_version: schema::SCHEMA_VERSION.to_string(), environment: environment(),
        proof_work_definition: "proof_work_v1 = sum of OpenVM metered segment trace_heights across AIR indices; it is padded trace rows, not instruction count".to_string(),
        workloads,
    })
}

pub fn run_ci(output: &Path) -> Result<(), CostModelError> {
    fs::create_dir_all(output)?;
    let static_report = analyze_static()?;
    let trace_report = validate_trace()?;
    let combined = combine(&static_report, &trace_report)?;
    write_json(&output.join("cost-model-static.json"), &static_report)?;
    write_json(&output.join("cost-model-trace.json"), &trace_report)?;
    write_json(&output.join("cost-model-combined.json"), &combined)?;
    fs::write(output.join("pvm-openvm-cost-model.md"), markdown(&combined))?;
    Ok(())
}

pub fn combine(
    static_report: &StaticReport,
    trace_report: &TraceReport,
) -> Result<CombinedReport, CostModelError> {
    if static_report.workloads.len() != trace_report.workloads.len() {
        return Err(CostModelError::Correctness(
            "static/trace workload count mismatch".into(),
        ));
    }
    let mut workloads = Vec::new();
    for static_workload in &static_report.workloads {
        let trace = trace_report
            .workloads
            .iter()
            .find(|item| item.name == static_workload.name)
            .ok_or_else(|| {
                CostModelError::Correctness(format!(
                    "missing trace workload {}",
                    static_workload.name
                ))
            })?;
        if trace.predicted_static_instruction_count
            != static_workload.openvm_static.total.total_instructions
            || !trace.public_values_match
        {
            return Err(CostModelError::Correctness(format!(
                "{} static/trace/reference mismatch",
                static_workload.name
            )));
        }
        let proof_work_per_pvm_gas = trace
            .proof_work_v1
            .map(|work| work as f64 / static_workload.pvm_gas.total_gas.max(1) as f64);
        let core_alpha = trace.proof_work_v1.map(|work| {
            work as f64
                / static_workload
                    .openvm_static
                    .pvm_core
                    .total_instructions
                    .max(1) as f64
        });
        let total_alpha = trace.proof_work_v1.map(|work| {
            work as f64
                / static_workload
                    .openvm_static
                    .total
                    .total_instructions
                    .max(1) as f64
        });
        workloads.push(CombinedWorkloadReport {
            name: static_workload.name.clone(),
            pvm_gas: static_workload.pvm_gas.total_gas,
            pvm_core_instructions: static_workload.openvm_static.pvm_core.total_instructions,
            proof_envelope_instructions: static_workload
                .openvm_static
                .proof_envelope
                .total_instructions,
            total_static_instructions: static_workload.openvm_static.total.total_instructions,
            executed_instruction_count: trace.executed_instruction_count,
            proof_work_v1: trace.proof_work_v1,
            static_instruction_ratio: static_workload.static_instruction_ratio,
            proof_work_per_pvm_gas,
            core_alpha,
            total_alpha,
            measurement_status: trace.measurement_status.clone(),
        });
    }
    Ok(CombinedReport {
        schema_version: schema::SCHEMA_VERSION.to_string(),
        environment: static_report.environment.clone(),
        proof_work_definition: trace_report.proof_work_definition.clone(),
        workloads,
    })
}

pub fn gpu_calibrate(
    workload_name: Option<&str>,
    samples: usize,
    warmup: usize,
) -> Result<GpuCalibrationReport, CostModelError> {
    let backend = OpenVmBackend;
    let all = workload::all();
    let selected = all
        .into_iter()
        .filter(|workload| workload_name.is_none() || Some(workload.name) == workload_name)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(CostModelError::Correctness(
            "no matching calibration workload".into(),
        ));
    }
    let mut output = Vec::new();
    for workload in selected {
        let lowered =
            NativePvmLowerer::default().lower(&workload.program, workload.output_register)?;
        let gas = pvm_gas::analyze(&workload.program)?.total_gas;
        let artifact = backend.program_from_vm_exe(
            workload.benchmark.clone(),
            lowered.exe,
            "C: local GPU calibration direct lowering",
        )?;
        let trace = backend.execute_metered(&artifact, workload.input)?;
        let proof_work = trace
            .segments
            .iter()
            .flat_map(|segment| segment.trace_heights.iter())
            .map(|height| u64::from(*height))
            .sum::<u64>();
        let prepared = backend.prepare(artifact)?;
        for _ in 0..warmup {
            let _ = backend.prove_prepared(&prepared, workload.input)?;
        }
        for sample in 0..samples {
            let started = Instant::now();
            let proof = backend.prove_prepared(&prepared, workload.input)?;
            let prove_ns = started.elapsed().as_nanos();
            let started = Instant::now();
            proof.verify(&workload.input.context_hash(&workload.benchmark))?;
            let verify_ns = started.elapsed().as_nanos();
            output.push(GpuCalibrationSample {
                workload: workload.name.to_string(),
                sample,
                pvm_gas: gas,
                proof_work,
                prove_ns,
                verify_ns,
                proof_bytes: proof.proof_payload_size_bytes(),
            });
        }
    }
    Ok(GpuCalibrationReport {
        schema_version: schema::SCHEMA_VERSION.to_string(),
        environment: environment(),
        hardware: hardware(),
        fit: aggregate::fit(&output),
        samples: output,
    })
}

pub fn aggregate_file(
    combined_path: &Path,
    calibration_path: &Path,
    output: &Path,
) -> Result<(), CostModelError> {
    let combined: CombinedReport = read_json(combined_path)?;
    let calibration: GpuCalibrationReport = read_json(calibration_path)?;
    let final_report = aggregate::aggregate(&combined, calibration.fit.as_ref())?;
    write_json(output, &final_report)
}

fn environment() -> Environment {
    Environment {
        zk_jam_revision: std::env::var("GITHUB_SHA")
            .ok()
            .or_else(git_revision)
            .unwrap_or_else(|| "unknown".into()),
        jambda_revision: zk_jam_translation::JAMBDA_REVISION.into(),
        openvm_revision: zk_jam_openvm_backend::OPENVM_REVISION.into(),
        rust_toolchain: std::env::var("RUSTUP_TOOLCHAIN").unwrap_or_else(|_| "unknown".into()),
        cost_model_version: COST_MODEL_VERSION.into(),
        translation_version: zk_jam_translation::TRANSLATION_VERSION,
    }
}

fn git_revision() -> Option<String> {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|result| result.status.success())
        .map(|result| String::from_utf8_lossy(&result.stdout).trim().to_string())
}
fn hardware() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            String::from("gpu_model"),
            std::env::var("GPU_MODEL").unwrap_or_else(|_| "unknown".into()),
        ),
        (
            String::from("gpu_count"),
            std::env::var("GPU_COUNT").unwrap_or_else(|_| "0".into()),
        ),
        (String::from("cpu"), std::env::consts::ARCH.into()),
    ])
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<(), CostModelError> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, CostModelError> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn markdown(report: &CombinedReport) -> String {
    let mut out = String::from("# PVM → OpenVM Cost Model\n\n");
    out.push_str("A+B execution-only report. No ZK proving is performed.\n\n| Workload | PVM gas | Core OpenVM | Envelope | Proof work v1 | Core alpha |\n|---|---:|---:|---:|---:|---:|\n");
    for workload in &report.workloads {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {:.4} |\n",
            workload.name,
            workload.pvm_gas,
            workload.pvm_core_instructions,
            workload.proof_envelope_instructions,
            workload
                .proof_work_v1
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            workload.core_alpha.unwrap_or_default()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use zk_jam_openvm_backend::M4PublicValuesV1;

    #[test]
    fn sixteen_item_latency_is_not_summed_work() {
        let model = CalibrationReport {
            t0_seconds: 1.0,
            k_seconds_per_work: 0.001,
            throughput_work_per_second: 1000.0,
            r_squared: 1.0,
            max_relative_error: 0.0,
            calibration_status: "linear".into(),
        };
        let combined = CombinedReport {
            schema_version: schema::SCHEMA_VERSION.into(),
            environment: environment(),
            proof_work_definition: "test".into(),
            workloads: vec![CombinedWorkloadReport {
                name: "x".into(),
                pvm_gas: 1,
                pvm_core_instructions: 1,
                proof_envelope_instructions: 1,
                total_static_instructions: 2,
                executed_instruction_count: 1,
                proof_work_v1: Some(1),
                static_instruction_ratio: 2.0,
                proof_work_per_pvm_gas: Some(1.0),
                core_alpha: Some(2.0),
                total_alpha: Some(1.0),
                measurement_status: "complete".into(),
            }],
        };
        let report = aggregate::aggregate(&combined, Some(&model)).unwrap();
        assert_eq!(report.sixteen_item_latency_seconds, Some(624001.0));
        assert!(
            report.sixteen_item_total_gpu_work_seconds.unwrap()
                > report.sixteen_item_latency_seconds.unwrap()
        );
    }

    #[test]
    fn all_cost_workloads_pass_execute_only_statement_check() {
        let backend = OpenVmBackend;
        for workload in workload::all() {
            let lowered = NativePvmLowerer::default()
                .lower(&workload.program, workload.output_register)
                .unwrap();
            let artifact = backend
                .program_from_vm_exe(
                    workload.benchmark.clone(),
                    lowered.exe,
                    "test: direct cost-model execute",
                )
                .unwrap();
            let execution = backend.execute(&artifact, workload.input).unwrap();
            let reference = execute_reference(
                &workload.program,
                &ExecutionInputV1::new(vec![workload.input.a, workload.input.b]),
                workload.output_register,
            )
            .unwrap();
            let mut output = [0u8; 32];
            output[..4].copy_from_slice(&(reference as u32).to_le_bytes());
            let expected = M4PublicValuesV1 {
                program_commitment: program_commitment(&workload.program),
                input_commitment: input_commitment(&ExecutionInputV1::new(vec![
                    workload.input.a,
                    workload.input.b,
                ])),
                output,
            }
            .encode_openvm();
            assert_eq!(execution.public_output, expected, "{}", workload.name);
        }
    }
}
