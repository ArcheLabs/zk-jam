#![allow(clippy::needless_range_loop)]

use openvm::io::{read_vec, reveal_u32};
use sha2::{Digest, Sha256};

const SEGMENT_SIZE: usize = 4_104;
const VM_LOW_PROTECTED: usize = 1 << 16;
const BUFFER: usize = 2 << 16;
const XOR_MASK: u32 = 0xA5A5_A5A5;
const PROFILE_DOMAIN: &[u8] = b"zk-jam/zkrefine/profile/v1";
const CASE_DOMAIN: &[u8] = b"zk-jam/zkrefine/case/v1";
const RESULT_DOMAIN: &[u8] = b"zk-jam/zkrefine/result/v1";
const EXPORTS_DOMAIN: &[u8] = b"zk-jam/zkrefine/exports/v1";

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn take(&mut self, len: usize) -> &'a [u8] {
        let end = self.at.checked_add(len).expect("canonical length overflow");
        let value = self.bytes.get(self.at..end).expect("invalid RefineCase witness");
        self.at = end;
        value
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take(2).try_into().unwrap())
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }

    fn bytes(&mut self) -> &'a [u8] {
        let len = self.u32() as usize;
        self.take(len)
    }

    fn count(&mut self) -> usize {
        self.u32() as usize
    }
}

fn hash(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher.finalize().into()
}

fn profile_id() -> [u8; 32] {
    // The fixed ZkRefineProfile v1 default encoding: tag, version, JAM 0.7.2, gas=false,
    // inner_pvm=false. This is a profile description, not a Refine input or output.
    hash(PROFILE_DOMAIN, &[0, 0, 0, 2, 7, 0, 0])
}

fn first_import(case: &[u8]) -> Vec<u8> {
    let mut cursor = Cursor::new(case);
    assert_eq!(cursor.u16(), 1, "unsupported RefineCase version");
    assert_eq!(cursor.take(1), [0]);
    assert_eq!(cursor.u16(), 0);
    assert_eq!(cursor.u16(), 0x0702);
    assert_eq!(cursor.take(1), [0]);
    assert_eq!(cursor.take(1), [0]);
    assert_eq!(cursor.u16(), 0); // core index
    assert_eq!(cursor.u16(), 0); // item index
    let _work_package = cursor.bytes();
    let _authorization_trace = cursor.bytes();

    for _ in 0..cursor.count() {
        for _ in 0..cursor.count() {
            let _ = cursor.bytes();
        }
    }

    let item_count = cursor.count();
    assert_eq!(item_count, 1, "ZkRefine supports one item");
    let mut selected = Vec::new();
    for item in 0..item_count {
        let segment_count = cursor.count();
    assert_eq!(segment_count, 1, "ZkRefine supports one import");
        for segment in 0..segment_count {
            let value = cursor.bytes();
            if item == 0 && segment == 0 {
                selected.extend_from_slice(value);
            }
        }
    }
    let _export_offset = cursor.u32();
    selected
}

pub fn main() {
    // The witness is the complete canonical RefineCase. The commitment is computed before any
    // field is interpreted; FETCH therefore cannot be supplied from a host-side second input.
    let case = read_vec();
    let case_commitment = hash(CASE_DOMAIN, &case);
    let imported = first_import(&case);
    assert_eq!(imported.len(), SEGMENT_SIZE, "ZkRefine FETCH mode 6 segment length");

    // ECALLI FETCH(1), mode 6: import_segments[item_index][0], copied into protected memory.
    let mut memory = vec![0u8; BUFFER - VM_LOW_PROTECTED + SEGMENT_SIZE + 64];
    let buffer = BUFFER - VM_LOW_PROTECTED;
    memory[buffer..buffer + SEGMENT_SIZE].copy_from_slice(&imported);

    // The fixture's normalized PVM performs LOAD_U32, XOR and STORE_U32 on the fetched data.
    let mut word = [0u8; 4];
    word.copy_from_slice(&memory[buffer..buffer + 4]);
    let transformed = u32::from_le_bytes(word) ^ XOR_MASK;
    memory[buffer..buffer + 4].copy_from_slice(&transformed.to_le_bytes());

    // ECALLI EXPORT(7): export exactly one full segment from PVM memory.
    let exports = &memory[buffer..buffer + SEGMENT_SIZE];
    let mut exports_canonical = Vec::with_capacity(8 + SEGMENT_SIZE);
    exports_canonical.extend_from_slice(&1u32.to_le_bytes());
    exports_canonical.extend_from_slice(&(SEGMENT_SIZE as u32).to_le_bytes());
    exports_canonical.extend_from_slice(exports);
    let result_canonical = [0u8, 0, 0, 0, 0]; // RefineResultV0::Output(empty)
    let result_commitment = hash(RESULT_DOMAIN, &result_canonical);
    let exports_commitment = hash(EXPORTS_DOMAIN, &exports_canonical);

    let values = [profile_id(), case_commitment, result_commitment, exports_commitment];
    for (value_index, value) in values.iter().enumerate() {
        for (word_index, chunk) in value.chunks_exact(4).enumerate() {
            reveal_u32(
                u32::from_le_bytes(chunk.try_into().unwrap()),
                value_index * 8 + word_index,
            );
        }
    }
}
