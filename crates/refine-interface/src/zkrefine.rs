use sha2::{Digest, Sha256};

use crate::{CanonicalCodec, CodecError, RefineResultV0, ZkRefineProfile};

pub const ZKREFINE_CASE_DOMAIN: &[u8] = b"zk-jam/zkrefine/case/v1";
pub const ZKREFINE_RESULT_DOMAIN: &[u8] = b"zk-jam/zkrefine/result/v1";
pub const ZKREFINE_EXPORTS_DOMAIN: &[u8] = b"zk-jam/zkrefine/exports/v1";
pub const ZKREFINE_PROFILE_DOMAIN: &[u8] = b"zk-jam/zkrefine/profile/v1";

/// The four 32-byte values revealed by the ZkRefine guest, in public-value order.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ZkRefineStatementV1 {
    pub profile_id: [u8; 32],
    pub case_commitment: [u8; 32],
    pub result_commitment: [u8; 32],
    pub exports_commitment: [u8; 32],
}

impl ZkRefineStatementV1 {
    pub const OPENVM_LEN: usize = 128;

    pub fn encode_openvm(&self) -> [u8; Self::OPENVM_LEN] {
        let mut out = [0u8; Self::OPENVM_LEN];
        out[..32].copy_from_slice(&self.profile_id);
        out[32..64].copy_from_slice(&self.case_commitment);
        out[64..96].copy_from_slice(&self.result_commitment);
        out[96..128].copy_from_slice(&self.exports_commitment);
        out
    }

    pub fn decode_openvm(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() != Self::OPENVM_LEN {
            return Err(CodecError::InvalidValue("ZkRefine public values length"));
        }
        Ok(Self {
            profile_id: bytes[..32].try_into().unwrap(),
            case_commitment: bytes[32..64].try_into().unwrap(),
            result_commitment: bytes[64..96].try_into().unwrap(),
            exports_commitment: bytes[96..128].try_into().unwrap(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZkRefineExportsV1(pub Vec<Vec<u8>>);

impl CanonicalCodec for ZkRefineExportsV1 {
    fn encode_canonical(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.0.len() as u32).to_le_bytes());
        for segment in &self.0 {
            out.extend_from_slice(&(segment.len() as u32).to_le_bytes());
            out.extend_from_slice(segment);
        }
        out
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, CodecError> {
        let mut at = 0usize;
        let take = |at: &mut usize, n: usize| -> Result<&[u8], CodecError> {
            let end = at.checked_add(n).ok_or(CodecError::UnexpectedEof)?;
            let value = bytes.get(*at..end).ok_or(CodecError::UnexpectedEof)?;
            *at = end;
            Ok(value)
        };
        let count = u32::from_le_bytes(take(&mut at, 4)?.try_into().unwrap()) as usize;
        let mut segments = Vec::with_capacity(count);
        for _ in 0..count {
            let len = u32::from_le_bytes(take(&mut at, 4)?.try_into().unwrap()) as usize;
            segments.push(take(&mut at, len)?.to_vec());
        }
        if at != bytes.len() {
            return Err(CodecError::TrailingBytes);
        }
        Ok(Self(segments))
    }
}

pub fn zkrefine_exports_canonical(exports: &[Vec<u8>]) -> Vec<u8> {
    ZkRefineExportsV1(exports.to_vec()).encode_canonical()
}

pub fn zkrefine_hash(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher.finalize().into()
}

pub fn zkrefine_profile_id(profile: &ZkRefineProfile) -> [u8; 32] {
    zkrefine_hash(ZKREFINE_PROFILE_DOMAIN, &profile.encode_canonical())
}

pub fn zkrefine_result_commitment(result: &RefineResultV0) -> [u8; 32] {
    zkrefine_hash(ZKREFINE_RESULT_DOMAIN, &result.encode_canonical())
}

pub fn zkrefine_exports_commitment(exports: &[Vec<u8>]) -> [u8; 32] {
    zkrefine_hash(
        ZKREFINE_EXPORTS_DOMAIN,
        &zkrefine_exports_canonical(exports),
    )
}
