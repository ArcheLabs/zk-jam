#![feature(generic_const_exprs)]
#![allow(incomplete_features)]

use std::{collections::BTreeMap, env, fs, path::PathBuf, sync::Arc};

use blake2b_simd::Params;
use jam_codec::Encode;
use jambda_state_backend::StateBackend;
use jp_core_primitives::{
    crypto::OpaqueHash,
    error::DataBaseError,
    simple::{ByteSequence, TimeSlot},
    spec::TinySpec,
    state::{ColumnFamily, StoreKey},
    traits::{DataBase, JamHash},
    types::ServiceInfo,
    work::{ImportSpec, PreimagesLookups, RefineContext, WorkItem, WorkPackage},
};
use jp_vm_engine::InnerEngine;
use jp_vm_interp::InterpBackend;
use serde::Serialize;
use zk_jam_refine_interface::{
    CanonicalCodec, PvmBlockV1, PvmInstructionV1, PvmProgramV1, PvmTerminatorV1, RefineCaseV1,
    RefineResultV0, RefineStateWitnessV1, RegisterOperandsV1, SmokeProfile, StateWitnessBindingV1,
    PVM_PROGRAM_FORMAT_V1, REFINE_CASE_FORMAT_V1,
};

const SEGMENT_SIZE: usize = 4_104;
const SERVICE_ID: u32 = 1;
// Jambda's Refine interpreter reserves 0x10000.. for the read-only O region;
// the first writable W page therefore starts at 0x20000.
const O: u64 = 2 << 16;
const XOR_MASK: u32 = 0xA5A5_A5A5;

#[derive(Serialize)]
struct Output {
    schema: String,
    jambda_repository: String,
    jambda_revision: String,
    case_hex: String,
    reference_result_hex: String,
    reference_exports_hex: Vec<String>,
    work_package_hash_hex: String,
    work_report_package_hash_hex: String,
    work_report_exports_root_hex: String,
    work_report_exports_count: u16,
    work_report_result_hex: String,
}

#[derive(Default)]
struct Db {
    values: BTreeMap<(ColumnFamily, Vec<u8>), Vec<u8>>,
}

impl Db {
    fn put_key(&mut self, key: StoreKey, bytes: Vec<u8>) {
        self.values.insert((key.col(), key.to_db_key()), bytes);
    }
}

impl DataBase for Db {
    fn key_may_exist<K: AsRef<[u8]>>(&self, col: ColumnFamily, key: &K) -> bool {
        self.values.contains_key(&(col, key.as_ref().to_vec()))
    }
    fn get<K: AsRef<[u8]>>(
        &self,
        col: ColumnFamily,
        key: &K,
    ) -> Result<Option<Vec<u8>>, DataBaseError> {
        Ok(self.values.get(&(col, key.as_ref().to_vec())).cloned())
    }
    fn del<K: AsRef<[u8]>>(&self, _col: ColumnFamily, _key: &K) -> Result<(), DataBaseError> {
        Ok(())
    }
    fn multi_get<K: AsRef<[u8]>>(
        &self,
        keys: &[K],
        col: ColumnFamily,
    ) -> Result<Vec<Option<Vec<u8>>>, DataBaseError> {
        Ok(keys
            .iter()
            .map(|key| self.values.get(&(col, key.as_ref().to_vec())).cloned())
            .collect())
    }
    fn put<K: AsRef<[u8]>>(
        &self,
        _col: ColumnFamily,
        _key: &K,
        _value: Box<[u8]>,
    ) -> Result<(), DataBaseError> {
        Ok(())
    }
    fn batch_write(
        &self,
        _put_entries: &[jp_core_primitives::state::StoreChange],
    ) -> Result<(), DataBaseError> {
        Ok(())
    }
    fn batch_write_cf<K: AsRef<[u8]>>(
        &self,
        _col: ColumnFamily,
        _put_entries: &[(K, Vec<u8>)],
    ) -> Result<(), DataBaseError> {
        Ok(())
    }
    fn multi_seek_for_prev<F>(
        &self,
        _col: ColumnFamily,
        keys: &[&jp_core_primitives::state::StateKey],
        mut callback: F,
    ) -> Result<(), DataBaseError>
    where
        F: FnMut(usize, Option<(&[u8], &[u8])>),
    {
        for (index, _) in keys.iter().enumerate() {
            callback(index, None);
        }
        Ok(())
    }
}

