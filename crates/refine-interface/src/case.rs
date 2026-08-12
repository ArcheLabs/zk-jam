use crate::{
    codec::*, program::PvmProgramV1, state_witness::RefineStateWitnessV1, CanonicalCodec,
    CodecError, JAM_SEMANTICS_VERSION_0_7_2, REFINE_CASE_FORMAT_V1,
};
use serde::{Deserialize, Serialize};

pub type SegmentBytes = Vec<u8>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmokeProfileV0 {
    pub version: u16,
    pub jam_semantics: u16,
    pub gas_proof: bool,
    pub inner_pvm: bool,
}

impl Default for SmokeProfileV0 {
    fn default() -> Self {
        Self {
            version: 0,
            jam_semantics: JAM_SEMANTICS_VERSION_0_7_2,
            gas_proof: false,
            inner_pvm: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SmokeProfile {
    V0(SmokeProfileV0),
}

impl Default for SmokeProfile {
    fn default() -> Self {
        Self::V0(SmokeProfileV0::default())
    }
}

impl SmokeProfile {
    pub fn id(&self) -> [u8; 32] {
        crate::commitment::hash_domain(b"zk-jam/profile-v0", &self.encode_canonical())
    }

    pub fn validate(&self) -> Result<(), CodecError> {
        match self {
            Self::V0(profile)
                if profile.version == 0
                    && profile.jam_semantics == JAM_SEMANTICS_VERSION_0_7_2
                    && !profile.gas_proof
                    && !profile.inner_pvm =>
            {
                Ok(())
            }
            Self::V0(_) => Err(CodecError::InvalidValue("invalid SmokeProfileV0")),
        }
    }
}

impl CanonicalCodec for SmokeProfile {
    fn encode_canonical(&self) -> Vec<u8> {
        encode_with(|w| {
            match self {
                SmokeProfile::V0(profile) => {
                    w.u8(0);
                    w.u16(profile.version);
                    w.u16(profile.jam_semantics);
                    w.u8(profile.gas_proof as u8);
                    w.u8(profile.inner_pvm as u8);
                }
            }
            Ok(())
        })
        .expect("in-memory canonical encoding cannot fail")
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, CodecError> {
        decode_with(bytes, |r| match r.u8()? {
            0 => Ok(SmokeProfile::V0(SmokeProfileV0 {
                version: r.u16()?,
                jam_semantics: r.u16()?,
                gas_proof: r.u8()? != 0,
                inner_pvm: r.u8()? != 0,
            })),
            _ => Err(CodecError::InvalidValue("SmokeProfile tag")),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefineCaseV1 {
    pub format_version: u16,
    pub profile: SmokeProfile,
    pub core_index: u16,
    pub item_index: u16,
    pub work_package: Vec<u8>,
    pub authorization_trace: Vec<u8>,
    pub external_data: Vec<Vec<Vec<u8>>>,
    pub import_segments: Vec<Vec<SegmentBytes>>,
    pub export_offset: u32,
    pub program: PvmProgramV1,
    pub state_witness: RefineStateWitnessV1,
}

impl RefineCaseV1 {
    pub fn validate(&self) -> Result<(), CodecError> {
        if self.format_version != REFINE_CASE_FORMAT_V1 {
            return Err(CodecError::InvalidValue("unsupported RefineCaseV1 version"));
        }
        self.profile.validate()?;
        self.program.validate()?;
        for segment in self.import_segments.iter().flatten() {
            if segment.len() > crate::SEGMENT_SIZE {
                return Err(CodecError::InvalidValue(
                    "import segment exceeds segment size",
                ));
            }
        }
        Ok(())
    }

    pub fn debug_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

impl CanonicalCodec for RefineCaseV1 {
    fn encode_canonical(&self) -> Vec<u8> {
        encode_with(|w| {
            w.u16(self.format_version);
            match &self.profile {
                SmokeProfile::V0(profile) => {
                    w.u8(0);
                    w.u16(profile.version);
                    w.u16(profile.jam_semantics);
                    w.u8(profile.gas_proof as u8);
                    w.u8(profile.inner_pvm as u8);
                }
            }
            w.u16(self.core_index);
            w.u16(self.item_index);
            w.bytes(&self.work_package)?;
            w.bytes(&self.authorization_trace)?;
            encode_nested_bytes(w, &self.external_data)?;
            encode_nested_segments(w, &self.import_segments)?;
            w.u32(self.export_offset);
            w.bytes(&self.program.encode_canonical())?;
            w.bytes(&self.state_witness.encode_canonical())
        })
        .expect("in-memory canonical encoding cannot fail")
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, CodecError> {
        decode_with(bytes, |r| {
            let format_version = r.u16()?;
            let profile = match r.u8()? {
                0 => SmokeProfile::V0(SmokeProfileV0 {
                    version: r.u16()?,
                    jam_semantics: r.u16()?,
                    gas_proof: r.u8()? != 0,
                    inner_pvm: r.u8()? != 0,
                }),
                _ => return Err(CodecError::InvalidValue("SmokeProfile tag")),
            };
            let core_index = r.u16()?;
            let item_index = r.u16()?;
            let work_package = r.bytes()?;
            let authorization_trace = r.bytes()?;
            let external_data = decode_nested_bytes(r)?;
            let import_segments = decode_nested_segments(r)?;
            let export_offset = r.u32()?;
            let program = PvmProgramV1::decode_canonical(&r.bytes()?)?;
            let state_witness = RefineStateWitnessV1::decode_canonical(&r.bytes()?)?;
            Ok(Self {
                format_version,
                profile,
                core_index,
                item_index,
                work_package,
                authorization_trace,
                external_data,
                import_segments,
                export_offset,
                program,
                state_witness,
            })
        })
    }
}

fn encode_nested_bytes(w: &mut Writer, values: &[Vec<Vec<u8>>]) -> Result<(), CodecError> {
    w.count(values.len())?;
    for outer in values {
        w.count(outer.len())?;
        for value in outer {
            w.bytes(value)?;
        }
    }
    Ok(())
}

fn encode_nested_segments(w: &mut Writer, values: &[Vec<SegmentBytes>]) -> Result<(), CodecError> {
    w.count(values.len())?;
    for outer in values {
        w.count(outer.len())?;
        for value in outer {
            w.bytes(value)?;
        }
    }
    Ok(())
}

fn decode_nested_bytes(r: &mut Reader<'_>) -> Result<Vec<Vec<Vec<u8>>>, CodecError> {
    r.vec(|r| r.vec(|r| r.bytes()))
}

fn decode_nested_segments(r: &mut Reader<'_>) -> Result<Vec<Vec<SegmentBytes>>, CodecError> {
    r.vec(|r| r.vec(|r| r.bytes()))
}
