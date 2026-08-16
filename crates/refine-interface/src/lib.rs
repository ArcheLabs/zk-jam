//! Client-independent, versioned input and output types for the ZkRefine Profile v1.
//!
//! This crate deliberately contains no client or zkVM implementation types. A
//! client adapter owns the conversion from its internal representation into
//! these types. The canonical binary codec in [`codec`] is the interchange
//! format; JSON is only a debugging representation.

#![forbid(unsafe_code)]

pub mod case;
pub mod codec;
pub mod commitment;
pub mod program;
pub mod result;
pub mod state_witness;

pub use case::{RefineCaseV1, SegmentBytes, ZkRefineProfile, ZkRefineProfileV1};
pub use codec::{CanonicalCodec, CodecError};
pub use commitment::{
    input_commitments, statement_for, RefineInputCommitmentsV0, ZkRefineStatementV0,
};
pub use program::{
    PvmBlockV1, PvmInstructionV1, PvmProgramV1, PvmTerminatorV1, RegisterOperandsV1,
    PVM_PROGRAM_FORMAT_V1,
};
pub use result::{ReferenceRefineOutput, RefineResultV0};
pub use state_witness::{HistoricalLookupWitnessV1, RefineStateWitnessV1, StateWitnessBindingV1};
pub use zkrefine::{
    zkrefine_exports_canonical, zkrefine_exports_commitment, zkrefine_hash, zkrefine_profile_id,
    zkrefine_result_commitment, ZkRefineExportsV1, ZkRefineStatementV1, ZKREFINE_CASE_DOMAIN,
};

pub const REFINE_CASE_FORMAT_V1: u16 = 1;
pub const JAM_SEMANTICS_VERSION_0_7_2: u16 = 0x0702;
pub const PVM_REGISTER_COUNT: usize = 13;

mod zkrefine;
pub const SEGMENT_SIZE: usize = 4_104;

/// A host call or execution feature outside ZkRefine Profile v1.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum UnsupportedFeature {
    GasHostCall,
    InnerPvm,
    UnsupportedHostCall(u32),
}

impl std::fmt::Display for UnsupportedFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GasHostCall => {
                f.write_str("GAS host call is not supported by ZkRefine Profile v1")
            }
            Self::InnerPvm => {
                f.write_str("Inner PVM host calls are not supported by ZkRefine Profile v1")
            }
            Self::UnsupportedHostCall(id) => {
                write!(f, "host call {id} is not supported by ZkRefine Profile v1")
            }
        }
    }
}

impl std::error::Error for UnsupportedFeature {}

/// Check the fixed ZkRefine Profile v1 host-call admission policy.
pub fn check_host_call(id: u32) -> Result<(), UnsupportedFeature> {
    match id {
        0 => Err(UnsupportedFeature::GasHostCall),
        1 | 6 | 7 => Ok(()),
        8..=13 => Err(UnsupportedFeature::InnerPvm),
        _ => Err(UnsupportedFeature::UnsupportedHostCall(id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(opcode: u8, immediate: Vec<u8>) -> PvmProgramV1 {
        PvmProgramV1 {
            format_version: PVM_PROGRAM_FORMAT_V1,
            code_hash: [7; 32],
            o_blob: Vec::new(),
            w_blob: Vec::new(),
            z_pages: 1,
            s_bytes: 0,
            blocks: vec![PvmBlockV1 {
                entry_pc: 0,
                instructions: vec![PvmInstructionV1 {
                    pc: 0,
                    opcode,
                    registers: RegisterOperandsV1::default(),
                    immediate,
                    pc_delta: 0,
                }],
                terminator: PvmTerminatorV1::Halt,
            }],
            jump_table: Vec::new(),
            c_blob: Vec::new(),
        }
    }

    #[test]
    fn zkrefine_profile_is_fixed_to_no_gas_no_inner_pvm() {
        assert!(ZkRefineProfile::default().validate().is_ok());
        let profile = ZkRefineProfileV1 {
            gas_proof: true,
            ..ZkRefineProfileV1::default()
        };
        assert!(ZkRefineProfile::V1(profile).validate().is_err());
    }

    #[test]
    fn forbidden_host_calls_are_explicit() {
        assert_eq!(check_host_call(0), Err(UnsupportedFeature::GasHostCall));
        assert_eq!(check_host_call(8), Err(UnsupportedFeature::InnerPvm));
        assert_eq!(
            check_host_call(99),
            Err(UnsupportedFeature::UnsupportedHostCall(99))
        );
        assert!(check_host_call(1).is_ok());
    }

    #[test]
    fn program_roundtrips_deterministically() {
        let value = program(10, 7u32.to_le_bytes().to_vec());
        let encoded = value.encode_canonical();
        assert_eq!(encoded, value.encode_canonical());
        let decoded = PvmProgramV1::decode_canonical(&encoded).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn program_rejects_gas_and_inner_pvm() {
        assert!(program(10, 0u32.to_le_bytes().to_vec()).validate().is_err());
        assert!(program(10, 12u32.to_le_bytes().to_vec())
            .validate()
            .is_err());
    }

    #[test]
    fn refine_case_roundtrips_with_fixture_witness() {
        let case = RefineCaseV1 {
            format_version: REFINE_CASE_FORMAT_V1,
            profile: ZkRefineProfile::default(),
            core_index: 2,
            item_index: 0,
            work_package: vec![1, 2, 3],
            authorization_trace: vec![4],
            external_data: vec![vec![vec![5, 6]]],
            import_segments: vec![vec![vec![7, 8]]],
            export_offset: 0,
            program: program(1, Vec::new()),
            state_witness: RefineStateWitnessV1 {
                binding: StateWitnessBindingV1::Fixture,
                historical_lookups: Vec::new(),
            },
        };
        case.validate().unwrap();
        let encoded = case.encode_canonical();
        assert_eq!(RefineCaseV1::decode_canonical(&encoded).unwrap(), case);
    }
}
