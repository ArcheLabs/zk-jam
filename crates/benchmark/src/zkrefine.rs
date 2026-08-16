use std::{fs, path::Path};

use blake2b_simd::Params;
use eyre::{eyre, Result, WrapErr};
use openvm_sdk::StdIn;
use serde::{Deserialize, Serialize};
use tiny_keccak::{Hasher, Keccak};
use zk_jam_openvm_backend::{M2Benchmark, OpenVmBackend, OpenVmProofArtifact};
use zk_jam_refine_interface::{
    zkrefine_exports_commitment, zkrefine_hash, zkrefine_profile_id, zkrefine_result_commitment,
    CanonicalCodec, PvmProgramV1, RefineCaseV1, RefineResultV0, ZkRefineStatementV1,
    ZKREFINE_CASE_DOMAIN,
};

pub const ZKREFINE_REPORT_SCHEMA: &str = "zk-jam/zkrefine-report/v1";
pub const ZKREFINE_PROFILE: &str = "zk-jam/zkrefine/import-export/v1";
pub const SEGMENT_BYTES: usize = 4_104;
const REFERENCE_REVISION: &str = "b850a458fa00da81e80be4cc84ddd7d2222f1edc";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WorkReportReference {
    schema: String,
    package_hash: String,
    result: String,
    exports_root: String,
    exports_count: u16,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ReferenceProvenance {
    schema: String,
    reference_client: String,
    reference_revision: String,
    jam_semantics: String,
    profile: String,
    work_items: usize,
    fetch_host_call: u32,
    fetch_modes: Vec<u32>,
    export_host_call: u32,
    segment_bytes: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZkRefineReportV1 {
    pub schema: String,
    pub verified: bool,
    pub profile: String,
    pub jam_semantics: String,
    pub execution: ExecutionSummary,
    pub reference: ReferenceValidation,
    pub proof: ProofValidation,
    pub work_report: WorkReportValidation,
    pub tamper: TamperValidation,
    pub capabilities: ZkRefineScope,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionSummary {
    pub work_items: usize,
    pub fetch_id: u32,
    pub fetch_modes: Vec<u32>,
    pub export_id: u32,
    pub segment_bytes: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReferenceValidation {
    pub client: String,
    pub revision: String,
    pub result_match: bool,
    pub exports_match: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProofValidation {
    pub verified: bool,
    pub serialized_reload_verified: bool,
    pub bytes: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkReportValidation {
    pub package_hash_match: bool,
    pub result_match: bool,
    pub exports_root_match: bool,
    pub exports_count_match: bool,
    pub gas_verified: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TamperValidation {
    pub case_rejected: bool,
    pub result_rejected: bool,
    pub exports_rejected: bool,
    pub statement_rejected: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZkRefineScope {
    pub gas_proof: bool,
    pub historical_lookup: bool,
    pub inner_pvm: bool,
    pub aggregation: bool,
}

#[derive(Serialize)]
struct StatementFile {
    schema: &'static str,
    profile_id: String,
    case_commitment: String,
    result_commitment: String,
    exports_commitment: String,
}

pub fn run_zkrefine(fixture: &Path, output: &Path) -> Result<ZkRefineReportV1> {
    fs::create_dir_all(output).wrap_err("create ZkRefine output directory")?;
    let case_bytes = fs::read(fixture.join("case.bin")).wrap_err("read reference case")?;
    let result_bytes = fs::read(fixture.join("result.bin")).wrap_err("read reference result")?;
    let exports_bytes = fs::read(fixture.join("exports.bin")).wrap_err("read reference exports")?;
    let work_report: WorkReportReference = read_json(&fixture.join("work-report.json"))?;
    let provenance: ReferenceProvenance = read_json(&fixture.join("provenance.json"))?;
    if provenance.schema != "zk-jam/reference-case/v1"
        || provenance.reference_client != "Jambda"
        || provenance.reference_revision != REFERENCE_REVISION
        || provenance.profile != ZKREFINE_PROFILE
        || provenance.jam_semantics != "0.7.2"
        || provenance.work_items != 1
        || provenance.fetch_host_call != 1
        || provenance.fetch_modes != [6]
        || provenance.export_host_call != 7
        || provenance.segment_bytes != SEGMENT_BYTES
    {
        return Err(eyre!("reference provenance is outside ZkRefine Profile v1"));
    }
    if work_report.schema != "zk-jam/work-report-reference/v1" {
        return Err(eyre!("unsupported reference WorkReport schema"));
    }
    let case = RefineCaseV1::decode_canonical(&case_bytes)?;
    validate_zkrefine_case(&case)?;
    if case.encode_canonical() != case_bytes {
        return Err(eyre!("reference case is not canonical"));
    }
    let result = RefineResultV0::decode_canonical(&result_bytes)?;
    if result.encode_canonical() != result_bytes {
        return Err(eyre!("reference result is not canonical"));
    }
    let exports = zk_jam_refine_interface::ZkRefineExportsV1::decode_canonical(&exports_bytes)?.0;
    if exports.len() != 1 || exports[0].len() != SEGMENT_BYTES {
        return Err(eyre!("reference exports are not one 4104-byte segment"));
    }
    let package_hash = hex(&blake2_hash(&case.work_package));
    let result_hex = hex(&result_bytes);
    let exports_root_hex = hex(&exports_root(&exports));
    let expected = ZkRefineStatementV1 {
        profile_id: zkrefine_profile_id(&case.profile),
        case_commitment: zkrefine_hash(ZKREFINE_CASE_DOMAIN, &case_bytes),
        result_commitment: zkrefine_result_commitment(&result),
        exports_commitment: zkrefine_exports_commitment(&exports),
    };
    let reference = ReferenceValidation {
        client: provenance.reference_client,
        revision: provenance.reference_revision,
        result_match: work_report.result == result_hex,
        exports_match: work_report.exports_root == exports_root_hex,
    };
    let work_report_validation = WorkReportValidation {
        package_hash_match: work_report.package_hash == package_hash,
        result_match: reference.result_match,
        exports_root_match: reference.exports_match,
        exports_count_match: work_report.exports_count == exports.len() as u16,
        gas_verified: false,
    };
    if !reference.result_match
        || !reference.exports_match
        || !work_report_validation.package_hash_match
    {
        return Err(eyre!("reference WorkReport does not bind the fixture"));
    }

    let backend = OpenVmBackend;
    let program = backend.program(M2Benchmark::ZkRefine)?;
    let mut stdin = StdIn::default();
    stdin.write(&case_bytes);
    let execution = backend.execute_stdin(&program, stdin.clone())?;
    let statement = ZkRefineStatementV1::decode_openvm(&execution.public_output)
        .map_err(|error| eyre!("decode ZkRefine execution statement: {error}"))?;
    if statement != expected {
        return Err(eyre!(
            "ZkRefine execution statement does not match reference"
        ));
    }
    let context_hash = hex(&blake2_hash(
        &[M2Benchmark::ZkRefine.name().as_bytes(), &case_bytes].concat(),
    ));
    let proof = backend.prove_stdin(&program, stdin, context_hash.clone())?;
    verify_statement(&proof, &expected)?;
    proof.verify(&context_hash)?;
    let proof_bytes = proof.to_bytes()?;
    let reloaded = OpenVmProofArtifact::from_bytes(&proof_bytes)?;
    reloaded.verify(&context_hash)?;
    verify_statement(&reloaded, &expected)?;
    let proof_validation = ProofValidation {
        verified: true,
        serialized_reload_verified: true,
        bytes: proof_bytes.len(),
    };
    let tamper = TamperValidation {
        case_rejected: tamper_rejected(&reloaded, &expected, 0),
        result_rejected: tamper_rejected(&reloaded, &expected, 1),
        exports_rejected: tamper_rejected(&reloaded, &expected, 2),
        statement_rejected: tamper_rejected(&reloaded, &expected, 3),
    };
    let verified = reference.result_match
        && reference.exports_match
        && proof_validation.verified
        && proof_validation.serialized_reload_verified
        && work_report_validation.package_hash_match
        && work_report_validation.result_match
        && work_report_validation.exports_root_match
        && work_report_validation.exports_count_match
        && tamper.case_rejected
        && tamper.result_rejected
        && tamper.exports_rejected
        && tamper.statement_rejected;
    if !verified {
        return Err(eyre!("ZkRefine acceptance failed closed"));
    }
    fs::write(output.join("case.bin"), &case_bytes)?;
    fs::write(output.join("result.bin"), &result_bytes)?;
    fs::write(output.join("exports.bin"), &exports_bytes)?;
    fs::write(output.join("proof.json"), &proof_bytes)?;
    fs::write(
        output.join("statement.json"),
        serde_json::to_vec_pretty(&StatementFile {
            schema: "zk-jam/zkrefine-statement/v1",
            profile_id: hex(&expected.profile_id),
            case_commitment: hex(&expected.case_commitment),
            result_commitment: hex(&expected.result_commitment),
            exports_commitment: hex(&expected.exports_commitment),
        })?,
    )?;
    let report = ZkRefineReportV1 {
        schema: ZKREFINE_REPORT_SCHEMA.to_string(),
        verified,
        profile: ZKREFINE_PROFILE.to_string(),
        jam_semantics: "0.7.2".to_string(),
        execution: ExecutionSummary {
            work_items: 1,
            fetch_id: 1,
            fetch_modes: vec![6],
            export_id: 7,
            segment_bytes: SEGMENT_BYTES,
        },
        reference,
        proof: proof_validation,
        work_report: work_report_validation,
        tamper,
        capabilities: ZkRefineScope {
            gas_proof: false,
            historical_lookup: false,
            inner_pvm: false,
            aggregation: false,
        },
    };
    fs::write(
        output.join("zkrefine-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}

fn validate_zkrefine_case(case: &RefineCaseV1) -> Result<()> {
    case.validate()?;
    if case.item_index != 0
        || case.core_index != 0
        || case.import_segments.len() != 1
        || case.import_segments[0].len() != 1
        || case.import_segments[0][0].len() != SEGMENT_BYTES
        || case.export_offset != 0
        || !case.state_witness.historical_lookups.is_empty()
    {
        return Err(eyre!(
            "reference case is outside the single-item mode-6 profile"
        ));
    }
    validate_zkrefine_program(&case.program)
}

fn validate_zkrefine_program(program: &PvmProgramV1) -> Result<()> {
    let instructions = program
        .blocks
        .iter()
        .flat_map(|block| block.instructions.iter())
        .collect::<Vec<_>>();
    let expected = [
        (20, 0, 7, 0, 8),
        (20, 0, 8, 0, 8),
        (20, 0, 9, 0, 8),
        (20, 0, 10, 0, 8),
        (20, 0, 11, 0, 8),
        (20, 0, 12, 0, 8),
        (10, 0, 0, 0, 4),
        (56, 0, 1, 0, 4),
        (20, 0, 2, 0, 8),
        (211, 1, 1, 2, 0),
        (61, 0, 1, 0, 4),
        (20, 0, 7, 0, 8),
        (20, 0, 8, 0, 8),
        (20, 0, 7, 0, 8),
        (20, 0, 8, 0, 8),
        (10, 0, 0, 0, 4),
        (50, 0, 7, 0, 4),
    ];
    if program.blocks.len() != 1 || instructions.len() != expected.len() {
        return Err(eyre!(
            "ZkRefine PVM program instruction count mismatch: actual={} expected={} blocks={}",
            instructions.len(),
            expected.len(),
            program.blocks.len()
        ));
    }
    let mut pc = 0u32;
    for (index, (instruction, &(opcode, rd, ra, rb, immediate_len))) in
        instructions.iter().zip(expected.iter()).enumerate()
    {
        if instruction.pc != pc
            || instruction.opcode != opcode
            || instruction.registers.rd != rd
            || instruction.registers.ra != ra
            || instruction.registers.rb != rb
            || instruction.pc_delta != 0
            || instruction.immediate.len() != immediate_len
        {
            return Err(eyre!(
                "ZkRefine PVM instruction {index} mismatch: actual pc={} opcode={} rd={} ra={} rb={} pc_delta={} immediate_len={}; expected pc={} opcode={} rd={} ra={} rb={} pc_delta=0 immediate_len={}",
                instruction.pc,
                instruction.opcode,
                instruction.registers.rd,
                instruction.registers.ra,
                instruction.registers.rb,
                instruction.pc_delta,
                instruction.immediate.len(),
                pc,
                opcode,
                rd,
                ra,
                rb,
                immediate_len
            ));
        }
        pc += match opcode {
            10 => 5,
            20 => 10,
            50 | 56 | 61 => 6,
            211 => 3,
            _ => return Err(eyre!("unsupported ZkRefine opcode {opcode}")),
        };
    }
    let expected_immediates = [
        0x20000u64.to_le_bytes().to_vec(),
        0u64.to_le_bytes().to_vec(),
        (SEGMENT_BYTES as u64).to_le_bytes().to_vec(),
        6u64.to_le_bytes().to_vec(),
        0u64.to_le_bytes().to_vec(),
        0u64.to_le_bytes().to_vec(),
        1u32.to_le_bytes().to_vec(),
        0x20000u32.to_le_bytes().to_vec(),
        0xA5A5_A5A5u64.to_le_bytes().to_vec(),
        Vec::new(),
        0x20000u32.to_le_bytes().to_vec(),
        0x20000u64.to_le_bytes().to_vec(),
        (SEGMENT_BYTES as u64).to_le_bytes().to_vec(),
        0x20000u64.to_le_bytes().to_vec(),
        (SEGMENT_BYTES as u64).to_le_bytes().to_vec(),
        7u32.to_le_bytes().to_vec(),
        (-65_537i32).to_le_bytes().to_vec(),
    ];
    for (index, (instruction, expected_immediate)) in instructions
        .iter()
        .zip(expected_immediates.iter())
        .enumerate()
    {
        if instruction.immediate != *expected_immediate {
            return Err(eyre!(
                "ZkRefine PVM instruction {index} immediate mismatch: actual_len={} actual={:02x?} expected_len={} expected={:02x?}",
                instruction.immediate.len(),
                instruction.immediate,
                expected_immediate.len(),
                expected_immediate
            ));
        }
    }
    if !matches!(
        program.blocks[0].terminator,
        zk_jam_refine_interface::PvmTerminatorV1::Halt
    ) {
        return Err(eyre!("ZkRefine PVM fixture must halt after EXPORT"));
    }
    Ok(())
}

fn tamper_rejected(proof: &OpenVmProofArtifact, expected: &ZkRefineStatementV1, kind: u8) -> bool {
    let mut value = expected.clone();
    match kind {
        0 => value.case_commitment[0] ^= 1,
        1 => value.result_commitment[0] ^= 1,
        2 => value.exports_commitment[0] ^= 1,
        _ => value.profile_id[0] ^= 1,
    };
    verify_statement(proof, &value).is_err()
}
fn verify_statement(proof: &OpenVmProofArtifact, expected: &ZkRefineStatementV1) -> Result<()> {
    if proof.benchmark != M2Benchmark::ZkRefine {
        return Err(eyre!("proof is not a ZkRefine proof"));
    }
    let actual = ZkRefineStatementV1::decode_openvm(&proof.public_output)?;
    if actual != *expected {
        return Err(eyre!("ZkRefine proof public values do not match"));
    }
    Ok(())
}
fn exports_root(exports: &[Vec<u8>]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(exports.first().map_or(&[], Vec::as_slice));
    let mut root = [0; 32];
    hasher.finalize(&mut root);
    root
}
fn blake2_hash(bytes: &[u8]) -> [u8; 32] {
    Params::new()
        .hash_length(32)
        .hash(bytes)
        .as_bytes()
        .try_into()
        .unwrap()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_reference_case_is_canonical_profile_v1() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/refine-import-export-v1/case.bin");
        let bytes = fs::read(path).expect("checked-in reference case");
        let case = RefineCaseV1::decode_canonical(&bytes).expect("canonical reference case");
        validate_zkrefine_case(&case).expect("profile v1 reference case");
        assert_eq!(case.program.instruction_count(), 17);
    }
}