fn fnencode(value: usize, out: &mut Vec<u8>) {
    if value == 0 {
        out.push(0);
    } else if value < 0x80 {
        out.push(value as u8);
    } else {
        let value = value as u64;
        for length in 1..=7usize {
            let high_bits = value >> (8 * length);
            if high_bits < (1u64 << (7 - length)) {
                let prefix = (0xFFu16 << (8 - length)) as u8;
                out.push(prefix | high_bits as u8);
                out.extend_from_slice(&value.to_le_bytes()[..length]);
                return;
            }
        }
        out.push(0xFF);
        out.extend_from_slice(&value.to_le_bytes());
    }
}

fn instruction_bytes() -> (Vec<u8>, Vec<PvmInstructionV1>) {
    let mut code = Vec::new();
    let mut instructions = Vec::new();
    let mut push =
        |opcode: u8, registers: RegisterOperandsV1, immediate: Vec<u8>, encoded: Vec<u8>| {
            let pc = code.len() as u32;
            code.extend_from_slice(&encoded);
            instructions.push(PvmInstructionV1 {
                pc,
                opcode,
                registers,
                immediate,
                pc_delta: 0,
            });
        };
    let load = |reg: u8, value: u64| {
        let mut bytes = vec![20, reg];
        bytes.extend_from_slice(&value.to_le_bytes());
        bytes
    };
    push(
        20,
        RegisterOperandsV1 {
            rd: 0,
            ra: 7,
            rb: 0,
        },
        O.to_le_bytes().to_vec(),
        load(7, O),
    );
    push(
        20,
        RegisterOperandsV1 {
            rd: 0,
            ra: 8,
            rb: 0,
        },
        0u64.to_le_bytes().to_vec(),
        load(8, 0),
    );
    push(
        20,
        RegisterOperandsV1 {
            rd: 0,
            ra: 9,
            rb: 0,
        },
        (SEGMENT_SIZE as u64).to_le_bytes().to_vec(),
        load(9, SEGMENT_SIZE as u64),
    );
    push(
        20,
        RegisterOperandsV1 {
            rd: 0,
            ra: 10,
            rb: 0,
        },
        6u64.to_le_bytes().to_vec(),
        load(10, 6),
    );
    push(
        20,
        RegisterOperandsV1 {
            rd: 0,
            ra: 11,
            rb: 0,
        },
        0u64.to_le_bytes().to_vec(),
        load(11, 0),
    );
    push(
        20,
        RegisterOperandsV1 {
            rd: 0,
            ra: 12,
            rb: 0,
        },
        0u64.to_le_bytes().to_vec(),
        load(12, 0),
    );
    push(
        10,
        RegisterOperandsV1::default(),
        1u32.to_le_bytes().to_vec(),
        [vec![10], 1u32.to_le_bytes().to_vec()].concat(),
    );
    push(
        56,
        RegisterOperandsV1 {
            rd: 0,
            ra: 1,
            rb: 0,
        },
        (O as u32).to_le_bytes().to_vec(),
        [vec![56, 1], (O as u32).to_le_bytes().to_vec()].concat(),
    );
    push(
        20,
        RegisterOperandsV1 {
            rd: 0,
            ra: 2,
            rb: 0,
        },
        (XOR_MASK as u64).to_le_bytes().to_vec(),
        load(2, XOR_MASK as u64),
    );
    push(
        211,
        RegisterOperandsV1 {
            rd: 1,
            ra: 1,
            rb: 2,
        },
        Vec::new(),
        vec![211, 0x21, 1],
    );
    push(
        61,
        RegisterOperandsV1 {
            rd: 0,
            ra: 1,
            rb: 0,
        },
        (O as u32).to_le_bytes().to_vec(),
        [vec![61, 1], (O as u32).to_le_bytes().to_vec()].concat(),
    );
    push(
        20,
        RegisterOperandsV1 {
            rd: 0,
            ra: 7,
            rb: 0,
        },
        O.to_le_bytes().to_vec(),
        load(7, O),
    );
    push(
        20,
        RegisterOperandsV1 {
            rd: 0,
            ra: 8,
            rb: 0,
        },
        (SEGMENT_SIZE as u64).to_le_bytes().to_vec(),
        load(8, SEGMENT_SIZE as u64),
    );
    push(
        10,
        RegisterOperandsV1::default(),
        7u32.to_le_bytes().to_vec(),
        [vec![10], 7u32.to_le_bytes().to_vec()].concat(),
    );
    // EXPORT returns its count in r7. Jump from that reply value to Jambda's
    // magic halt address, which produces the canonical empty Refine result.
    push(
        50,
        RegisterOperandsV1 {
            rd: 0,
            ra: 7,
            rb: 0,
        },
        (-65_537i32).to_le_bytes().to_vec(),
        [vec![50, 7], (-65_537i32).to_le_bytes().to_vec()].concat(),
    );
    (code, instructions)
}

