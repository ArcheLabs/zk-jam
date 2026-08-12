use crate::codec::*;
use crate::{CanonicalCodec, CodecError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricalLookupWitnessV1 {
    pub service_id: u32,
    pub hash: [u8; 32],
    pub value: Vec<u8>,
    pub commitment_proof: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateWitnessBindingV1 {
    Fixture,
    Committed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefineStateWitnessV1 {
    pub binding: StateWitnessBindingV1,
    pub historical_lookups: Vec<HistoricalLookupWitnessV1>,
}

impl CanonicalCodec for RefineStateWitnessV1 {
    fn encode_canonical(&self) -> Vec<u8> {
        encode_with(|w| {
            w.u8(match self.binding {
                StateWitnessBindingV1::Fixture => 0,
                StateWitnessBindingV1::Committed => 1,
            });
            w.count(self.historical_lookups.len())?;
            for lookup in &self.historical_lookups {
                w.u32(lookup.service_id);
                w.fixed(&lookup.hash);
                w.bytes(&lookup.value)?;
                w.bytes(&lookup.commitment_proof)?;
            }
            Ok(())
        })
        .expect("in-memory canonical encoding cannot fail")
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, CodecError> {
        decode_with(bytes, |r| {
            let binding = match r.u8()? {
                0 => StateWitnessBindingV1::Fixture,
                1 => StateWitnessBindingV1::Committed,
                _ => return Err(CodecError::InvalidValue("state witness binding")),
            };
            let historical_lookups = r.vec(|r| {
                Ok(HistoricalLookupWitnessV1 {
                    service_id: r.u32()?,
                    hash: r.fixed()?,
                    value: r.bytes()?,
                    commitment_proof: r.bytes()?,
                })
            })?;
            Ok(Self {
                binding,
                historical_lookups,
            })
        })
    }
}
