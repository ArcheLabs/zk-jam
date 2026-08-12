use crate::{case::SegmentBytes, codec::*, CanonicalCodec, CodecError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefineResultV0 {
    Output(Vec<u8>),
    Bad,
    Big,
    OutOfGas,
    Panic,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceRefineOutput {
    pub result: RefineResultV0,
    pub exports: Vec<SegmentBytes>,
}

impl CanonicalCodec for RefineResultV0 {
    fn encode_canonical(&self) -> Vec<u8> {
        encode_with(|w| {
            match self {
                Self::Output(value) => {
                    w.u8(0);
                    w.bytes(value)?;
                }
                Self::Bad => w.u8(1),
                Self::Big => w.u8(2),
                Self::OutOfGas => w.u8(3),
                Self::Panic => w.u8(4),
            }
            Ok(())
        })
        .expect("in-memory canonical encoding cannot fail")
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, CodecError> {
        decode_with(bytes, |r| {
            Ok(match r.u8()? {
                0 => Self::Output(r.bytes()?),
                1 => Self::Bad,
                2 => Self::Big,
                3 => Self::OutOfGas,
                4 => Self::Panic,
                _ => return Err(CodecError::InvalidValue("refine result tag")),
            })
        })
    }
}
