use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use eyre::{eyre, Result, WrapErr};
use serde::{Deserialize, Serialize};
use zk_jam_openvm_backend::{
    M2Benchmark, M2Input, M4ExpectedStatement, M4ProofArtifact, M4PublicValuesV1, OpenVmBackend,
    OPENVM_PINNED_GUEST_TOOLCHAIN, OPENVM_REVISION, OPENVM_VERSION,
};
use zk_jam_translation::{
    emit_openvm_guest, execute_reference, input_commitment, program_commitment, translate,
    workload_program, ExecutionInputV1, M3Workload, TRANSLATION_VERSION,
};

use crate::{read_json, write_json};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M4CaseRecord {
    pub program: String,
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
    pub app_keygen_ns: u128,
    pub agg_keygen_ns: u128,
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
    pub program_reuse_verified: bool,
    pub complete: bool,
    pub publication_ready: bool,
}

pub fn validate_m4_preflight_report(report_path: &Path, schema_path: &Path) -> Result<()> {
    let schema: serde_json::Value = serde_json::from_slice(&fs::read(schema_path)?)?;
    let report: serde_json::Value = serde_json::from_slice(&fs::read(report_path)?)?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|error| eyre!("compile M4 preflight JSON Schema: {error}"))?;
    let errors = validator
        .iter_errors(&report)
        .map(|error| format!("{} at {}", error, error.instance_path))
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(eyre!(
            "M4 preflight report failed JSON Schema validation:\n{}",
            errors.join("\n")
        ));
    }
    let typed: M4PreflightReport = serde_json::from_value(report)?;
    let complete = typed.programs == 3
        && typed.cases.len() == 6
        && typed.cases.iter().all(|case| {
            case.error.is_none()
                && case.complete
                && case.program_binding_verified
                && case.input_binding_verified
                && case.output_matches_reference
                && case.public_values_len == M4PublicValuesV1::LEN
        });
    if typed.complete != complete {
        return Err(eyre!("M4 preflight complete does not match case semantics"));
    }
    Ok(())
}

pub fn validate_m4_proof_partial_report(report_path: &Path, schema_path: &Path) -> Result<()> {
    let schema: serde_json::Value = serde_json::from_slice(&fs::read(schema_path)?)?;
    let report: serde_json::Value = serde_json::from_slice(&fs::read(report_path)?)?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|error| eyre!("compile M4 proof partial JSON Schema: {error}"))?;
    let errors = validator
        .iter_errors(&report)
        .map(|error| format!("{} at {}", error, error.instance_path))
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(eyre!(
            "M4 proof partial report failed JSON Schema validation:\n{}",
            errors.join("\n")
        ));
    }
    let typed: M4ProofPartialReport = serde_json::from_value(report)?;
    let expected_cases = m4_program_specs()
        .iter()
        .find(|spec| spec.id.name() == typed.program)
        .map(|spec| spec.cases.len())
        .ok_or_else(|| eyre!("unknown M4 proof program: {}", typed.program))?;
    let complete = typed.cases.len() == expected_cases
        && typed.cases.iter().all(|case| {
            case.program == typed.program
                && case.error.is_none()
                && case.complete
                && case.proof_verified
                && case.program_binding_verified
                && case.input_binding_verified
                && case.output_matches_reference
        });
    if typed.complete != complete {
        return Err(eyre!(
            "M4 proof partial complete does not match case semantics"
        ));
    }
    Ok(())
}