fn code_blob(code: &[u8]) -> Vec<u8> {
    let mut c_blob = Vec::new();
    fnencode(0, &mut c_blob);
    c_blob.push(1);
    fnencode(code.len(), &mut c_blob);
    c_blob.extend_from_slice(code);
    let mut valid = vec![0u8; code.len().div_ceil(8)];
    let (_, instructions) = instruction_bytes();
    for instruction in instructions {
        let pc = instruction.pc as usize;
        valid[pc / 8] |= 1 << (pc % 8);
    }
    c_blob.extend_from_slice(&valid);
    let mut image = Vec::new();
    image.extend_from_slice(&0u32.to_le_bytes()[..3]);
    image.extend_from_slice(&0u32.to_le_bytes()[..3]);
    image.extend_from_slice(&2u16.to_le_bytes());
    image.extend_from_slice(&0u32.to_le_bytes()[..3]);
    image.extend_from_slice(&(c_blob.len() as u32).to_le_bytes());
    image.extend_from_slice(&c_blob);
    let mut outer = Vec::new();
    // PVM package encoding is E(|m|) || m || guest-image; this fixture has no
    // authorization metadata, so the image follows a zero-length metadata field.
    fnencode(0, &mut outer);
    outer.extend_from_slice(&image);
    outer
}

