use crate::{case::RefineCaseV1, result::RefineResultV0, CanonicalCodec};
use blake2b_simd::Params;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefineInputCommitmentsV0 {
    pub work_package_hash: [u8; 32],
    pub program_hash: [u8; 32],
    pub authorization_trace_hash: [u8; 32],
    pub external_data_hash: [u8; 32],
    pub imports_hash: [u8; 32],
    pub state_witness_hash: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZkRefineStatementV0 {
    pub profile_id: [u8; 32],
    pub input_commitments: RefineInputCommitmentsV0,
    pub result: RefineResultV0,
    pub exports_root: [u8; 32],
}

pub fn hash_domain(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut params = Params::new();
    params.hash_length(32);
    let mut state = params.to_state();
    state.update(domain);
    state.update(bytes);
    state.finalize().as_bytes().try_into().unwrap()
}

/// M0 helper hashes are deterministic placeholders until the exact JAM 0.7.2
/// commitment mapping is frozen in `docs/refine-commitments.md`.
pub fn input_commitments(case: &RefineCaseV1) -> RefineInputCommitmentsV0 {
    RefineInputCommitmentsV0 {
        work_package_hash: hash_domain(b"zk-jam/m0/work-package", &case.work_package),
        program_hash: hash_domain(b"zk-jam/m0/program", &case.program.encode_canonical()),
        authorization_trace_hash: hash_domain(
            b"zk-jam/m0/authorization-trace",
            &case.authorization_trace,
        ),
        external_data_hash: hash_domain(
            b"zk-jam/m0/external-data",
            &encode_nested_bytes(&case.external_data),
        ),
        imports_hash: hash_domain(
            b"zk-jam/m0/imports",
            &encode_nested_segments(&case.import_segments),
        ),
        state_witness_hash: hash_domain(
            b"zk-jam/m0/state-witness",
            &case.state_witness.encode_canonical(),
        ),
    }
}

pub fn statement_for(
    case: &RefineCaseV1,
    output: &crate::ReferenceRefineOutput,
) -> ZkRefineStatementV0 {
    ZkRefineStatementV0 {
        profile_id: case.profile.id(),
        input_commitments: input_commitments(case),
        result: output.result.clone(),
        exports_root: hash_domain(b"zk-jam/m0/exports", &encode_segments(&output.exports)),
    }
}

fn encode_nested_bytes(values: &[Vec<Vec<u8>>]) -> Vec<u8> {
    let mut out = Vec::new();
    for outer in values {
        out.extend_from_slice(&(outer.len() as u32).to_le_bytes());
        for value in outer {
            out.extend_from_slice(&(value.len() as u32).to_le_bytes());
            out.extend_from_slice(value);
        }
    }
    out
}

fn encode_nested_segments(values: &[Vec<Vec<u8>>]) -> Vec<u8> {
    encode_nested_bytes(values)
}

fn encode_segments(values: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(values.len() as u32).to_le_bytes());
    for value in values {
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value);
    }
    out
}
