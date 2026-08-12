use crate::{check_host_call, codec::*, CodecError, UnsupportedFeature, PVM_REGISTER_COUNT};
use serde::{Deserialize, Serialize};

pub const PVM_PROGRAM_FORMAT_V1: u16 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterOperandsV1 {
    pub rd: u8,
    pub ra: u8,
    pub rb: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PvmInstructionV1 {
    pub pc: u32,
    pub opcode: u8,
    pub registers: RegisterOperandsV1,
    pub immediate: Vec<u8>,
    pub pc_delta: u8,
}

impl PvmInstructionV1 {
    pub fn host_call_id(&self) -> Option<u32> {
        (self.opcode == 10 && self.immediate.len() >= 4)
            .then(|| u32::from_le_bytes(self.immediate[..4].try_into().expect("checked length")))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PvmTerminatorV1 {
    Fallthrough {
        next_pc: u32,
        next: Option<u32>,
    },
    BrEqz {
        true_pc: u32,
        false_pc: Option<u32>,
        true_block: Option<u32>,
        false_block: Option<u32>,
    },
    Jump {
        target_pc: u32,
        target: Option<u32>,
    },
    DJump,
    Halt,
    Trap(u32),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PvmBlockV1 {
    pub entry_pc: u32,
    pub instructions: Vec<PvmInstructionV1>,
    pub terminator: PvmTerminatorV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PvmProgramV1 {
    pub format_version: u16,
    pub code_hash: [u8; 32],
    pub o_blob: Vec<u8>,
    pub w_blob: Vec<u8>,
    pub z_pages: u16,
    pub s_bytes: u32,
    pub blocks: Vec<PvmBlockV1>,
    pub jump_table: Vec<u32>,
    pub c_blob: Vec<u8>,
}

impl PvmProgramV1 {
    pub fn instruction_count(&self) -> usize {
        self.blocks
            .iter()
            .map(|block| block.instructions.len())
            .sum()
    }

    pub fn validate(&self) -> Result<(), CodecError> {
        if self.format_version != PVM_PROGRAM_FORMAT_V1 {
            return Err(CodecError::InvalidValue("unsupported PvmProgramV1 version"));
        }
        if self.blocks.is_empty() {
            return Err(CodecError::InvalidValue("program must contain a block"));
        }
        for block in &self.blocks {
            if block.instructions.is_empty() {
                return Err(CodecError::InvalidValue("PVM blocks may not be empty"));
            }
            for instruction in &block.instructions {
                for register in [
                    instruction.registers.rd,
                    instruction.registers.ra,
                    instruction.registers.rb,
                ] {
                    if register as usize >= PVM_REGISTER_COUNT {
                        return Err(CodecError::InvalidValue("PVM register index out of range"));
                    }
                }
                if let Some(id) = instruction.host_call_id() {
                    check_host_call(id).map_err(|feature| match feature {
                        UnsupportedFeature::GasHostCall => {
                            CodecError::InvalidValue("GAS host call is forbidden")
                        }
                        UnsupportedFeature::InnerPvm => {
                            CodecError::InvalidValue("Inner PVM host call is forbidden")
                        }
                        UnsupportedFeature::UnsupportedHostCall(_) => {
                            CodecError::InvalidValue("unsupported host call")
                        }
                    })?;
                }
            }
        }
        Ok(())
    }
}

impl CanonicalCodec for PvmProgramV1 {
    fn encode_canonical(&self) -> Vec<u8> {
        encode_with(|w| {
            w.u16(self.format_version);
            w.fixed(&self.code_hash);
            w.bytes(&self.o_blob)?;
            w.bytes(&self.w_blob)?;
            w.u16(self.z_pages);
            w.u32(self.s_bytes);
            w.count(self.blocks.len())?;
            for block in &self.blocks {
                w.u32(block.entry_pc);
                w.count(block.instructions.len())?;
                for instruction in &block.instructions {
                    w.u32(instruction.pc);
                    w.u8(instruction.opcode);
                    w.u8(instruction.registers.rd);
                    w.u8(instruction.registers.ra);
                    w.u8(instruction.registers.rb);
                    w.u8(instruction.pc_delta);
                    w.bytes(&instruction.immediate)?;
                }
                encode_terminator(w, &block.terminator)?;
            }
            w.count(self.jump_table.len())?;
            for entry in &self.jump_table {
                w.u32(*entry);
            }
            w.bytes(&self.c_blob)
        })
        .expect("in-memory canonical encoding cannot fail")
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, CodecError> {
        decode_with(bytes, |r| {
            let format_version = r.u16()?;
            let code_hash = r.fixed()?;
            let o_blob = r.bytes()?;
            let w_blob = r.bytes()?;
            let z_pages = r.u16()?;
            let s_bytes = r.u32()?;
            let blocks = r.vec(|r| {
                let entry_pc = r.u32()?;
                let instructions = r.vec(|r| {
                    Ok(PvmInstructionV1 {
                        pc: r.u32()?,
                        opcode: r.u8()?,
                        registers: RegisterOperandsV1 {
                            rd: r.u8()?,
                            ra: r.u8()?,
                            rb: r.u8()?,
                        },
                        pc_delta: r.u8()?,
                        immediate: r.bytes()?,
                    })
                })?;
                Ok(PvmBlockV1 {
                    entry_pc,
                    instructions,
                    terminator: decode_terminator(r)?,
                })
            })?;
            let jump_table = r.vec(|r| r.u32())?;
            let c_blob = r.bytes()?;
            Ok(Self {
                format_version,
                code_hash,
                o_blob,
                w_blob,
                z_pages,
                s_bytes,
                blocks,
                jump_table,
                c_blob,
            })
        })
    }
}

fn encode_terminator(w: &mut Writer, value: &PvmTerminatorV1) -> Result<(), CodecError> {
    match value {
        PvmTerminatorV1::Fallthrough { next_pc, next } => {
            w.u8(0);
            w.u32(*next_pc);
            encode_optional_u32(w, *next);
        }
        PvmTerminatorV1::BrEqz {
            true_pc,
            false_pc,
            true_block,
            false_block,
        } => {
            w.u8(1);
            w.u32(*true_pc);
            encode_optional_u32(w, *false_pc);
            encode_optional_u32(w, *true_block);
            encode_optional_u32(w, *false_block);
        }
        PvmTerminatorV1::Jump { target_pc, target } => {
            w.u8(2);
            w.u32(*target_pc);
            encode_optional_u32(w, *target);
        }
        PvmTerminatorV1::DJump => w.u8(3),
        PvmTerminatorV1::Halt => w.u8(4),
        PvmTerminatorV1::Trap(pc) => {
            w.u8(5);
            w.u32(*pc);
        }
    }
    Ok(())
}

fn decode_terminator(r: &mut Reader<'_>) -> Result<PvmTerminatorV1, CodecError> {
    Ok(match r.u8()? {
        0 => PvmTerminatorV1::Fallthrough {
            next_pc: r.u32()?,
            next: decode_optional_u32(r)?,
        },
        1 => PvmTerminatorV1::BrEqz {
            true_pc: r.u32()?,
            false_pc: decode_optional_u32(r)?,
            true_block: decode_optional_u32(r)?,
            false_block: decode_optional_u32(r)?,
        },
        2 => PvmTerminatorV1::Jump {
            target_pc: r.u32()?,
            target: decode_optional_u32(r)?,
        },
        3 => PvmTerminatorV1::DJump,
        4 => PvmTerminatorV1::Halt,
        5 => PvmTerminatorV1::Trap(r.u32()?),
        _ => return Err(CodecError::InvalidValue("PVM terminator tag")),
    })
}

fn encode_optional_u32(w: &mut Writer, value: Option<u32>) {
    match value {
        Some(value) => {
            w.u8(1);
            w.u32(value);
        }
        None => w.u8(0),
    }
}

fn decode_optional_u32(r: &mut Reader<'_>) -> Result<Option<u32>, CodecError> {
    match r.u8()? {
        0 => Ok(None),
        1 => Ok(Some(r.u32()?)),
        _ => Err(CodecError::InvalidValue("optional u32 tag")),
    }
}