fn build() -> Result<(RefineCaseV1, Output), Box<dyn std::error::Error>>
where
    [(); 2]:,
    [(); 1]:,
    [(); 6]:,
    [(); 12]:,
    [(); 24]:,
    [(); 10]:,
{
    let (mut code, instructions) = instruction_bytes();
    // Keep one invalid padding byte after the terminal instruction. The pinned
    // predecoder's bounded validity scan otherwise shifts by eight when the
    // final valid PC lands at the end of a K byte.
    code.push(0);
    let blob = code_blob(&code);
    jp_vm_predecode::to_af_and_c_blob(&blob).map_err(|error| {
        format!(
            "M5 fixture code image failed Jambda predecode: {error:?} (len={}, prefix={:02x?})",
            blob.len(),
            &blob[..blob.len().min(24)]
        )
    })?;
    let code_hash = OpaqueHash(
        Params::new()
            .hash_length(32)
            .hash(&blob)
            .as_bytes()
            .try_into()
            .unwrap(),
    );
    let package = WorkPackage {
        auth_code_host: 0,
        auth_code_hash: OpaqueHash([0; 32]),
        context: RefineContext {
            anchor: OpaqueHash([0; 32]),
            state_root: OpaqueHash([0; 32]),
            beefy_root: OpaqueHash([0; 32]),
            lookup_anchor: OpaqueHash([0; 32]),
            lookup_anchor_slot: TimeSlot(0),
            prerequisites: Vec::new(),
        },
        authorization: ByteSequence::from(Vec::new()),
        authorizer_config: ByteSequence::from(Vec::new()),
        items: vec![WorkItem {
            service: SERVICE_ID,
            code_hash,
            refine_gas_limit: 10_000_000,
            accumulate_gas_limit: 10_000_000,
            export_count: 1,
            payload: ByteSequence::from(Vec::new()),
            import_segments: vec![ImportSpec {
                tree_root: OpaqueHash([0; 32]),
                index: 0,
            }],
            extrinsic: Vec::new(),
        }],
    };
    let imported: Vec<u8> = (0..SEGMENT_SIZE).map(|i| (i % 256) as u8).collect();
    let input = jambda_refine::WorkReportInput {
        core_index: 0,
        work_package: Arc::new(package.clone()),
        external_data: Arc::new(vec![Vec::new()]),
        import_segments: Arc::new(vec![vec![imported.clone().try_into().unwrap()]]),
        import_proofs: jambda_refine::ImportProofBundle::default(),
    };
    let mut db = Db::default();
    let info = ServiceInfo::new(code_hash, 1, 1, 0, TimeSlot(0), 0, blob.len() as u64);
    db.put_key(StoreKey::ServiceInfo(SERVICE_ID), info.encode());
    db.put_key(
        StoreKey::Preimage(jp_core_primitives::state::PreimageKey {
            service_id: SERVICE_ID,
            hash: code_hash,
        }),
        blob.clone(),
    );
    db.put_key(
        StoreKey::new_service_lookups_key(&SERVICE_ID, &code_hash, blob.len() as u32),
        PreimagesLookups::new(vec![TimeSlot(0)]).encode(),
    );
    let mut backend = StateBackend::<TinySpec, _>::new_tiny(db);
    backend.load_tiny_from_db()?;
    let result = jambda_refine::compute_work_report::<
        TinySpec,
        Db,
        StateBackend<TinySpec, Db>,
        InterpBackend,
        InnerEngine<InterpBackend>,
    >(&backend, input, InterpBackend::default())?;
    let work_result = &result.report.results[0].result;
    let output_bytes = match work_result {
        jp_core_primitives::work::WorkExecResult::Ok(value) => value.as_slice().to_vec(),
        other => return Err(format!("Jambda returned {other:?}").into()),
    };
    let exports = result
        .exported_segments
        .iter()
        .map(|segment| segment.to_vec())
        .collect::<Vec<_>>();
    let case = RefineCaseV1 {
        format_version: REFINE_CASE_FORMAT_V1,
        profile: SmokeProfile::default(),
        core_index: 0,
        item_index: 0,
        work_package: package.encode(),
        authorization_trace: Vec::new(),
        external_data: vec![Vec::new()],
        import_segments: vec![vec![imported]],
        export_offset: 0,
        program: PvmProgramV1 {
            format_version: PVM_PROGRAM_FORMAT_V1,
            code_hash: code_hash.0,
            o_blob: Vec::new(),
            w_blob: Vec::new(),
            z_pages: 2,
            s_bytes: 0,
            blocks: vec![PvmBlockV1 {
                entry_pc: 0,
                instructions,
                terminator: PvmTerminatorV1::Halt,
            }],
            jump_table: Vec::new(),
            c_blob: Vec::new(),
        },
        state_witness: RefineStateWitnessV1 {
            binding: StateWitnessBindingV1::Fixture,
            historical_lookups: Vec::new(),
        },
    };
    let result_value = RefineResultV0::Output(output_bytes);
    let case_hex = hex(&case.encode_canonical());
    Ok((
        case,
        Output {
            schema: "zk-jam/m5/jambda-adapter/v1".into(),
            jambda_repository: "ArcheLabs/jambda".into(),
            jambda_revision: env::var("ZK_JAM_JAMBDA_REVISION")
                .unwrap_or_else(|_| "unknown".into()),
            case_hex,
            reference_result_hex: hex(&result_value.encode_canonical()),
            reference_exports_hex: exports.iter().map(|v| hex(v)).collect(),
            work_package_hash_hex: hex(&package.jam_hash().0),
            work_report_package_hash_hex: hex(&result.report.package_spec.hash.0),
            work_report_exports_root_hex: hex(&result.report.package_spec.exports_root.0),
            work_report_exports_count: result.report.package_spec.exports_count,
            work_report_result_hex: hex(&result_value.encode_canonical()),
        },
    ))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = env::args().collect::<Vec<_>>();
    let jambda_repo = args
        .windows(2)
        .find(|pair| pair[0] == "--jambda-repo")
        .map(|pair| PathBuf::from(&pair[1]))
        .ok_or("missing --jambda-repo")?;
    if !jambda_repo.is_dir() {
        return Err(format!("Jambda checkout does not exist: {}", jambda_repo.display()).into());
    }
    let output = args
        .windows(2)
        .find(|pair| pair[0] == "--output")
        .map(|pair| PathBuf::from(&pair[1]))
        .ok_or("missing --output")?;
    fs::create_dir_all(&output)?;
    let (_, report) = build()?;
    fs::write(
        output.join("m5-jambda.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
