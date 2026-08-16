use std::{env, fs, path::Path, process::Command};

use blake2b_simd::Params;
use eyre::{eyre, Result, WrapErr};
use openvm_sdk::StdIn;
use serde::{Deserialize, Serialize};
use tiny_keccak::{Hasher, Keccak};
use zk_jam_openvm_backend::{M2Benchmark, OpenVmBackend, OpenVmProofArtifact};
use zk_jam_refine_interface::{
    m5_exports_canonical, m5_exports_commitment, m5_hash, m5_profile_id, m5_result_commitment,
    CanonicalCodec, M5ProfileStatementV1, RefineCaseV1, RefineResultV0, M5_CASE_DOMAIN,
};

pub const M5_ADAPTER_SCHEMA: &str = "zk-jam/m5/jambda-adapter/v1";
pub const M5_REPORT_SCHEMA: &str = "zk-jam/m5-zkrefine/v1";
pub const M5_SEGMENT_BYTES: usize = 4_104;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M5JambdaAdapterOutput {
    pub schema: String,
    pub jambda_repository: String,
    pub jambda_revision: String,
    pub case_hex: String,
    pub reference_result_hex: String,
    pub reference_exports_hex: Vec<String>,
    pub work_package_hash_hex: String,
    pub work_report_package_hash_hex: String,
    pub work_report_exports_root_hex: String,
    pub work_report_exports_count: u16,
    pub work_report_result_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M5WorkReportEnvelope {
    pub package_hash_hex: String,
    pub exports_root_hex: String,
    pub exports_count: u16,
    pub result_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M5Report {
    pub schema: String,
    pub complete: bool,
    pub jam_semantics: String,
    pub scope: M5Scope,
    pub host_calls: M5HostCalls,
    pub fixture: M5FixtureSummary,
    pub reference: M5ReferenceSummary,
    pub bindings: M5Bindings,
    pub work_report: M5WorkReportSummary,
    pub proof: M5ProofSummary,
    pub tamper: M5TamperSummary,
    pub jambda_repository: String,
    pub jambda_revision: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M5Scope {
    pub work_items: usize,
    pub gas_proof: bool,
    pub historical_lookup: bool,
    pub inner_pvm: bool,
    pub aggregation: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M5HostCalls {
    pub fetch: bool,
    pub fetch_id: u32,
    pub fetch_modes: Vec<u32>,
    pub export: bool,
    pub export_id: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M5FixtureSummary {
    pub real_work_package: bool,
    pub item_index: u16,
    pub imports: usize,
    pub exports: usize,
    pub segment_bytes: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M5ReferenceSummary {
    pub result_match: bool,
    pub exports_match: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M5Bindings {
    pub case: bool,
    pub program: bool,
    pub work_package: bool,
    pub imports: bool,
    pub result: bool,
    pub exports: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M5WorkReportSummary {
    pub package_hash_match: bool,
    pub result_match: bool,
    pub exports_root_match: bool,
    pub exports_count_match: bool,
    pub gas_verified: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M5ProofSummary {
    pub verified: bool,
    pub serialized_reload_verified: bool,
    pub bytes: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M5TamperSummary {
    pub case_rejected: bool,
    pub result_rejected: bool,
    pub exports_rejected: bool,
    pub statement_rejected: bool,
}

pub fn run_m5_zkrefine(output: &Path, jambda_repo: &Path, adapter: &Path) -> Result<M5Report> {
    if env::var("GITHUB_ACTIONS").as_deref() != Ok("true") {
        return Err(eyre!(
            "M5 proving is CI-only; run .github/workflows/m5-zkrefine.yml"
        ));
    }
    fs::create_dir_all(output).wrap_err("create M5 output directory")?;
    let adapter_dir = output.join("jambda-adapter");
    fs::create_dir_all(&adapter_dir)?;
    let status = Command::new(adapter)
        .args([
            "--jambda-repo",
            jambda_repo
                .to_str()
                .ok_or_else(|| eyre!("invalid Jambda path"))?,
            "--output",
            adapter_dir
                .to_str()
                .ok_or_else(|| eyre!("invalid adapter output path"))?,
        ])
        .status()
        .wrap_err("run pinned Jambda M5 adapter")?;
    if !status.success() {
        return Err(eyre!("pinned Jambda M5 adapter failed"));
    }
    let adapter_output: M5JambdaAdapterOutput = serde_json::from_slice(
        &fs::read(adapter_dir.join("m5-jambda.json")).wrap_err("read Jambda adapter output")?,
    )?;
    if adapter_output.schema != M5_ADAPTER_SCHEMA {
        return Err(eyre!("unsupported M5 adapter schema"));
    }

    let case_bytes = decode_hex(&adapter_output.case_hex)?;
    let case = RefineCaseV1::decode_canonical(&case_bytes)?;
    validate_m5_case(&case)?;
    if case.encode_canonical() != case_bytes {
        return Err(eyre!(
            "Jambda adapter did not return canonical RefineCase bytes"
        ));
    }
    let result_bytes = decode_hex(&adapter_output.reference_result_hex)?;
    let result = RefineResultV0::decode_canonical(&result_bytes)?;
    let exports = adapter_output
        .reference_exports_hex
        .iter()
        .map(|value| decode_hex(value))
        .collect::<Result<Vec<_>>>()?;
    if exports.len() != 1 || exports[0].len() != M5_SEGMENT_BYTES {
        return Err(eyre!(
            "Jambda reference did not return one 4104-byte export"
        ));
    }
    let work_report = M5WorkReportEnvelope {
        package_hash_hex: adapter_output.work_report_package_hash_hex.clone(),
        exports_root_hex: adapter_output.work_report_exports_root_hex.clone(),
        exports_count: adapter_output.work_report_exports_count,
        result_hex: adapter_output.work_report_result_hex.clone(),
    };
    let package_hash = blake2_hash(&case.work_package);
    let package_hash_hex = hex(&package_hash);
    let work_package_bound = package_hash_hex == adapter_output.work_package_hash_hex
        && package_hash_hex == work_report.package_hash_hex;
    let expected_exports_root = exports_root(&exports);
    if !work_package_bound
        || work_report.exports_count != 1
        || work_report.result_hex != adapter_output.reference_result_hex
        || work_report.exports_root_hex != hex(&expected_exports_root)
    {
        return Err(eyre!(
            "Jambda WorkReport does not bind the M5 package/result"
        ));
    }

    let expected = M5ProfileStatementV1 {
        profile_id: m5_profile_id(&case.profile),
        case_commitment: m5_hash(M5_CASE_DOMAIN, &case_bytes),
        result_commitment: m5_result_commitment(&result),
        exports_commitment: m5_exports_commitment(&exports),
    };
    let backend = OpenVmBackend;
    let program = backend.program(M2Benchmark::M5ZkRefine)?;
    let mut stdin = StdIn::default();
    stdin.write(&case_bytes);
    let execution = backend.execute_stdin(&program, stdin.clone())?;
    let statement = M5ProfileStatementV1::decode_openvm(&execution.public_output)
        .map_err(|error| eyre!("decode M5 execution statement: {error}"))?;
    if statement != expected {
        return Err(eyre!(
            "M5 guest execution statement does not match Jambda reference"
        ));
    }
    let context_hash = context_hash(&case_bytes);
    let proof = backend.prove_stdin(&program, stdin, context_hash.clone())?;
    verify_m5_statement(&proof, &expected)?;
    proof.verify(&context_hash)?;
    let proof_bytes = proof.to_bytes()?;
    let reloaded = OpenVmProofArtifact::from_bytes(&proof_bytes)?;
    reloaded.verify(&context_hash)?;
    verify_m5_statement(&reloaded, &expected)?;
    fs::write(output.join("refine-case.bin"), &case_bytes)?;
    fs::write(output.join("result.bin"), &result_bytes)?;
    fs::write(output.join("exports.bin"), m5_exports_canonical(&exports))?;
    fs::write(output.join("proof.json"), &proof_bytes)?;
    fs::write(
        output.join("statement.json"),
        serde_json::to_vec_pretty(&expected.encode_openvm().to_vec())?,
    )?;

    let report = M5Report {
        schema: M5_REPORT_SCHEMA.to_string(),
        complete: true,
        jam_semantics: "0.7.2".to_string(),
        scope: M5Scope {
            work_items: 1,
            gas_proof: false,
            historical_lookup: false,
            inner_pvm: false,
            aggregation: false,
        },
        host_calls: M5HostCalls {
            fetch: true,
            fetch_id: 1,
            fetch_modes: vec![6],
            export: true,
            export_id: 7,
        },
        fixture: M5FixtureSummary {
            real_work_package: true,
            item_index: case.item_index,
            imports: 1,
            exports: 1,
            segment_bytes: M5_SEGMENT_BYTES,
        },
        reference: M5ReferenceSummary {
            result_match: true,
            exports_match: true,
        },
        bindings: M5Bindings {
            case: true,
            program: true,
            work_package: true,
            imports: true,
            result: true,
            exports: true,
        },
        work_report: M5WorkReportSummary {
            package_hash_match: true,
            result_match: true,
            exports_root_match: work_report.exports_root_hex == hex(&expected_exports_root),
            exports_count_match: work_report.exports_count == exports.len() as u16,
            gas_verified: false,
        },
        proof: M5ProofSummary {
            verified: true,
            serialized_reload_verified: true,
            bytes: proof_bytes.len(),
        },
        tamper: M5TamperSummary {
            case_rejected: tamper_rejected(&reloaded, &expected, 0),
            result_rejected: tamper_rejected(&reloaded, &expected, 1),
            exports_rejected: tamper_rejected(&reloaded, &expected, 2),
            statement_rejected: tamper_rejected(&reloaded, &expected, 3),
        },
        jambda_repository: adapter_output.jambda_repository,
        jambda_revision: adapter_output.jambda_revision,
    };
    fs::write(
        output.join("m5-zkrefine.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}

fn validate_m5_case(case: &RefineCaseV1) -> Result<()> {
    case.validate()?;
    if case.item_index != 0
        || case.core_index != 0
        || case.import_segments.len() != 1
        || case.import_segments[0].len() != 1
        || case.import_segments[0][0].len() != M5_SEGMENT_BYTES
        || case.export_offset != 0
        || !case.state_witness.historical_lookups.is_empty()
    {
        return Err(eyre!("M5 case is outside the single-item mode-6 profile"));
    }
    validate_m5_program(&case.program)
}

fn validate_m5_program(program: &zk_jam_refine_interface::PvmProgramV1) -> Result<()> {
    let instructions = program
        .blocks
        .iter()
        .flat_map(|block| block.instructions.iter())
        .collect::<Vec<_>>();
    if program.blocks.len() != 1 || instructions.len() != 15 {
        return Err(eyre!("M5 PVM program is not the single fixture program"));
    }
    let mut expected_pc = 0u32;
    let expected = [
        (20, 7, 0, 0, 8),
        (20, 8, 0, 0, 8),
        (20, 9, 0, 0, 8),
        (20, 10, 0, 0, 8),
        (20, 11, 0, 0, 8),
        (20, 12, 0, 0, 8),
        (10, 0, 0, 0, 4),
        (56, 1, 0, 0, 4),
        (20, 2, 0, 0, 8),
        (211, 1, 1, 2, 0),
        (61, 1, 0, 0, 4),
        (20, 7, 0, 0, 8),
        (20, 8, 0, 0, 8),
        (10, 0, 0, 0, 4),
        (20, 7, 0, 0, 8),
        (20, 8, 0, 0, 8),
        (50, 0, 7, 0, 4),
    ];
    if instructions.len() != expected.len() {
        return Err(eyre!("M5 PVM instruction count mismatch"));
    }
    for (index, (instruction, &(opcode, rd, ra, rb, immediate_len))) in
        instructions.iter().zip(expected.iter()).enumerate()
    {
        if instruction.pc != expected_pc
            || instruction.opcode != opcode
            || instruction.registers.rd != rd
            || instruction.registers.ra != ra
            || instruction.registers.rb != rb
            || instruction.pc_delta != 0
            || instruction.immediate.len() != immediate_len
        {
            return Err(eyre!("M5 PVM instruction {index} is not canonical"));
        }
        expected_pc += match opcode {
            1 => 1,
            10 => 5,
            20 => 10,
            56 | 61 => 6,
            50 => 6,
            211 => 3,
            _ => return Err(eyre!("unsupported M5 fixture opcode {opcode}")),
        };
    }
    let immediate = |index: usize| instructions[index].immediate.as_slice();
    if u64::from_le_bytes(immediate(0).try_into().unwrap()) != 0x20000
        || u64::from_le_bytes(immediate(1).try_into().unwrap()) != 0
        || u64::from_le_bytes(immediate(2).try_into().unwrap()) != M5_SEGMENT_BYTES as u64
        || u64::from_le_bytes(immediate(3).try_into().unwrap()) != 6
        || u64::from_le_bytes(immediate(4).try_into().unwrap()) != 0
        || u64::from_le_bytes(immediate(5).try_into().unwrap()) != 0
        || u32::from_le_bytes(immediate(6).try_into().unwrap()) != 1
        || u32::from_le_bytes(immediate(7).try_into().unwrap()) != 0x20000
        || u64::from_le_bytes(immediate(8).try_into().unwrap()) != 0xA5A5_A5A5
        || u32::from_le_bytes(immediate(10).try_into().unwrap()) != 0x20000
        || u64::from_le_bytes(immediate(11).try_into().unwrap()) != 0x20000
        || u64::from_le_bytes(immediate(12).try_into().unwrap()) != M5_SEGMENT_BYTES as u64
        || u32::from_le_bytes(immediate(13).try_into().unwrap()) != 7
        || u32::from_le_bytes(immediate(14).try_into().unwrap()) != (-65_537i32) as u32
    {
        return Err(eyre!("M5 PVM fixture immediates do not match"));
    }
    if !matches!(
        program.blocks[0].terminator,
        zk_jam_refine_interface::PvmTerminatorV1::Halt
    ) {
        return Err(eyre!("M5 PVM fixture must halt after EXPORT"));
    }
    Ok(())
}

fn tamper_rejected(proof: &OpenVmProofArtifact, expected: &M5ProfileStatementV1, kind: u8) -> bool {
    let mut value = expected.clone();
    match kind {
        0 => value.case_commitment[0] ^= 1,
        1 => value.result_commitment[0] ^= 1,
        2 => value.exports_commitment[0] ^= 1,
        _ => value.profile_id[0] ^= 1,
    }
    verify_m5_statement(proof, &value).is_err()
}

fn verify_m5_statement(proof: &OpenVmProofArtifact, expected: &M5ProfileStatementV1) -> Result<()> {
    if proof.benchmark != M2Benchmark::M5ZkRefine {
        return Err(eyre!("proof is not an M5 ZkRefine proof"));
    }
    let actual = M5ProfileStatementV1::decode_openvm(&proof.public_output)
        .map_err(|error| eyre!("decode M5 public values: {error}"))?;
    if actual != *expected {
        return Err(eyre!(
            "M5 proof public values do not match expected bindings"
        ));
    }
    Ok(())
}

fn exports_root(exports: &[Vec<u8>]) -> [u8; 32] {
    if exports.is_empty() {
        return [0; 32];
    }
    assert_eq!(exports.len(), 1, "M5 fixture has exactly one export");
    let mut hasher = Keccak::v256();
    hasher.update(&exports[0]);
    let mut root = [0u8; 32];
    hasher.finalize(&mut root);
    root
}

fn context_hash(case: &[u8]) -> String {
    hex(&blake2_hash(
        &[M2Benchmark::M5ZkRefine.name().as_bytes(), case].concat(),
    ))
}

fn blake2_hash(bytes: &[u8]) -> [u8; 32] {
    Params::new()
        .hash_length(32)
        .hash(bytes)
        .as_bytes()
        .try_into()
        .unwrap()
}
fn decode_hex(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(eyre!("odd hex length"));
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|error| eyre!("invalid hex: {error}"))
        })
        .collect()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
