//! M4.1 NativePvm differential gate and three-way publication benchmark.
//!
//! This is intentionally separate from the M4 correctness report. The gate is execute-only; the
//! publication workload reuses the existing M4 proving façade and compares DirectGuest,
//! FrontendTranslated, and NativePvm on one runner per workload.

use std::{fs, path::Path, time::Instant};

use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use zk_jam_openvm_backend::{
    native_pvm::NativePvmLowerer, M2Benchmark, M2Input, M4ExpectedStatement, M4ProofArtifact,
    M4PublicValuesV1, OpenVmBackend,
};
use zk_jam_translation::{
    execute_reference, input_commitment, program_commitment, workload_program, ExecutionInputV1,
};

use crate::{
    m4::{build_m4_program, m4_case_specs, M4ProgramId},
    read_json, write_json,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M41PreflightCase {
    pub program: String,
    pub name: String,
    pub input: [u32; 2],
    pub reference_output_hex: String,
    pub direct_guest_output_hex: String,
    pub frontend_translated_output_hex: String,
    pub native_pvm_output_hex: String,
    pub direct_guest_match: bool,
    pub frontend_translated_match: bool,
    pub native_pvm_match: bool,
    pub public_values_len: usize,
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M41PreflightReport {
    pub schema_version: String,
    pub cases: Vec<M41PreflightCase>,
    pub complete: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct M41Side {
    pub build_ns: Option<u128>,
    pub transpile_ns: Option<u128>,
    pub native_lowering_ns: Option<u128>,
    pub keygen_ns: Option<u128>,
    pub execute_ns: Option<u128>,
    pub prove_ns: Option<u128>,
    pub verify_ns: Option<u128>,
    pub proof_bytes: Option<usize>,
    pub executable_bytes: Option<usize>,
    pub output_hex: Option<String>,
    pub public_values_len: Option<usize>,
    pub proof_verified: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M41WorkloadReport {
    pub schema_version: String,
    pub workload: String,
    pub input: [u32; 2],
    pub reference_output_hex: String,
    pub pvm_instruction_count: usize,
    pub translated_instruction_count: usize,
    pub direct_guest: M41Side,
    pub frontend_translated: M41Side,
    pub native_pvm: M41Side,
    pub semantics_match: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M41BenchmarkReport {
    pub schema_version: String,
    pub workloads: Vec<M41WorkloadReport>,
    pub comparison_status: String,
    pub comparison_complete: bool,
}

fn representative_input(workload: M4ProgramId) -> [u32; 2] {
    match workload {
        M4ProgramId::Arithmetic => [7, 9],
        M4ProgramId::Branch => [21, 8],
        M4ProgramId::Memory16K => [0x1234_5678, 16 * 1024],
    }
}

fn native_benchmark(workload: M4ProgramId) -> M2Benchmark {
    workload.native_benchmark()
}

fn execute_values(
    backend: &OpenVmBackend,
    artifact: &zk_jam_openvm_backend::OpenVmProgramArtifact,
    input: [u32; 2],
) -> Result<M4PublicValuesV1> {
    let execution = backend.execute(artifact, M2Input::arithmetic(input[0], input[1]))?;
    M4PublicValuesV1::decode_openvm(&execution.public_output)
        .map_err(|error| eyre!("decode M4.1 public values: {error}"))
}

pub fn run_m4_1_preflight(output_root: &Path) -> Result<M41PreflightReport> {
    let backend = OpenVmBackend;
    let mut artifacts = Vec::new();
    for program in M4ProgramId::ALL {
        let frontend = build_m4_program(&backend, program)?;
        let direct = backend.m4_native_program(native_benchmark(program))?;
        let native =
            NativePvmLowerer::default().lower(&frontend.program, program.output_register())?;
        let native = backend.program_from_vm_exe(
            native_benchmark(program),
            native.exe,
            "NativePvm: PvmProgramV1 -> OpenVM Instructions",
        )?;
        artifacts.push((program, frontend.artifact, direct, native));
    }

    let mut cases = Vec::new();
    for case in m4_case_specs() {
        let (_, frontend, direct, native) = artifacts
            .iter()
            .find(|(program, _, _, _)| *program == case.program)
            .ok_or_else(|| eyre!("missing M4.1 preflight artifact"))?;
        let source = workload_program(case.program.workload());
        let input = ExecutionInputV1::new(case.input.to_vec());
        let reference = execute_reference(&source, &input, case.program.output_register())? as u32;
        let mut expected = [0u8; 32];
        expected[..4].copy_from_slice(&reference.to_le_bytes());
        let expected_hex = hex(&expected);
        let direct_values = execute_values(&backend, direct, case.input)?;
        let frontend_values = execute_values(&backend, frontend, case.input)?;
        let native_values = execute_values(&backend, native, case.input)?;
        let direct_match = direct_values.program_commitment == program_commitment(&source)
            && direct_values.input_commitment == input_commitment(&input)
            && direct_values.output == expected;
        let frontend_match = frontend_values.program_commitment == program_commitment(&source)
            && frontend_values.input_commitment == input_commitment(&input)
            && frontend_values.output == expected;
        let native_match = native_values.program_commitment == program_commitment(&source)
            && native_values.input_commitment == input_commitment(&input)
            && native_values.output == expected;
        cases.push(M41PreflightCase {
            program: case.program.name().to_string(),
            name: case.name.to_string(),
            input: case.input,
            reference_output_hex: expected_hex,
            direct_guest_output_hex: hex(&direct_values.output),
            frontend_translated_output_hex: hex(&frontend_values.output),
            native_pvm_output_hex: hex(&native_values.output),
            direct_guest_match: direct_match,
            frontend_translated_match: frontend_match,
            native_pvm_match: native_match,
            public_values_len: M4PublicValuesV1::OPENVM_LEN,
            complete: direct_match && frontend_match && native_match,
        });
    }
    let report = M41PreflightReport {
        schema_version: "m4.1-preflight-v1".to_string(),
        complete: cases.len() == 6 && cases.iter().all(|case| case.complete),
        cases,
    };
    let result_dir = output_root.join(format!("m4.1-preflight-{}", timestamp()));
    fs::create_dir_all(&result_dir)?;
    write_json(result_dir.join("m4.1-preflight.json"), &report)?;
    fs::write(
        result_dir.join("m4.1-preflight.md"),
        render_preflight(&report),
    )?;
    if !report.complete {
        return Err(eyre!("M4.1 six-case execute-only gate failed"));
    }
    Ok(report)
}

fn prove_side(
    backend: &OpenVmBackend,
    artifact: zk_jam_openvm_backend::OpenVmProgramArtifact,
    input: [u32; 2],
    expected: &M4ExpectedStatement,
    native_lowering_ns: Option<u128>,
) -> Result<M41Side> {
    let prepared = backend.prepare(artifact)?;
    let execute_started = Instant::now();
    let execution = backend.execute_prepared(&prepared, M2Input::arithmetic(input[0], input[1]))?;
    let execute_ns = execute_started.elapsed().as_nanos();
    let prove_started = Instant::now();
    let proof = backend.prove_prepared(&prepared, M2Input::arithmetic(input[0], input[1]))?;
    let prove_ns = prove_started.elapsed().as_nanos();
    let values = M4PublicValuesV1::decode_openvm(&proof.public_output)?;
    let artifact = M4ProofArtifact {
        schema_version: 1,
        program_commitment: expected.program_commitment,
        input_commitment: expected.input_commitment,
        public_output: expected.public_output,
        proof,
    };
    let verify_started = Instant::now();
    let proof_verified = artifact
        .verify_m4(expected, M2Input::arithmetic(input[0], input[1]))
        .is_ok();
    let verify_ns = verify_started.elapsed().as_nanos();
    Ok(M41Side {
        build_ns: Some(prepared.program.build_time_ns),
        transpile_ns: Some(prepared.program.transpile_time_ns),
        native_lowering_ns,
        keygen_ns: Some(prepared.keygen_time_ns),
        execute_ns: Some(execute_ns),
        prove_ns: Some(prove_ns),
        verify_ns: Some(verify_ns),
        proof_bytes: Some(artifact.proof.proof_payload_size_bytes()),
        executable_bytes: Some(prepared.program.serialized_executable_size_bytes),
        output_hex: Some(hex(&values.output)),
        public_values_len: Some(execution.public_output.len()),
        proof_verified: proof_verified && values.output == expected.public_output,
        error: None,
    })
}

pub fn run_m4_1_publication_workload(
    output_root: &Path,
    preflight_path: &Path,
    workload: M4ProgramId,
) -> Result<M41WorkloadReport> {
    let preflight: M41PreflightReport = read_json(preflight_path)?;
    if !preflight.complete {
        return Err(eyre!(
            "M4.1 publication requires a complete six-case preflight"
        ));
    }
    let backend = OpenVmBackend;
    let input = representative_input(workload);
    let source = workload_program(workload.workload());
    let input_value = ExecutionInputV1::new(input.to_vec());
    let reference = execute_reference(&source, &input_value, workload.output_register())? as u32;
    let mut expected_output = [0u8; 32];
    expected_output[..4].copy_from_slice(&reference.to_le_bytes());
    let expected = M4ExpectedStatement {
        program_commitment: program_commitment(&source),
        input_commitment: input_commitment(&input_value),
        public_output: expected_output,
    };

    // These calls are deliberately sequential: all three implementations share this runner.
    let direct_guest = prove_side(
        &backend,
        backend.m4_native_program(native_benchmark(workload))?,
        input,
        &expected,
        None,
    )?;
    let frontend = build_m4_program(&backend, workload)?;
    let translated_count = frontend.translated.translated_instruction_count();
    let frontend_translated = prove_side(&backend, frontend.artifact, input, &expected, None)?;
    let lowering_started = Instant::now();
    let native = NativePvmLowerer::default().lower(&source, workload.output_register())?;
    let native_lowering_ns = lowering_started.elapsed().as_nanos();
    let native_pvm = prove_side(
        &backend,
        backend.program_from_vm_exe(
            native_benchmark(workload),
            native.exe,
            "NativePvm: PvmProgramV1 -> OpenVM Instructions",
        )?,
        input,
        &expected,
        Some(native_lowering_ns),
    )?;
    let semantics_match = [&direct_guest, &frontend_translated, &native_pvm]
        .iter()
        .all(|side| {
            side.proof_verified
                && side.output_hex.as_deref() == Some(hex(&expected_output).as_str())
        });
    let report = M41WorkloadReport {
        schema_version: "m4.1-native-pvm-workload-v1".to_string(),
        workload: workload.name().to_string(),
        input,
        reference_output_hex: hex(&expected_output),
        pvm_instruction_count: source.instruction_count(),
        translated_instruction_count: translated_count,
        direct_guest,
        frontend_translated,
        native_pvm,
        semantics_match,
    };
    fs::create_dir_all(output_root)?;
    write_json(
        output_root.join(format!("m4.1-native-pvm-{}.json", workload.name())),
        &report,
    )?;
    if !semantics_match {
        return Err(eyre!(
            "M4.1 {} three-way semantics mismatch",
            workload.name()
        ));
    }
    Ok(report)
}

fn unavailable(workload: M4ProgramId, error: String) -> M41WorkloadReport {
    let source = workload_program(workload.workload());
    let input = representative_input(workload);
    let mut output = [0u8; 32];
    output[..4].copy_from_slice(
        &(execute_reference(
            &source,
            &ExecutionInputV1::new(input.to_vec()),
            workload.output_register(),
        )
        .unwrap_or_default() as u32)
            .to_le_bytes(),
    );
    let failed = M41Side {
        error: Some(error),
        ..M41Side::default()
    };
    M41WorkloadReport {
        schema_version: "m4.1-native-pvm-workload-v1".to_string(),
        workload: workload.name().to_string(),
        input,
        reference_output_hex: hex(&output),
        pvm_instruction_count: source.instruction_count(),
        translated_instruction_count: 0,
        direct_guest: failed.clone(),
        frontend_translated: failed.clone(),
        native_pvm: failed,
        semantics_match: false,
    }
}

pub fn aggregate_m4_1_publication(
    output_root: &Path,
    partial_paths: [&Path; 3],
) -> Result<M41BenchmarkReport> {
    let workloads = M4ProgramId::ALL
        .into_iter()
        .zip(partial_paths)
        .map(|(workload, path)| {
            if path.exists() {
                read_json(path).unwrap_or_else(|error| unavailable(workload, error.to_string()))
            } else {
                unavailable(
                    workload,
                    "publication workload artifact unavailable".to_string(),
                )
            }
        })
        .collect::<Vec<_>>();
    let complete =
        workloads.len() == 3 && workloads.iter().all(|workload| workload.semantics_match);
    let report = M41BenchmarkReport {
        schema_version: "m4.1-native-pvm-v1".to_string(),
        comparison_status: if complete {
            "complete"
        } else if workloads.iter().all(|workload| !workload.semantics_match) {
            "unavailable"
        } else {
            "partial"
        }
        .to_string(),
        comparison_complete: complete,
        workloads,
    };
    let result_dir = output_root.join(format!("m4.1-native-pvm-{}", timestamp()));
    fs::create_dir_all(&result_dir)?;
    write_json(result_dir.join("m4.1-native-pvm.json"), &report)?;
    fs::write(result_dir.join("m4.1-native-pvm.csv"), render_csv(&report))?;
    fs::write(
        result_dir.join("m4.1-comparison.csv"),
        render_summary_csv(&report),
    )?;
    fs::write(
        result_dir.join("m4.1-native-pvm.md"),
        render_markdown(&report),
    )?;
    Ok(report)
}

fn render_csv(report: &M41BenchmarkReport) -> String {
    let mut csv = String::from("workload,implementation,metric,value\n");
    for workload in &report.workloads {
        for (name, side) in [
            ("DirectGuest", &workload.direct_guest),
            ("FrontendTranslated", &workload.frontend_translated),
            ("NativePvm", &workload.native_pvm),
        ] {
            for (metric, value) in [
                ("build_ns", side.build_ns.map(|v| v.to_string())),
                ("transpile_ns", side.transpile_ns.map(|v| v.to_string())),
                (
                    "native_lowering_ns",
                    side.native_lowering_ns.map(|v| v.to_string()),
                ),
                ("keygen_ns", side.keygen_ns.map(|v| v.to_string())),
                ("execute_ns", side.execute_ns.map(|v| v.to_string())),
                ("prove_ns", side.prove_ns.map(|v| v.to_string())),
                ("verify_ns", side.verify_ns.map(|v| v.to_string())),
                ("proof_bytes", side.proof_bytes.map(|v| v.to_string())),
                (
                    "executable_bytes",
                    side.executable_bytes.map(|v| v.to_string()),
                ),
            ] {
                csv.push_str(&format!(
                    "{},{},{},{}\n",
                    workload.workload,
                    name,
                    metric,
                    value.unwrap_or_default()
                ));
            }
        }
    }
    csv
}

fn render_summary_csv(report: &M41BenchmarkReport) -> String {
    let mut csv = String::from(
        "workload,pvm_instruction_count,translated_instruction_count,semantics_match\n",
    );
    for workload in &report.workloads {
        csv.push_str(&format!(
            "{},{},{},{}\n",
            workload.workload,
            workload.pvm_instruction_count,
            workload.translated_instruction_count,
            workload.semantics_match
        ));
    }
    csv
}

fn render_preflight(report: &M41PreflightReport) -> String {
    let mut markdown = format!(
        "# M4.1 execute-only preflight\n\ncomplete: `{}`\n\n",
        report.complete
    );
    markdown.push_str("| program | case | DirectGuest | FrontendTranslated | NativePvm |\n|---|---|---:|---:|---:|\n");
    for case in &report.cases {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            case.program,
            case.name,
            case.direct_guest_match,
            case.frontend_translated_match,
            case.native_pvm_match
        ));
    }
    markdown
}

fn render_markdown(report: &M41BenchmarkReport) -> String {
    let mut markdown = format!(
        "# M4.1 NativePvm benchmark\n\nstatus: `{}`\n\n",
        report.comparison_status
    );
    markdown.push_str(
        "| workload | DirectGuest | FrontendTranslated | NativePvm |\n|---|---:|---:|---:|\n",
    );
    for workload in &report.workloads {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            workload.workload,
            workload.direct_guest.proof_verified,
            workload.frontend_translated.proof_verified,
            workload.native_pvm.proof_verified
        ));
    }
    markdown
}

fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