pub fn validate_m4_publication_report(report_path: &Path, schema_path: &Path) -> Result<()> {
    let schema: serde_json::Value = serde_json::from_slice(&fs::read(schema_path)?)?;
    let report: serde_json::Value = serde_json::from_slice(&fs::read(report_path)?)?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|error| eyre!("compile M4 publication JSON Schema: {error}"))?;
    let errors = validator
        .iter_errors(&report)
        .map(|error| format!("{} at {}", error, error.instance_path))
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(eyre!(
            "M4 publication report failed JSON Schema validation:\n{}",
            errors.join("\n")
        ));
    }
    let typed: M4PublicationReport = serde_json::from_value(report)?;
    let complete = typed.workloads.len() == 3
        && typed.workloads.iter().all(|workload| {
            workload.semantics_match
                && workload.pvm_instruction_count > 0
                && workload.translated_instruction_count > 0
        });
    if typed.comparison_complete != complete {
        return Err(eyre!(
            "M4 publication comparison_complete does not match workload semantics"
        ));
    }
    let expected_status = if complete {
        "complete"
    } else if typed
        .workloads
        .iter()
        .flat_map(|workload| [&workload.native, &workload.translated])
        .all(|side| side.error.is_some())
    {
        "unavailable"
    } else {
        "partial"
    };
    if typed.comparison_status != expected_status {
        return Err(eyre!(
            "M4 publication comparison_status does not match workload availability"
        ));
    }
    Ok(())
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
    if typed.samples != 1 || typed.warmup != 0 {
        return Err(eyre!(
            "M4 functional reports require samples=1 and warmup=0"
        ));
    }
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
    let arithmetic_commits = typed
        .cases
        .iter()
        .filter(|case| case.program == "arithmetic")
        .map(|case| case.pvm_program_commitment.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let branch_commits = typed
        .cases
        .iter()
        .filter(|case| case.program == "branch")
        .map(|case| case.pvm_program_commitment.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let reuse = arithmetic_commits.len() == 1 && branch_commits.len() == 1;
    if typed.program_reuse_verified != reuse {
        return Err(eyre!(
            "M4 program_reuse_verified does not match case commitments"
        ));
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum M4ProgramId {
    Arithmetic,
    Branch,
    Memory16K,
}

impl M4ProgramId {
    pub const ALL: [Self; 3] = [Self::Arithmetic, Self::Branch, Self::Memory16K];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Arithmetic => "arithmetic",
            Self::Branch => "branch",
            Self::Memory16K => "memory",
        }
    }

    pub const fn workload(self) -> M3Workload {
        match self {
            Self::Arithmetic => M3Workload::Arithmetic,
            Self::Branch => M3Workload::BranchTrue,
            Self::Memory16K => M3Workload::Memory16K,
        }
    }

    pub const fn output_register(self) -> u8 {
        match self {
            Self::Arithmetic => 7,
            Self::Branch => 5,
            Self::Memory16K => 2,
        }
    }

    pub const fn benchmark(self) -> M2Benchmark {
        match self {
            Self::Arithmetic => M2Benchmark::M4GeneratedArithmetic,
            Self::Branch => M2Benchmark::M4GeneratedBranch,
            Self::Memory16K => M2Benchmark::M4GeneratedMemory16K,
        }
    }

    pub const fn native_benchmark(self) -> M2Benchmark {
        match self {
            Self::Arithmetic => M2Benchmark::M4NativeArithmetic,
            Self::Branch => M2Benchmark::M4NativeBranch,
            Self::Memory16K => M2Benchmark::M4NativeMemory16K,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct M4CaseSpec {
    pub name: &'static str,
    pub program: M4ProgramId,
    pub input: [u32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct M4ProgramSpec {
    pub id: M4ProgramId,
    pub cases: &'static [M4CaseSpec],
}

const ARITHMETIC_CASES: [M4CaseSpec; 2] = [
    M4CaseSpec {
        name: "arithmetic-input-a",
        program: M4ProgramId::Arithmetic,
        input: [7, 9],
    },
    M4CaseSpec {
        name: "arithmetic-input-b",
        program: M4ProgramId::Arithmetic,
        input: [10, 20],
    },
];

const BRANCH_CASES: [M4CaseSpec; 3] = [
    M4CaseSpec {
        name: "branch-true",
        program: M4ProgramId::Branch,
        input: [21, 8],
    },
    M4CaseSpec {
        name: "branch-false",
        program: M4ProgramId::Branch,
        input: [8, 21],
    },
    M4CaseSpec {
        name: "branch-equal",
        program: M4ProgramId::Branch,
        input: [8, 8],
    },
];

const MEMORY_CASES: [M4CaseSpec; 1] = [M4CaseSpec {
    name: "memory-16KiB",
    program: M4ProgramId::Memory16K,
    input: [0x1234_5678, 16 * 1024],
}];

pub const M4_PROGRAM_SPECS: [M4ProgramSpec; 3] = [
    M4ProgramSpec {
        id: M4ProgramId::Arithmetic,
        cases: &ARITHMETIC_CASES,
    },
    M4ProgramSpec {
        id: M4ProgramId::Branch,
        cases: &BRANCH_CASES,
    },
    M4ProgramSpec {
        id: M4ProgramId::Memory16K,
        cases: &MEMORY_CASES,
    },
];

pub fn m4_program_specs() -> &'static [M4ProgramSpec] {
    &M4_PROGRAM_SPECS
}

pub fn m4_case_specs() -> impl Iterator<Item = &'static M4CaseSpec> {
    M4_PROGRAM_SPECS
        .iter()
        .flat_map(|program| program.cases.iter())
}

fn m4_case_count() -> usize {
    m4_case_specs().count()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M4PreflightCaseRecord {
    pub program: String,
    pub name: String,
    pub pvm_program_commitment: String,
    pub input_commitment: String,
    pub reference_output_hex: String,
    pub execution_output_hex: String,
    pub translation_ns: u128,
    pub emission_ns: u128,
    pub build_ns: u128,
    pub transpile_ns: u128,
    pub public_values_len: usize,
    pub execute_ns: u128,
    pub program_binding_verified: bool,
    pub input_binding_verified: bool,
    pub output_matches_reference: bool,
    pub error: Option<String>,
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M4PreflightReport {
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
    pub programs: usize,
    pub cases: Vec<M4PreflightCaseRecord>,
    pub complete: bool,
}

struct M4BuiltProgram {
    id: M4ProgramId,
    program: zk_jam_refine_interface::PvmProgramV1,
    translated: zk_jam_translation::TranslatedProgramV1,
    artifact: zk_jam_openvm_backend::OpenVmProgramArtifact,
    translation_ns: u128,
    emission_ns: u128,
}

fn build_m4_program(backend: &OpenVmBackend, id: M4ProgramId) -> Result<M4BuiltProgram> {
    let program = workload_program(id.workload());
    let translation_started = Instant::now();
    let translated = translate(&program)?;
    let translation_ns = translation_started.elapsed().as_nanos();
    println!(
        "[M4][{}] translation: {} ms",
        id.name(),
        translation_ns as f64 / 1_000_000.0
    );
    let emission_started = Instant::now();
    let emitted = emit_openvm_guest(&translated, id.output_register())?;
    let emission_ns = emission_started.elapsed().as_nanos();
    println!(
        "[M4][{}] emission: {} ms",
        id.name(),
        emission_ns as f64 / 1_000_000.0
    );
    let guest_dir = generated_guest(&emitted.source)?;
    let build_started = Instant::now();
    let artifact = backend.program_from_guest_dir(id.benchmark(), &guest_dir, "m4-generated-v1")?;
    let _ = fs::remove_dir_all(&guest_dir);
    println!(
        "[M4][{}] build: {} s, transpile: {} ms",
        id.name(),
        artifact.build_time_ns as f64 / 1_000_000_000.0,
        artifact.transpile_time_ns as f64 / 1_000_000.0
    );
    debug_assert!(build_started.elapsed().as_nanos() >= artifact.build_time_ns);
    Ok(M4BuiltProgram {
        id,
        program,
        translated,
        artifact,
        translation_ns,
        emission_ns,
    })
}

pub fn run_m4_preflight(output_root: &Path, jambda_repo: &Path) -> Result<M4PreflightReport> {
    let manifest =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../integration/jambda-m3.json");
    let provenance = crate::verify_jambda_provenance(jambda_repo, &manifest)?;
    let zk_jam_revision = command("git", &["rev-parse", "HEAD"])?;
    let git_dirty =
        !command("git", &["status", "--porcelain", "--untracked-files=all"])?.is_empty();
    let backend = OpenVmBackend;
    let mut built = Vec::with_capacity(M4_PROGRAM_SPECS.len());
    for spec in m4_program_specs() {
        built.push(build_m4_program(&backend, spec.id)?);
    }
    let mut cases = Vec::with_capacity(m4_case_count());
    for case in m4_case_specs() {
        let program = built
            .iter()
            .find(|program| program.id == case.program)
            .ok_or_else(|| eyre!("missing M4 preflight program"))?;
        let input = ExecutionInputV1::new(case.input.to_vec());
        let reference_output =
            execute_reference(&program.program, &input, case.program.output_register())? as u32;
        let mut expected_output = [0u8; 32];
        expected_output[..4].copy_from_slice(&reference_output.to_le_bytes());
        let input_commit = input_commitment(&input);
        let m2_input = M2Input::arithmetic(case.input[0], case.input[1]);
        println!(
            "[M4][{}][{}] execute started",
            case.program.name(),
            case.name
        );
        let execute_started = Instant::now();
        let execution = backend.execute(&program.artifact, m2_input)?;
        let execute_ns = execute_started.elapsed().as_nanos();
        let public_values = M4PublicValuesV1::decode(&execution.public_output)?;
        let program_ok = public_values.program_commitment == program_commitment(&program.program);
        let input_ok = public_values.input_commitment == input_commit;
        let output_ok = public_values.output == expected_output;
        let complete = program_ok && input_ok && output_ok;
        println!(
            "[M4][{}][{}] execute completed: {} ms, complete={}",
            case.program.name(),
            case.name,
            execute_ns as f64 / 1_000_000.0,
            complete
        );
        cases.push(M4PreflightCaseRecord {
            program: case.program.name().to_string(),
            name: case.name.to_string(),
            pvm_program_commitment: hex(&program_commitment(&program.program)),
            input_commitment: hex(&input_commit),
            reference_output_hex: hex(&expected_output),
            execution_output_hex: hex(&public_values.output),
            translation_ns: program.translation_ns,
            emission_ns: program.emission_ns,
            build_ns: program.artifact.build_time_ns,
            transpile_ns: program.artifact.transpile_time_ns,
            public_values_len: execution.public_output.len(),
            execute_ns,
            program_binding_verified: program_ok,
            input_binding_verified: input_ok,
            output_matches_reference: output_ok,
            error: None,
            complete,
        });
    }
    let complete = cases.len() == m4_case_count() && cases.iter().all(|case| case.complete);
    let report = M4PreflightReport {
        schema_version: "m4-preflight-v1".to_string(),
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
        programs: M4_PROGRAM_SPECS.len(),
        cases,
        complete,
    };
    let result_dir = output_root.join(format!(
        "m4-preflight-{}",
        command("date", &["-u", "+%Y%m%d-%H%M%SZ"])?
    ));
    fs::create_dir_all(&result_dir)?;
    fs::write(
        result_dir.join("m4-preflight.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(
        result_dir.join("m4-preflight.md"),
        render_preflight_markdown(&report),
    )?;
    Ok(report)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M4ProofPartialReport {
    pub schema_version: String,
    pub translation_version: u32,
    pub program: String,
    pub zk_jam_revision: String,
    pub git_dirty: bool,
    pub jambda_repository: String,
    pub jambda_revision: String,
    pub jambda_provenance_verified: bool,
    pub openvm_version: String,
    pub openvm_revision: String,
    pub guest_toolchain: String,
    pub backend: String,
    pub cases: Vec<M4CaseRecord>,
    pub complete: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct M4PublicationSide {
    pub build_ns: Option<u128>,
    pub transpile_ns: Option<u128>,
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
    pub output_hex: Option<String>,
    pub public_values_len: Option<usize>,
    pub proof_verified: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct M4PublicationRatios {
    pub execute_overhead_ratio: Option<f64>,
    pub prove_overhead_ratio: Option<f64>,
    pub verify_overhead_ratio: Option<f64>,
    pub keygen_overhead_ratio: Option<f64>,
    pub proof_size_overhead_ratio: Option<f64>,
    pub peak_rss_overhead_ratio: Option<f64>,
    pub executable_size_overhead_ratio: Option<f64>,
    pub serialized_executable_size_overhead_ratio: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct M4FixedCostObservation {
    pub translated_pipeline_ns: Option<u128>,
    pub translation_share_of_pipeline: Option<f64>,
    pub prove_share_of_pipeline: Option<f64>,
    pub keygen_share_of_pipeline: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M4PublicationWorkload {
    pub name: String,
    pub input: [u32; 2],
    pub reference_output_hex: String,
    pub pvm_instruction_count: usize,
    pub translated_instruction_count: usize,
    pub instruction_expansion_ratio: f64,
    pub translation_ns: u128,
    pub emission_ns: u128,
    pub reference_execute_ns: Option<u128>,
    pub native: M4PublicationSide,
    pub translated: M4PublicationSide,
    pub ratios: M4PublicationRatios,
    pub fixed_cost_observation: M4FixedCostObservation,
    pub semantics_match: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M4PublicationReport {
    pub schema_version: String,
    pub zk_jam_revision: String,
    pub git_dirty: bool,
    pub jambda_repository: String,
    pub jambda_revision: String,
    pub jambda_provenance_verified: bool,
    pub openvm_version: String,
    pub openvm_revision: String,
    pub guest_toolchain: String,
    pub backend: String,
    pub security_bits: u32,
    pub runner: crate::EnvironmentReport,
    pub m4_complete: bool,
    pub m4_publication_ready: bool,
    pub comparison_status: String,
    pub comparison_complete: bool,
    pub single_sample_diagnostic: bool,
    pub workloads: Vec<M4PublicationWorkload>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct M4PublicationWorkerOutput {
    pub side: M4PublicationSide,
    pub pvm_instruction_count: usize,
    pub translated_instruction_count: usize,
    pub translation_ns: u128,
    pub emission_ns: u128,
}

pub fn run_m4_publication_worker(
    implementation: &str,
    workload: M4ProgramId,
    input: [u32; 2],
    output: &Path,
) -> Result<()> {
    let backend = OpenVmBackend;
    let source_program = workload_program(workload.workload());
    let (artifact, translation_ns, emission_ns, translated_instruction_count) =
        if implementation == "translated" {
            let built = build_m4_program(&backend, workload)?;
            (
                built.artifact,
                built.translation_ns,
                built.emission_ns,
                built.translated.translated_instruction_count(),
            )
        } else if implementation == "native" {
            (
                backend.m4_native_program(workload.native_benchmark())?,
                0,
                0,
                0,
            )
        } else {
            return Err(eyre!(
                "unknown M4 publication implementation: {implementation}"
            ));
        };
    let input_value = ExecutionInputV1::new(input.to_vec());
    let reference_output =
        execute_reference(&source_program, &input_value, workload.output_register())? as u32;
    let mut expected_output = [0u8; 32];
    expected_output[..4].copy_from_slice(&reference_output.to_le_bytes());
    let pvm_commitment = program_commitment(&source_program);
    let input_commit = input_commitment(&input_value);
    let prepared = backend.prepare(artifact)?;
    let execute_started = Instant::now();
    let execution = backend.execute_prepared(&prepared, M2Input::arithmetic(input[0], input[1]))?;
    let execute_ns = execute_started.elapsed().as_nanos();
    let execution_values = M4PublicValuesV1::decode(&execution.public_output)?;
    let prove_started = Instant::now();
    let proof = backend.prove_prepared(&prepared, M2Input::arithmetic(input[0], input[1]))?;
    let prove_ns = prove_started.elapsed().as_nanos();
    let proof_values = M4PublicValuesV1::decode(&proof.public_output)?;
    let artifact = M4ProofArtifact {
        schema_version: 1,
        program_commitment: pvm_commitment,
        input_commitment: input_commit,
        public_output: expected_output,
        proof,
    };
    let verify_started = Instant::now();
    let proof_verified = artifact
        .verify_m4(
            &M4ExpectedStatement {
                program_commitment: pvm_commitment,
                input_commitment: input_commit,
                public_output: expected_output,
            },
            M2Input::arithmetic(input[0], input[1]),
        )
        .is_ok();
    let verify_ns = verify_started.elapsed().as_nanos();
    let side = M4PublicationSide {
        build_ns: Some(prepared.program.build_time_ns),
        transpile_ns: Some(prepared.program.transpile_time_ns),
        app_keygen_ns: Some(prepared.app_keygen_time_ns),
        agg_keygen_ns: Some(prepared.agg_keygen_time_ns),
        keygen_ns: Some(prepared.keygen_time_ns),
        execute_ns: Some(execute_ns),
        prove_ns: Some(prove_ns),
        verify_ns: Some(verify_ns),
        proof_bytes: Some(artifact.proof.proof_payload_size_bytes()),
        peak_rss_bytes: process_peak_rss_bytes(),
        executable_bytes: Some(prepared.program.executable_bytes),
        serialized_executable_bytes: Some(prepared.program.serialized_executable_size_bytes),
        output_hex: Some(hex(&proof_values.output)),
        public_values_len: Some(execution.public_output.len()),
        proof_verified: proof_verified
            && execution_values == proof_values
            && execution_values.output == expected_output,
        error: None,
    };
    write_json(
        output.to_path_buf(),
        &M4PublicationWorkerOutput {
            side,
            pvm_instruction_count: source_program.instruction_count(),
            translated_instruction_count,
            translation_ns,
            emission_ns,
        },
    )
}

fn representative_workloads() -> [(M4ProgramId, [u32; 2]); 3] {
    [
        (M4ProgramId::Arithmetic, [7, 9]),
        (M4ProgramId::Branch, [21, 8]),
        (M4ProgramId::Memory16K, [0x1234_5678, 16 * 1024]),
    ]
}

fn collect_m4_publication_workload(
    output_root: &Path,
    run_id: &str,
    workload: M4ProgramId,
    input: [u32; 2],
) -> Result<M4PublicationWorkload> {
    let mut sides = [M4PublicationSide::default(), M4PublicationSide::default()];
    let mut metadata = [(0usize, 0usize, 0u128, 0u128); 2];
    // Keep Native and Translated sequential on this process/runner. The matrix
    // in CI parallelizes only across workloads.
    for implementation in ["native", "translated"] {
        let slot = if implementation == "native" { 0 } else { 1 };
        let worker_path = output_root.join(format!(
            ".m4-publication-{run_id}-{implementation}-{}.json",
            workload.name()
        ));
        let executable = env::current_exe().wrap_err("locate M4 publication worker")?;
        let a = input[0].to_string();
        let b = input[1].to_string();
        let output_text = worker_path
            .to_str()
            .ok_or_else(|| eyre!("invalid publication worker path"))?;
        let status = Command::new(&executable)
            .args([
                "__m4-publication-worker",
                "--implementation",
                implementation,
                "--workload",
                workload.name(),
                "--a",
                &a,
                "--b",
                &b,
                "--output",
                output_text,
            ])
            .status()
            .wrap_err("spawn M4 publication worker")?;
        if status.success() {
            let worker: M4PublicationWorkerOutput = read_json(&worker_path)?;
            sides[slot] = worker.side;
            metadata[slot] = (
                worker.pvm_instruction_count,
                worker.translated_instruction_count,
                worker.translation_ns,
                worker.emission_ns,
            );
        } else {
            sides[slot].error = Some(format!("{implementation} publication worker failed"));
        }
        let _ = fs::remove_file(worker_path);
    }

    let reference_started = Instant::now();
    let reference_output = execute_reference(
        &workload_program(workload.workload()),
        &ExecutionInputV1::new(input.to_vec()),
        workload.output_register(),
    )? as u32;
    let reference_execute_ns = reference_started.elapsed().as_nanos();
    let mut expected_output = [0u8; 32];
    expected_output[..4].copy_from_slice(&reference_output.to_le_bytes());
    let expected_hex = hex(&expected_output);
    let semantics_match = sides.iter().all(|side| {
        side.proof_verified
            && side.error.is_none()
            && side.public_values_len == Some(M4PublicValuesV1::LEN)
            && side.output_hex.as_deref() == Some(expected_hex.as_str())
    }) && sides[0].output_hex == sides[1].output_hex;
    let translated = &sides[1];
    let native = &sides[0];
    let ratios = M4PublicationRatios {
        execute_overhead_ratio: ratio_opt(translated.execute_ns, native.execute_ns),
        prove_overhead_ratio: ratio_opt(translated.prove_ns, native.prove_ns),
        verify_overhead_ratio: ratio_opt(translated.verify_ns, native.verify_ns),
        keygen_overhead_ratio: ratio_opt(translated.keygen_ns, native.keygen_ns),
        proof_size_overhead_ratio: ratio_opt_usize(translated.proof_bytes, native.proof_bytes),
        peak_rss_overhead_ratio: ratio_opt_u64(translated.peak_rss_bytes, native.peak_rss_bytes),
        executable_size_overhead_ratio: ratio_opt_usize(
            translated.executable_bytes,
            native.executable_bytes,
        ),
        serialized_executable_size_overhead_ratio: ratio_opt_usize(
            translated.serialized_executable_bytes,
            native.serialized_executable_bytes,
        ),
    };
    let translated_pipeline_ns = sum_pipeline(translated, metadata[1].2, metadata[1].3);
    let fixed_cost_observation = M4FixedCostObservation {
        translated_pipeline_ns,
        translation_share_of_pipeline: share(metadata[1].2, translated_pipeline_ns),
        prove_share_of_pipeline: share_opt(translated.prove_ns, translated_pipeline_ns),
        keygen_share_of_pipeline: share_opt(translated.keygen_ns, translated_pipeline_ns),
    };
    Ok(M4PublicationWorkload {
        name: workload.name().to_string(),
        input,
        reference_output_hex: expected_hex,
        pvm_instruction_count: metadata[0].0,
        translated_instruction_count: metadata[1].1,
        instruction_expansion_ratio: if metadata[0].0 == 0 {
            0.0
        } else {
            metadata[1].1 as f64 / metadata[0].0 as f64
        },
        translation_ns: metadata[1].2,
        emission_ns: metadata[1].3,
        reference_execute_ns: Some(reference_execute_ns),
        native: sides[0].clone(),
        translated: sides[1].clone(),
        ratios,
        fixed_cost_observation,
        semantics_match,
    })
}

fn publication_report(
    m4_report: &M4BenchmarkReport,
    run_id: &str,
    workloads: Vec<M4PublicationWorkload>,
) -> Result<M4PublicationReport> {
    let runner = crate::environment_report(run_id, "cpu")?;
    let available = workloads
        .iter()
        .flat_map(|workload| [&workload.native, &workload.translated])
        .filter(|side| side.error.is_none())
        .count();
    let comparison_complete =
        workloads.len() == 3 && workloads.iter().all(|workload| workload.semantics_match);
    let comparison_status = if comparison_complete {
        "complete"
    } else if available == 0 {
        "unavailable"
    } else {
        "partial"
    };
    let report = M4PublicationReport {
        schema_version: "m4-publication-v1".to_string(),
        zk_jam_revision: m4_report.zk_jam_revision.clone(),
        git_dirty: m4_report.git_dirty,
        jambda_repository: m4_report.jambda_repository.clone(),
        jambda_revision: m4_report.jambda_revision.clone(),
        jambda_provenance_verified: m4_report.jambda_provenance_verified,
        openvm_version: m4_report.openvm_version.clone(),
        openvm_revision: m4_report.openvm_revision.clone(),
        guest_toolchain: m4_report.guest_toolchain.clone(),
        backend: m4_report.backend.clone(),
        security_bits: 100,
        runner,
        m4_complete: m4_report.complete,
        m4_publication_ready: m4_report.publication_ready,
        comparison_status: comparison_status.to_string(),
        comparison_complete,
        single_sample_diagnostic: true,
        workloads,
    };
    Ok(report)
}

fn write_publication_artifacts(
    output_root: &Path,
    run_id: &str,
    report: &M4PublicationReport,
) -> Result<()> {
    let result_dir = output_root.join(run_id);
    fs::create_dir_all(&result_dir)?;
    write_json(result_dir.join("m4-publication.json"), &report)?;
    fs::write(
        result_dir.join("m4-publication.csv"),
        render_publication_csv(report),
    )?;
    fs::write(
        result_dir.join("m4-comparison.csv"),
        render_publication_summary_csv(report),
    )?;
    fs::write(
        result_dir.join("m4-publication.md"),
        render_publication_markdown(report),
    )?;
    Ok(())
}

pub fn run_m4_publication_workload(
    output_root: &Path,
    m4_report_path: &Path,
    workload: M4ProgramId,
) -> Result<M4PublicationWorkload> {
    let _: M4BenchmarkReport = read_json(m4_report_path)?;
    fs::create_dir_all(output_root)?;
    let run_id = format!(
        "m4-publication-{}-{}",
        workload.name(),
        command("date", &["-u", "+%Y%m%d-%H%M%SZ"])?
    );
    let input = representative_workloads()
        .into_iter()
        .find(|(candidate, _)| *candidate == workload)
        .map(|(_, input)| input)
        .ok_or_else(|| eyre!("unsupported M4 publication workload"))?;
    let record = collect_m4_publication_workload(output_root, &run_id, workload, input)?;
    write_json(
        output_root.join(format!("m4-publication-{}.json", workload.name())),
        &record,
    )?;
    Ok(record)
}

fn unavailable_publication_workload(
    workload: M4ProgramId,
    input: [u32; 2],
    error: String,
) -> Result<M4PublicationWorkload> {
    let program = workload_program(workload.workload());
    let reference_output = execute_reference(
        &program,
        &ExecutionInputV1::new(input.to_vec()),
        workload.output_register(),
    )? as u32;
    let mut expected_output = [0u8; 32];
    expected_output[..4].copy_from_slice(&reference_output.to_le_bytes());
    let failed_side = M4PublicationSide {
        error: Some(error),
        ..M4PublicationSide::default()
    };
    Ok(M4PublicationWorkload {
        name: workload.name().to_string(),
        input,
        reference_output_hex: hex(&expected_output),
        pvm_instruction_count: program.instruction_count(),
        translated_instruction_count: 0,
        instruction_expansion_ratio: 0.0,
        translation_ns: 0,
        emission_ns: 0,
        reference_execute_ns: None,
        native: failed_side.clone(),
        translated: failed_side,
        ratios: M4PublicationRatios::default(),
        fixed_cost_observation: M4FixedCostObservation::default(),
        semantics_match: false,
    })
}

pub fn aggregate_m4_publication(
    output_root: &Path,
    m4_report_path: &Path,
    partial_paths: [&Path; 3],
) -> Result<M4PublicationReport> {
    let m4_report: M4BenchmarkReport = read_json(m4_report_path)?;
    fs::create_dir_all(output_root)?;
    let run_id = format!(
        "m4-publication-{}",
        command("date", &["-u", "+%Y%m%d-%H%M%SZ"])?
    );
    let workloads = representative_workloads()
        .into_iter()
        .zip(partial_paths)
        .map(|((workload, input), path)| {
            if path.exists() {
                match read_json::<M4PublicationWorkload>(path) {
                    Ok(record) if record.name == workload.name() => Ok(record),
                    Ok(record) => unavailable_publication_workload(
                        workload,
                        input,
                        format!("partial report names {}", record.name),
                    ),
                    Err(error) => unavailable_publication_workload(
                        workload,
                        input,
                        format!("invalid or unreadable partial report: {error}"),
                    ),
                }
            } else {
                unavailable_publication_workload(
                    workload,
                    input,
                    "publication workload artifact unavailable".to_string(),
                )
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let report = publication_report(&m4_report, &run_id, workloads)?;
    write_publication_artifacts(output_root, &run_id, &report)?;
    Ok(report)
}

pub fn run_m4_publication(
    output_root: &Path,
    m4_report_path: &Path,
) -> Result<M4PublicationReport> {
    let m4_report: M4BenchmarkReport = read_json(m4_report_path)?;
    fs::create_dir_all(output_root)?;
    let run_id = format!(
        "m4-publication-{}",
        command("date", &["-u", "+%Y%m%d-%H%M%SZ"])?
    );
    let workloads = representative_workloads()
        .into_iter()
        .map(|(workload, input)| {
            collect_m4_publication_workload(output_root, &run_id, workload, input)
        })
        .collect::<Result<Vec<_>>>()?;
    let report = publication_report(&m4_report, &run_id, workloads)?;
    write_publication_artifacts(output_root, &run_id, &report)?;
    Ok(report)
}

fn sum_pipeline(side: &M4PublicationSide, translation_ns: u128, emission_ns: u128) -> Option<u128> {
    Some(
        translation_ns
            + emission_ns
            + side.build_ns?
            + side.transpile_ns?
            + side.keygen_ns?
            + side.prove_ns?,
    )
}

fn share(value: u128, total: Option<u128>) -> Option<f64> {
    Some(value as f64 / total? as f64)
}
fn share_opt(value: Option<u128>, total: Option<u128>) -> Option<f64> {
    Some(value? as f64 / total? as f64)
}
fn ratio_opt(numerator: Option<u128>, denominator: Option<u128>) -> Option<f64> {
    Some(numerator? as f64 / denominator? as f64)
}
fn ratio_opt_usize(numerator: Option<usize>, denominator: Option<usize>) -> Option<f64> {
    Some(numerator? as f64 / denominator? as f64)
}
fn ratio_opt_u64(numerator: Option<u64>, denominator: Option<u64>) -> Option<f64> {
    Some(numerator? as f64 / denominator? as f64)
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

fn render_publication_csv(report: &M4PublicationReport) -> String {
    let mut csv = String::from("workload,implementation,metric,value,unit\n");
    for workload in &report.workloads {
        for (implementation, side) in [
            ("native", &workload.native),
            ("translated", &workload.translated),
        ] {
            let values = [
                (
                    "build_ns",
                    side.build_ns.map(|value| value.to_string()),
                    "ns",
                ),
                (
                    "transpile_ns",
                    side.transpile_ns.map(|value| value.to_string()),
                    "ns",
                ),
                (
                    "app_keygen_ns",
                    side.app_keygen_ns.map(|value| value.to_string()),
                    "ns",
                ),
                (
                    "agg_keygen_ns",
                    side.agg_keygen_ns.map(|value| value.to_string()),
                    "ns",
                ),
                (
                    "keygen_ns",
                    side.keygen_ns.map(|value| value.to_string()),
                    "ns",
                ),
                (
                    "execute_ns",
                    side.execute_ns.map(|value| value.to_string()),
                    "ns",
                ),
                (
                    "prove_ns",
                    side.prove_ns.map(|value| value.to_string()),
                    "ns",
                ),
                (
                    "verify_ns",
                    side.verify_ns.map(|value| value.to_string()),
                    "ns",
                ),
                (
                    "proof_bytes",
                    side.proof_bytes.map(|value| value.to_string()),
                    "bytes",
                ),
                (
                    "peak_rss_bytes",
                    side.peak_rss_bytes.map(|value| value.to_string()),
                    "bytes",
                ),
                (
                    "executable_bytes",
                    side.executable_bytes.map(|value| value.to_string()),
                    "bytes",
                ),
                (
                    "serialized_executable_bytes",
                    side.serialized_executable_bytes
                        .map(|value| value.to_string()),
                    "bytes",
                ),
            ];
            for (metric, value, unit) in values {
                csv.push_str(&format!(
                    "{},{},{},{},{}\n",
                    workload.name,
                    implementation,
                    metric,
                    value.unwrap_or_default(),
                    unit
                ));
            }
        }
        for (metric, value) in [
            (
                "execute_overhead_ratio",
                workload.ratios.execute_overhead_ratio,
            ),
            ("prove_overhead_ratio", workload.ratios.prove_overhead_ratio),
            (
                "verify_overhead_ratio",
                workload.ratios.verify_overhead_ratio,
            ),
            (
                "keygen_overhead_ratio",
                workload.ratios.keygen_overhead_ratio,
            ),
            (
                "proof_size_overhead_ratio",
                workload.ratios.proof_size_overhead_ratio,
            ),
            (
                "peak_rss_overhead_ratio",
                workload.ratios.peak_rss_overhead_ratio,
            ),
            (
                "executable_size_overhead_ratio",
                workload.ratios.executable_size_overhead_ratio,
            ),
            (
                "serialized_executable_size_overhead_ratio",
                workload.ratios.serialized_executable_size_overhead_ratio,
            ),
        ] {
            csv.push_str(&format!(
                "{},ratio,{},{},ratio\n",
                workload.name,
                metric,
                value.map_or(String::new(), |value| value.to_string())
            ));
        }
        for (metric, value, unit) in [
            (
                "reference_output",
                Some(workload.reference_output_hex.clone()),
                "hex",
            ),
            (
                "pvm_instruction_count",
                Some(workload.pvm_instruction_count.to_string()),
                "count",
            ),
            (
                "translated_instruction_count",
                Some(workload.translated_instruction_count.to_string()),
                "count",
            ),
            (
                "instruction_expansion_ratio",
                Some(workload.instruction_expansion_ratio.to_string()),
                "ratio",
            ),
            (
                "translation_ns",
                Some(workload.translation_ns.to_string()),
                "ns",
            ),
            ("emission_ns", Some(workload.emission_ns.to_string()), "ns"),
            (
                "reference_execute_ns",
                workload.reference_execute_ns.map(|value| value.to_string()),
                "ns",
            ),
            (
                "translated_pipeline_ns",
                workload
                    .fixed_cost_observation
                    .translated_pipeline_ns
                    .map(|value| value.to_string()),
                "ns",
            ),
            (
                "translation_share_of_pipeline",
                workload
                    .fixed_cost_observation
                    .translation_share_of_pipeline
                    .map(|value| value.to_string()),
                "ratio",
            ),
            (
                "prove_share_of_pipeline",
                workload
                    .fixed_cost_observation
                    .prove_share_of_pipeline
                    .map(|value| value.to_string()),
                "ratio",
            ),
            (
                "keygen_share_of_pipeline",
                workload
                    .fixed_cost_observation
                    .keygen_share_of_pipeline
                    .map(|value| value.to_string()),
                "ratio",
            ),
            (
                "semantics_match",
                Some(workload.semantics_match.to_string()),
                "boolean",
            ),
        ] {
            csv.push_str(&format!(
                "{},metadata,{},{},{}\n",
                workload.name,
                metric,
                value.unwrap_or_default(),
                unit
            ));
        }
    }
    csv
}

fn render_publication_summary_csv(report: &M4PublicationReport) -> String {
    let mut csv = String::from("workload,native_prove_ns,translated_prove_ns,prove_ratio,native_proof_bytes,translated_proof_bytes,proof_size_ratio,native_peak_rss_bytes,translated_peak_rss_bytes,peak_rss_ratio,native_executable_bytes,translated_executable_bytes,executable_size_ratio\n");
    for workload in &report.workloads {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            workload.name,
            optional_u128(workload.native.prove_ns),
            optional_u128(workload.translated.prove_ns),
            optional_f64(workload.ratios.prove_overhead_ratio),
            optional_usize(workload.native.proof_bytes),
            optional_usize(workload.translated.proof_bytes),
            optional_f64(workload.ratios.proof_size_overhead_ratio),
            optional_u64(workload.native.peak_rss_bytes),
            optional_u64(workload.translated.peak_rss_bytes),
            optional_f64(workload.ratios.peak_rss_overhead_ratio),
            optional_usize(workload.native.executable_bytes),
            optional_usize(workload.translated.executable_bytes),
            optional_f64(workload.ratios.executable_size_overhead_ratio),
        ));
    }
    csv
}

fn optional_u128(value: Option<u128>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}
fn optional_usize(value: Option<usize>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}
fn optional_u64(value: Option<u64>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}
fn optional_f64(value: Option<f64>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

fn render_publication_markdown(report: &M4PublicationReport) -> String {
    let mut output = format!("# ZK-JAM M4 Publication Benchmark\n\n- Commit: `{}`\n- OpenVM: `{}` at `{}`\n- Toolchain: `{}`\n- Runner: `{}/{}`\n- M4 correctness: `{}`\n- M4 publication ready: `{}`\n- Comparison status: `{}`\n- Single-sample diagnostic: `{}`\n\n", report.zk_jam_revision, report.openvm_version, report.openvm_revision, report.guest_toolchain, report.runner.os, report.runner.arch, report.m4_complete, report.m4_publication_ready, report.comparison_status, report.single_sample_diagnostic);
    output.push_str("| Workload | Native prove | Translated prove | Overhead | Native proof | Translated proof | Ratio |\n|---|---:|---:|---:|---:|---:|---:|\n");
    for workload in &report.workloads {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            workload.name,
            format_ns(workload.native.prove_ns),
            format_ns(workload.translated.prove_ns),
            format_ratio(workload.ratios.prove_overhead_ratio),
            optional_usize(workload.native.proof_bytes),
            optional_usize(workload.translated.proof_bytes),
            format_ratio(workload.ratios.proof_size_overhead_ratio)
        ));
    }
    output.push_str("\n| Workload | Native RSS | Translated RSS | Ratio | PVM instructions | Translated instructions | Expansion |\n|---|---:|---:|---:|---:|---:|---:|\n");
    for workload in &report.workloads {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {:.2}x |\n",
            workload.name,
            optional_u64(workload.native.peak_rss_bytes),
            optional_u64(workload.translated.peak_rss_bytes),
            format_ratio(workload.ratios.peak_rss_overhead_ratio),
            workload.pvm_instruction_count,
            workload.translated_instruction_count,
            workload.instruction_expansion_ratio
        ));
    }
    output.push_str("\n## Claims boundary\n\nM4 demonstrates that a bounded PVM subset can be deterministically translated into the actual OpenVM executable being proved, with program identity, runtime input, and output bound into the proof public statement. The native comparison is a direct OpenVM guest baseline with the same 96-byte public-values envelope; its embedded program commitment keeps the envelope comparable but does not provide M4 mechanical translation binding.\n\nThese single-sample diagnostic measurements do not demonstrate full JAM Refine, Refine Host Calls, sub-VM, full PVM coverage, production proving performance, or Kusama integration.\n");
    output
}

fn format_ns(value: Option<u128>) -> String {
    value.map_or_else(
        || "n/a".to_string(),
        |value| format!("{:.3} ms", value as f64 / 1_000_000.0),
    )
}
fn format_ratio(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_string(), |value| format!("{value:.2}x"))
}

pub fn run_m4_proof_program(
    output_root: &Path,
    jambda_repo: &Path,
    program_id: M4ProgramId,
) -> Result<M4ProofPartialReport> {
    let manifest =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../integration/jambda-m3.json");
    let provenance = crate::verify_jambda_provenance(jambda_repo, &manifest)?;
    let zk_jam_revision = command("git", &["rev-parse", "HEAD"])?;
    let git_dirty =
        !command("git", &["status", "--porcelain", "--untracked-files=all"])?.is_empty();
    let backend = OpenVmBackend;
    let built = build_m4_program(&backend, program_id)?;
    println!("[M4][{}] keygen started", program_id.name());
    let prepared = backend.prepare(built.artifact)?;
    println!(
        "[M4][{}] keygen completed: {} s (app: {} s, agg: {} s)",
        program_id.name(),
        prepared.keygen_time_ns as f64 / 1_000_000_000.0,
        prepared.app_keygen_time_ns as f64 / 1_000_000_000.0,
        prepared.agg_keygen_time_ns as f64 / 1_000_000_000.0
    );
    let mut cases = Vec::new();
    for case in m4_case_specs().filter(|case| case.program == program_id) {
        let input = ExecutionInputV1::new(case.input.to_vec());
        let reference_output =
            execute_reference(&built.program, &input, program_id.output_register())? as u32;
        let mut expected_output = [0u8; 32];
        expected_output[..4].copy_from_slice(&reference_output.to_le_bytes());
        let pvm_commitment = program_commitment(&built.program);
        let input_commit = input_commitment(&input);
        let m2_input = M2Input::arithmetic(case.input[0], case.input[1]);
        let execute_started = Instant::now();
        let execution = backend.execute_prepared(&prepared, m2_input)?;
        let execute_ns = execute_started.elapsed().as_nanos();
        let execution_values = M4PublicValuesV1::decode(&execution.public_output)?;
        let prove_started = Instant::now();
        println!("[M4][{}][{}] prove started", program_id.name(), case.name);
        let proof = backend.prove_prepared(&prepared, m2_input)?;
        let prove_ns = prove_started.elapsed().as_nanos();
        println!(
            "[M4][{}][{}] prove completed: {} s",
            program_id.name(),
            case.name,
            prove_ns as f64 / 1_000_000_000.0
        );
        let proof_values = M4PublicValuesV1::decode(&proof.public_output)?;
        let program_binding_verified = proof_values.program_commitment == pvm_commitment
            && execution_values.program_commitment == pvm_commitment;
        let input_binding_verified = proof_values.input_commitment == input_commit
            && execution_values.input_commitment == input_commit;
        let output_matches_reference =
            proof_values.output == expected_output && execution_values.output == expected_output;
        let artifact = M4ProofArtifact {
            schema_version: 1,
            program_commitment: pvm_commitment,
            input_commitment: input_commit,
            public_output: expected_output,
            proof,
        };
        let verify_started = Instant::now();
        let proof_verified = artifact
            .verify_m4(
                &M4ExpectedStatement {
                    program_commitment: pvm_commitment,
                    input_commitment: input_commit,
                    public_output: expected_output,
                },
                m2_input,
            )
            .is_ok();
        let verify_ns = verify_started.elapsed().as_nanos();
        let complete = proof_verified
            && program_binding_verified
            && input_binding_verified
            && output_matches_reference;
        cases.push(M4CaseRecord {
            program: program_id.name().to_string(),
            name: case.name.to_string(),
            pvm_program_commitment: hex(&pvm_commitment),
            translated_program_commitment: hex(&zk_jam_translation::translated_program_commitment(
                &built.translated,
            )),
            input_commitment: hex(&input_commit),
            reference_output_hex: hex(&expected_output),
            proven_output_hex: hex(&proof_values.output),
            translation_ns: built.translation_ns,
            emission_ns: built.emission_ns,
            build_ns: prepared.program.build_time_ns,
            transpile_ns: prepared.program.transpile_time_ns,
            app_keygen_ns: prepared.app_keygen_time_ns,
            agg_keygen_ns: prepared.agg_keygen_time_ns,
            keygen_ns: prepared.keygen_time_ns,
            execute_ns,
            prove_ns,
            verify_ns,
            proof_bytes: artifact.proof.proof_payload_size_bytes(),
            peak_rss_bytes: None,
            proof_verified,
            program_binding_verified,
            input_binding_verified,
            output_matches_reference,
            error: None,
            complete,
        });
    }
    let complete = cases.len()
        == m4_program_specs()
            .iter()
            .find(|spec| spec.id == program_id)
            .map_or(0, |spec| spec.cases.len())
        && cases.iter().all(|case| case.complete);
    let report = M4ProofPartialReport {
        schema_version: "m4-proof-partial-v1".to_string(),
        translation_version: TRANSLATION_VERSION,
        program: program_id.name().to_string(),
        zk_jam_revision,
        git_dirty,
        jambda_repository: provenance.repository,
        jambda_revision: provenance.revision,
        jambda_provenance_verified: provenance.verified,
        openvm_version: OPENVM_VERSION.to_string(),
        openvm_revision: OPENVM_REVISION.to_string(),
        guest_toolchain: OPENVM_PINNED_GUEST_TOOLCHAIN.to_string(),
        backend: "cpu".to_string(),
        cases,
        complete,
    };
    let result_dir = output_root.join(format!(
        "m4-proof-{}-{}",
        program_id.name(),
        command("date", &["-u", "+%Y%m%d-%H%M%SZ"])?
    ));
    fs::create_dir_all(&result_dir)?;
    fs::write(
        result_dir.join(format!("m4-proof-{}.json", program_id.name())),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(
        result_dir.join(format!("m4-proof-{}.md", program_id.name())),
        render_proof_partial_markdown(&report),
    )?;
    Ok(report)
}

pub fn aggregate_m4_reports(
    output_root: &Path,
    preflight_path: &Path,
    proof_paths: &[PathBuf],
) -> Result<M4BenchmarkReport> {
    if proof_paths.len() != M4_PROGRAM_SPECS.len() {
        return Err(eyre!(
            "M4 aggregate requires exactly three proof partial reports"
        ));
    }
    let preflight: M4PreflightReport = serde_json::from_slice(&fs::read(preflight_path)?)?;
    let partials = proof_paths
        .iter()
        .map(|path| -> Result<M4ProofPartialReport> {
            Ok(serde_json::from_slice(&fs::read(path)?)?)
        })
        .collect::<Result<Vec<_>>>()?;
    if partials.is_empty() {
        return Err(eyre!("missing M4 proof partials"));
    }
    for partial in &partials {
        if partial.schema_version != "m4-proof-partial-v1"
            || partial.translation_version != preflight.translation_version
            || partial.zk_jam_revision != preflight.zk_jam_revision
            || partial.git_dirty != preflight.git_dirty
            || partial.jambda_repository != preflight.jambda_repository
            || partial.jambda_revision != preflight.jambda_revision
            || partial.jambda_provenance_verified != preflight.jambda_provenance_verified
            || partial.openvm_version != preflight.openvm_version
            || partial.openvm_revision != preflight.openvm_revision
            || partial.guest_toolchain != preflight.guest_toolchain
            || partial.backend != preflight.backend
        {
            return Err(eyre!("M4 proof partial metadata mismatch"));
        }
    }
    let programs = partials
        .iter()
        .map(|partial| partial.program.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let expected_programs = M4ProgramId::ALL
        .iter()
        .map(|program| program.name())
        .collect::<std::collections::BTreeSet<_>>();
    if programs != expected_programs
        || !partials.iter().all(|partial| {
            partial.complete
                && partial
                    .cases
                    .iter()
                    .all(|case| case.program == partial.program)
        })
    {
        return Err(eyre!("M4 proof partial set is incomplete"));
    }
    let preflight_by_name = preflight
        .cases
        .iter()
        .map(|case| (case.name.as_str(), case))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut cases = Vec::with_capacity(m4_case_count());
    let mut case_names = std::collections::BTreeSet::new();
    for partial in &partials {
        for case in &partial.cases {
            if !case_names.insert(case.name.as_str()) {
                return Err(eyre!("duplicate M4 proof case: {}", case.name));
            }
            let preflight_case = preflight_by_name
                .get(case.name.as_str())
                .ok_or_else(|| eyre!("proof case missing from preflight: {}", case.name))?;
            if preflight_case.program != case.program
                || preflight_case.pvm_program_commitment != case.pvm_program_commitment
                || preflight_case.input_commitment != case.input_commitment
                || preflight_case.reference_output_hex != case.reference_output_hex
                || !preflight_case.complete
            {
                return Err(eyre!(
                    "M4 proof case does not match preflight: {}",
                    case.name
                ));
            }
            cases.push(case.clone());
        }
    }
    cases.sort_by(|left, right| left.name.cmp(&right.name));
    if cases.len() != m4_case_count() {
        return Err(eyre!("M4 proof partials do not contain all cases"));
    }
    let arithmetic_commits = cases
        .iter()
        .filter(|case| case.program == "arithmetic")
        .map(|case| case.pvm_program_commitment.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let branch_commits = cases
        .iter()
        .filter(|case| case.program == "branch")
        .map(|case| case.pvm_program_commitment.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let program_reuse_verified = arithmetic_commits.len() == 1 && branch_commits.len() == 1;
    let complete = preflight.complete
        && cases.len() == m4_case_count()
        && cases.iter().all(|case| {
            case.error.is_none()
                && case.complete
                && case.proof_verified
                && case.program_binding_verified
                && case.input_binding_verified
                && case.output_matches_reference
        })
        && program_reuse_verified;
    let publication_ready = complete
        && preflight.jambda_provenance_verified
        && !preflight.git_dirty
        && preflight.translation_version == TRANSLATION_VERSION
        && preflight.guest_toolchain == OPENVM_PINNED_GUEST_TOOLCHAIN
        && preflight.openvm_revision == OPENVM_REVISION;
    let report = M4BenchmarkReport {
        schema_version: "m4-proven-translation-v1".to_string(),
        translation_version: preflight.translation_version,
        zk_jam_revision: preflight.zk_jam_revision,
        git_dirty: preflight.git_dirty,
        jambda_repository: preflight.jambda_repository,
        jambda_revision: preflight.jambda_revision,
        jambda_provenance_verified: preflight.jambda_provenance_verified,
        openvm_version: preflight.openvm_version,
        openvm_revision: preflight.openvm_revision,
        guest_toolchain: preflight.guest_toolchain,
        backend: preflight.backend,
        samples: 1,
        warmup: 0,
        programs: 3,
        cases,
        program_reuse_verified,
        complete,
        publication_ready,
    };
    let result_dir = output_root.join(format!(
        "m4-{}",
        command("date", &["-u", "+%Y%m%d-%H%M%SZ"])?
    ));
    fs::create_dir_all(&result_dir)?;
    fs::write(
        result_dir.join("m4-benchmark.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(result_dir.join("m4-benchmark.csv"), render_csv(&report))?;
    fs::write(
        result_dir.join("m4-benchmark.md"),
        render_markdown(&report, 0),
    )?;
    Ok(report)
}

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
    let mut cases = Vec::with_capacity(m4_case_count());
    let started = Instant::now();
    let backend = OpenVmBackend;
    let mut prepared_programs = Vec::new();

    for workload in M3Workload::ALL {
        let program = workload_program(workload);
        let translation_started = Instant::now();
        let translated = translate(&program)?;
        let translation_ns = translation_started.elapsed().as_nanos();
        let emission_started = Instant::now();
        let program_id = M4ProgramId::ALL
            .into_iter()
            .find(|program_id| program_id.workload() == workload)
            .ok_or_else(|| eyre!("missing M4 program identity"))?;
        let emitted = emit_openvm_guest(&translated, program_id.output_register())?;
        let emission_ns = emission_started.elapsed().as_nanos();
        let guest_dir = generated_guest(&emitted.source)?;
        let benchmark = program_id.benchmark();
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

    for case in m4_case_specs() {
        let program_id = case.program;
        let (_, program, translated, _emitted, prepared, translation_ns, emission_ns) =
            prepared_programs
                .iter()
                .find(|(workload, ..)| *workload == program_id.workload())
                .ok_or_else(|| eyre!("missing prepared M4 workload"))?;
        let input = ExecutionInputV1::new(case.input.to_vec());
        let reference_output =
            execute_reference(program, &input, program_id.output_register())? as u32;
        let m2_input = M2Input::arithmetic(case.input[0], case.input[1]);
        let execute_started = Instant::now();
        let execution = backend.execute_prepared(prepared, m2_input)?;
        let execute_ns = execute_started.elapsed().as_nanos();
        let prove_started = Instant::now();
        let proof = backend.prove_prepared(prepared, m2_input)?;
        let prove_ns = prove_started.elapsed().as_nanos();
        let pvm_commitment = program_commitment(program);
        let input_commit = input_commitment(&input);
        let mut expected_output = [0u8; 32];
        expected_output[..4].copy_from_slice(&reference_output.to_le_bytes());
        let execution_values = M4PublicValuesV1::decode(&execution.public_output)?;
        let proof_values = M4PublicValuesV1::decode(&proof.public_output)?;
        let proof_program_ok = proof_values.program_commitment == pvm_commitment;
        let proof_input_ok = proof_values.input_commitment == input_commit;
        let proven_output = proof_values.output;
        let output_matches =
            proven_output == expected_output && execution_values.output == expected_output;
        let artifact = M4ProofArtifact {
            schema_version: 1,
            program_commitment: pvm_commitment,
            input_commitment: input_commit,
            public_output: expected_output,
            proof,
        };
        let verify_started = Instant::now();
        let proof_verified = artifact
            .verify_m4(
                &M4ExpectedStatement {
                    program_commitment: pvm_commitment,
                    input_commitment: input_commit,
                    public_output: expected_output,
                },
                m2_input,
            )
            .is_ok();
        let verify_ns = verify_started.elapsed().as_nanos();
        let complete = proof_verified && proof_program_ok && proof_input_ok && output_matches;
        cases.push(M4CaseRecord {
            program: program_id.name().to_string(),
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
            app_keygen_ns: prepared.app_keygen_time_ns,
            agg_keygen_ns: prepared.agg_keygen_time_ns,
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

    let complete = cases.len() == m4_case_count() && cases.iter().all(|case| case.complete);
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
        program_reuse_verified: true,
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
    let mut output = String::from("program,name,pvm_program_commitment,translated_program_commitment,input_commitment,reference_output_hex,proven_output_hex,translation_ns,emission_ns,build_ns,transpile_ns,app_keygen_ns,agg_keygen_ns,keygen_ns,execute_ns,prove_ns,verify_ns,proof_bytes,complete\n");
    for case in &report.cases {
        output.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            case.program,
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
            case.app_keygen_ns,
            case.agg_keygen_ns,
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

fn render_preflight_markdown(report: &M4PreflightReport) -> String {
    let passed = report.cases.iter().filter(|case| case.complete).count();
    let mut output = format!(
        "# ZK-JAM M4 Execute-Only Preflight\n\n- Programs: `{}`\n- Cases: `{}` / `{}` passed\n- Complete: `{}`\n\n",
        report.programs,
        passed,
        report.cases.len(),
        report.complete
    );
    output.push_str("| Program | Case | Translate ns | Emit ns | Build ns | Transpile ns | Execute ns | Program bound | Input bound | Output matches | Complete |\n|---|---|---:|---:|---:|---:|---:|:---:|:---:|:---:|:---:|\n");
    for case in &report.cases {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            case.program,
            case.name,
            case.translation_ns,
            case.emission_ns,
            case.build_ns,
            case.transpile_ns,
            case.execute_ns,
            case.program_binding_verified,
            case.input_binding_verified,
            case.output_matches_reference,
            case.complete
        ));
    }
    output
}

fn render_proof_partial_markdown(report: &M4ProofPartialReport) -> String {
    let passed = report.cases.iter().filter(|case| case.complete).count();
    let mut output = format!(
        "# ZK-JAM M4 Proof Partial: {}\n\n- Cases: `{}` / `{}` passed\n- Complete: `{}`\n\n",
        report.program,
        passed,
        report.cases.len(),
        report.complete
    );
    output.push_str("| Case | App keygen ns | Agg keygen ns | Keygen ns | Prove ns | Verify ns | Proof bytes | Complete |\n|---|---:|---:|---:|---:|---:|---:|:---:|\n");
    for case in &report.cases {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            case.name,
            case.app_keygen_ns,
            case.agg_keygen_ns,
            case.keygen_ns,
            case.prove_ns,
            case.verify_ns,
            case.proof_bytes,
            case.complete
        ));
    }
    output
}

fn render_markdown(report: &M4BenchmarkReport, elapsed_ns: u128) -> String {
    let mut output = format!("# ZK-JAM M4 Proven Translation Benchmark\n\n- Translation version: `{}`\n- ZK-JAM revision: `{}`\n- Git dirty: `{}`\n- Jambda: `{}`\n- Jambda revision: `{}`\n- Jambda provenance verified: `{}`\n- OpenVM: `{}` at `{}`\n- Guest toolchain: `{}`\n- Programs: `{}`\n- Program reuse verified: `{}`\n- Cases: `{}`\n- Complete: `{}`\n- Publication ready: `{}`\n- Collection time: {} ns\n\n", report.translation_version, report.zk_jam_revision, report.git_dirty, report.jambda_repository, report.jambda_revision, report.jambda_provenance_verified, report.openvm_version, report.openvm_revision, report.guest_toolchain, report.programs, report.program_reuse_verified, report.cases.len(), report.complete, report.publication_ready, elapsed_ns);
    output.push_str("| Program | Case | Prove ns | Verify ns | Proof bytes | Program bound | Input bound | Output matches | Complete |\n|---|---|---:|---:|---:|:---:|:---:|:---:|:---:|\n");
    for case in &report.cases {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            case.program,
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
