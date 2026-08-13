use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use zk_jam_openvm_backend::{
    M2Benchmark, M2Input, M4ExpectedStatement, M4ProofArtifact, OpenVmBackend,
    OPENVM_PINNED_GUEST_TOOLCHAIN, OPENVM_REVISION, OPENVM_VERSION,
};
use zk_jam_translation::{
    emit_openvm_guest, execute_reference, input_commitment, program_commitment, translate,
    workload_program, ExecutionInputV1, M3Workload, TRANSLATION_VERSION,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M4CaseRecord {
    pub name: String,
    pub pvm_program_commitment: String,
    pub translated_program_commitment: String,
    pub input_commitment: String,
    pub reference_output_hex: String,
    pub proven_output_hex: String,
    pub translation_ns: u128,
    pub emission_ns: u128,
    pub build_ns: u128,
    pub transpile_ns: u128,
    pub keygen_ns: u128,
    pub execute_ns: u128,
    pub prove_ns: u128,
    pub verify_ns: u128,
    pub proof_bytes: usize,
    pub peak_rss_bytes: Option<u64>,
    pub proof_verified: bool,
    pub program_binding_verified: bool,
    pub input_binding_verified: bool,
    pub output_matches_reference: bool,
    pub error: Option<String>,
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M4BenchmarkReport {
    pub schema_version: String,
    pub translation_version: u32,
    pub zk_jam_revision: String,
    pub git_dirty: bool,
    pub jambda_repository: String,
    pub jambda_revision: String,
    pub jambda_provenance_verified: bool,
    pub openvm_version: String,
    pub openvm_revision: String,
    pub guest_toolchain: String,
    pub backend: String,
    pub samples: usize,
    pub warmup: usize,
    pub programs: usize,
    pub cases: Vec<M4CaseRecord>,
    pub complete: bool,
    pub publication_ready: bool,
}

pub fn validate_m4_report(report_path: &Path, schema_path: &Path) -> Result<()> {
    let schema: serde_json::Value = serde_json::from_slice(&fs::read(schema_path)?)?;
    let report: serde_json::Value = serde_json::from_slice(&fs::read(report_path)?)?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|error| eyre!("compile M4 JSON Schema: {error}"))?;
    let errors = validator
        .iter_errors(&report)
        .map(|error| format!("{} at {}", error, error.instance_path))
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(eyre!(
            "M4 benchmark report failed JSON Schema validation:\n{}",
            errors.join("\n")
        ));
    }
    let typed: M4BenchmarkReport = serde_json::from_value(report)?;
    let complete = typed.cases.len() == 6
        && typed.cases.iter().all(|case| {
            case.error.is_none()
                && case.complete
                && case.proof_verified
                && case.program_binding_verified
                && case.input_binding_verified
                && case.output_matches_reference
        });
    if typed.complete != complete {
        return Err(eyre!("M4 report complete does not match case semantics"));
    }
    if typed.publication_ready
        != (typed.complete
            && typed.jambda_provenance_verified
            && !typed.git_dirty
            && typed.translation_version == TRANSLATION_VERSION
            && typed.guest_toolchain == OPENVM_PINNED_GUEST_TOOLCHAIN)
    {
        return Err(eyre!(
            "M4 report publication_ready does not match readiness semantics"
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct M4InputCase {
    name: &'static str,
    workload: M3Workload,
    input: [u32; 2],
    output_register: u8,
}

const M4_CASES: [M4InputCase; 6] = [
    M4InputCase {
        name: "arithmetic-input-a",
        workload: M3Workload::Arithmetic,
        input: [7, 9],
        output_register: 7,
    },
    M4InputCase {
        name: "arithmetic-input-b",
        workload: M3Workload::Arithmetic,
        input: [10, 20],
        output_register: 7,
    },
    M4InputCase {
        name: "branch-true",
        workload: M3Workload::BranchTrue,
        input: [21, 8],
        output_register: 5,
    },
    M4InputCase {
        name: "branch-false",
        workload: M3Workload::BranchTrue,
        input: [8, 21],
        output_register: 5,
    },
    M4InputCase {
        name: "branch-equal",
        workload: M3Workload::BranchTrue,
        input: [8, 8],
        output_register: 5,
    },
    M4InputCase {
        name: "memory-16KiB",
        workload: M3Workload::Memory16K,
        input: [0x1234_5678, 16 * 1024],
        output_register: 2,
    },
];

pub fn run_m4(
    output_root: &Path,
    samples: usize,
    warmup: usize,
    jambda_repo: &Path,
) -> Result<M4BenchmarkReport> {
    if samples == 0 {
        return Err(eyre!("--samples must be at least 1"));
    }
    let manifest =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../integration/jambda-m3.json");
    let provenance = crate::verify_jambda_provenance(jambda_repo, &manifest)?;
    let zk_jam_revision = command("git", &["rev-parse", "HEAD"])?;
    let git_dirty =
        !command("git", &["status", "--porcelain", "--untracked-files=all"])?.is_empty();
    let mut cases = Vec::with_capacity(M4_CASES.len());
    let started = Instant::now();
    let backend = OpenVmBackend;
    let mut prepared_programs = Vec::new();

    for workload in M3Workload::ALL {
        let program = workload_program(workload);
        let translation_started = Instant::now();
        let translated = translate(&program)?;
        let translation_ns = translation_started.elapsed().as_nanos();
        let emission_started = Instant::now();
        let emitted = emit_openvm_guest(&translated, output_register(workload))?;
        let emission_ns = emission_started.elapsed().as_nanos();
        let guest_dir = generated_guest(&emitted.source)?;
        let benchmark = match workload {
            M3Workload::Arithmetic | M3Workload::BranchTrue => M2Benchmark::Arithmetic,
            M3Workload::Memory16K => M2Benchmark::Memory { bytes: 16 * 1024 },
        };
        let artifact = backend.program_from_guest_dir(benchmark, &guest_dir, "m4-generated-v1")?;
        let prepared = backend.prepare(artifact)?;
        let _ = fs::remove_dir_all(&guest_dir);
        prepared_programs.push((
            workload,
            program,
            translated,
            emitted,
            prepared,
            translation_ns,
            emission_ns,
        ));
    }

    for case in M4_CASES {
        let (_, program, translated, _emitted, prepared, translation_ns, emission_ns) =
            prepared_programs
                .iter()
                .find(|(workload, ..)| *workload == case.workload)
                .ok_or_else(|| eyre!("missing prepared M4 workload"))?;
        let input = ExecutionInputV1::new(case.input.to_vec());
        let reference_output = execute_reference(program, &input, case.output_register)? as u32;
        let m2_input = M2Input::arithmetic(case.input[0], case.input[1]);
        let execute_started = Instant::now();
        let execution = backend.execute_prepared(prepared, m2_input)?;
        let execute_ns = execute_started.elapsed().as_nanos();
        let prove_started = Instant::now();
        let proof = backend.prove_prepared(prepared, m2_input)?;
        let prove_ns = prove_started.elapsed().as_nanos();
        let pvm_commitment = program_commitment(program);
        let input_commit = input_commitment(&input);
        let mut expected_output = vec![0u8; 32];
        expected_output[..4].copy_from_slice(&reference_output.to_le_bytes());
        let proof_values = &proof.public_output;
        let proof_program_ok = proof_values.get(..32) == Some(&pvm_commitment);
        let proof_input_ok = proof_values.get(32..64) == Some(&input_commit);
        let proven_output = proof_values.get(64..96).unwrap_or(&[]).to_vec();
        let output_matches = proven_output == expected_output
            && execution.public_output.get(..4) == Some(&reference_output.to_le_bytes());
        let artifact = M4ProofArtifact {
            schema_version: 1,
            program_commitment: pvm_commitment,
            input_commitment: input_commit,
            public_output: expected_output.clone(),
            proof,
        };
        let verify_started = Instant::now();
        let proof_verified = artifact
            .verify_m4(
                &M4ExpectedStatement {
                    program_commitment: pvm_commitment,
                    input_commitment: input_commit,
                    public_output: expected_output.clone(),
                },
                m2_input,
            )
            .is_ok();
        let verify_ns = verify_started.elapsed().as_nanos();
        let complete = proof_verified && proof_program_ok && proof_input_ok && output_matches;
        cases.push(M4CaseRecord {
            name: case.name.to_string(),
            pvm_program_commitment: hex(&pvm_commitment),
            translated_program_commitment: hex(&zk_jam_translation::translated_program_commitment(
                translated,
            )),
            input_commitment: hex(&input_commit),
            reference_output_hex: hex(&expected_output),
            proven_output_hex: hex(&proven_output),
            translation_ns: *translation_ns,
            emission_ns: *emission_ns,
            build_ns: prepared.program.build_time_ns,
            transpile_ns: prepared.program.transpile_time_ns,
            keygen_ns: prepared.keygen_time_ns,
            execute_ns,
            prove_ns,
            verify_ns,
            proof_bytes: artifact.proof.proof_payload_size_bytes(),
            peak_rss_bytes: None,
            proof_verified,
            program_binding_verified: proof_program_ok,
            input_binding_verified: proof_input_ok,
            output_matches_reference: output_matches,
            error: None,
            complete,
        });
    }

    let complete = cases.len() == M4_CASES.len() && cases.iter().all(|case| case.complete);
    let publication_ready = complete
        && provenance.verified
        && !git_dirty
        && zk_jam_revision.len() == 40
        && OPENVM_VERSION == "2.0.1"
        && OPENVM_REVISION.len() == 40
        && OPENVM_PINNED_GUEST_TOOLCHAIN == "nightly-2026-01-18";
    let report = M4BenchmarkReport {
        schema_version: "m4-proven-translation-v1".to_string(),
        translation_version: TRANSLATION_VERSION,
        zk_jam_revision,
        git_dirty,
        jambda_repository: provenance.repository,
        jambda_revision: provenance.revision,
        jambda_provenance_verified: provenance.verified,
        openvm_version: OPENVM_VERSION.to_string(),
        openvm_revision: OPENVM_REVISION.to_string(),
        guest_toolchain: OPENVM_PINNED_GUEST_TOOLCHAIN.to_string(),
        backend: "cpu".to_string(),
        samples,
        warmup,
        programs: 3,
        cases,
        complete,
        publication_ready,
    };
    let run_id = format!("m4-{}", command("date", &["-u", "+%Y%m%d-%H%M%SZ"])?);
    let result_dir = output_root.join(run_id);
    fs::create_dir_all(&result_dir)?;
    fs::write(
        result_dir.join("m4-benchmark.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(result_dir.join("m4-benchmark.csv"), render_csv(&report))?;
    fs::write(
        result_dir.join("m4-benchmark.md"),
        render_markdown(&report, started.elapsed().as_nanos()),
    )?;
    Ok(report)
}

fn output_register(workload: M3Workload) -> u8 {
    match workload {
        M3Workload::Arithmetic => 7,
        M3Workload::BranchTrue => 5,
        M3Workload::Memory16K => 2,
    }
}

fn generated_guest(source: &str) -> Result<PathBuf> {
    let root = std::env::temp_dir().join(format!(
        "zk-jam-m4-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/m4_generated.rs"), source)?;
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "zk-jam-m4-generated"
version = "0.1.0"
edition = "2021"

[workspace]

[dependencies]
openvm = { version = "2.0.1", git = "https://github.com/openvm-org/openvm.git", rev = "b820b25baab6c5d9b055f64e0286b6b1058e707c", features = ["std"] }
sha2 = { version = "0.10", default-features = false }

[[bin]]
name = "m4-generated-v1"
path = "src/m4_generated.rs"
"#,
    )?;
    Ok(root)
}

fn unique_suffix() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}

fn command(command: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(command).args(args).output()?;
    if !output.status.success() {
        return Err(eyre!("command {command} failed"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn render_csv(report: &M4BenchmarkReport) -> String {
    let mut output = String::from("name,pvm_program_commitment,translated_program_commitment,input_commitment,reference_output_hex,proven_output_hex,translation_ns,emission_ns,build_ns,transpile_ns,keygen_ns,execute_ns,prove_ns,verify_ns,proof_bytes,complete\n");
    for case in &report.cases {
        output.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            case.name,
            case.pvm_program_commitment,
            case.translated_program_commitment,
            case.input_commitment,
            case.reference_output_hex,
            case.proven_output_hex,
            case.translation_ns,
            case.emission_ns,
            case.build_ns,
            case.transpile_ns,
            case.keygen_ns,
            case.execute_ns,
            case.prove_ns,
            case.verify_ns,
            case.proof_bytes,
            case.complete
        ));
    }
    output
}

fn render_markdown(report: &M4BenchmarkReport, elapsed_ns: u128) -> String {
    let mut output = format!("# ZK-JAM M4 Proven Translation Benchmark\n\n- Translation version: `{}`\n- ZK-JAM revision: `{}`\n- Git dirty: `{}`\n- Jambda: `{}`\n- Jambda revision: `{}`\n- Jambda provenance verified: `{}`\n- OpenVM: `{}` at `{}`\n- Guest toolchain: `{}`\n- Programs: `{}`\n- Cases: `{}`\n- Complete: `{}`\n- Publication ready: `{}`\n- Collection time: {} ns\n\n", report.translation_version, report.zk_jam_revision, report.git_dirty, report.jambda_repository, report.jambda_revision, report.jambda_provenance_verified, report.openvm_version, report.openvm_revision, report.guest_toolchain, report.programs, report.cases.len(), report.complete, report.publication_ready, elapsed_ns);
    output.push_str("| Case | Prove ns | Verify ns | Proof bytes | Program bound | Input bound | Output matches | Complete |\n|---|---:|---:|---:|:---:|:---:|:---:|:---:|\n");
    for case in &report.cases {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            case.name,
            case.prove_ns,
            case.verify_ns,
            case.proof_bytes,
            case.program_binding_verified,
            case.input_binding_verified,
            case.output_matches_reference,
            case.complete
        ));
    }
    output.push_str("\nM4 proves the checked-in bounded PVM fixtures through deterministic Translation IR and generated OpenVM guest source. Full JAM Refine, Host Calls, GAS, sub-VM, Native AIR, and consensus semantics are outside this milestone.\n");
    output
}
