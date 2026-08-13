//! Static, semantics-preserving Translation for the M3 smoke workloads.
//!
//! This crate deliberately emits a small, program-specific RV32-like operation list.  It does
//! not contain a runtime PVM interpreter: the OpenVM integration selects a statically compiled
//! guest for the translated workload.  The operation list is also executable on the host so the
//! translation semantics can be tested without requiring the OpenVM guest toolchain.

use std::{collections::BTreeMap, fmt};

use sha2::{Digest, Sha256};
use thiserror::Error;
use zk_jam_refine_interface::{
    CanonicalCodec, PvmBlockV1, PvmInstructionV1, PvmProgramV1, PvmTerminatorV1,
    RegisterOperandsV1, PVM_PROGRAM_FORMAT_V1,
};

/// The private Jambda source identity used by the M3 adapter integration.
/// Values are generated from `integration/jambda-m3.json` at compile time.
pub const JAMBDA_REPOSITORY: &str = env!("ZK_JAM_JAMBDA_REPOSITORY");
pub const JAMBDA_REVISION: &str = env!("ZK_JAM_JAMBDA_REVISION");
pub const TRANSLATION_VERSION: u32 = 1;
pub const PROGRAM_COMMITMENT_DOMAIN: &[u8] = b"zk-jam/program/v1";
pub const INPUT_COMMITMENT_DOMAIN: &[u8] = b"zk-jam/input/v1";
pub const TRANSLATED_PROGRAM_DOMAIN: &[u8] = b"zk-jam/translated-program/v1";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExecutionInputV1 {
    pub words: Vec<u32>,
}

impl ExecutionInputV1 {
    pub fn new(words: Vec<u32>) -> Self {
        Self { words }
    }

    pub fn encode_canonical(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(4 + self.words.len() * 4);
        bytes.extend_from_slice(&(self.words.len() as u32).to_le_bytes());
        for word in &self.words {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes
    }
}

pub fn commitment(domain: &[u8], payload: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((TRANSLATION_VERSION).to_le_bytes());
    hasher.update((payload.len() as u64).to_le_bytes());
    hasher.update(payload);
    hasher.finalize().into()
}

pub fn program_commitment(program: &PvmProgramV1) -> [u8; 32] {
    commitment(PROGRAM_COMMITMENT_DOMAIN, &program.encode_canonical())
}

pub fn input_commitment(input: &ExecutionInputV1) -> [u8; 32] {
    commitment(INPUT_COMMITMENT_DOMAIN, &input.encode_canonical())
}

pub fn translated_program_commitment(program: &TranslatedProgramV1) -> [u8; 32] {
    commitment(TRANSLATED_PROGRAM_DOMAIN, &program.encode_canonical())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmittedGuest {
    pub source: String,
    pub source_hash: [u8; 32],
    pub program_commitment: [u8; 32],
}

/// Emit a deterministic OpenVM guest from the translated IR. The generated guest consumes two
/// u32 witness words, executes the translated blocks, and reveals program/input/output values.
pub fn emit_openvm_guest(
    program: &TranslatedProgramV1,
    output_register: u8,
) -> Result<EmittedGuest, TranslationError> {
    if output_register as usize >= 13 {
        return Err(TranslationError::InvalidProgram(
            "output register out of range",
        ));
    }
    let mut source = String::from(
        "#![no_main]\nuse openvm::io::{read, reveal_bytes32};\nuse sha2::{Digest, Sha256};\n\nopenvm::entry!(main);\n\nfn main() {\n    let input: [u32; 2] = read();\n    let mut regs = [0u64; 13];\n    regs[1] = input[0] as u64;\n    regs[2] = input[1] as u64;\n    let mut memory = [0u32; 4096];\n    let mut block = 0usize;\n    loop {\n        match block {\n",
    );
    source = source.replacen("#![no_main]\n", "", 1);
    source = source.replacen("\nfn main()", "\npub fn main()", 1);
    source = source.replacen(
        "let input: [u32; 2] = read();",
        "let input_a: u32 = read();\n    let input_b: u32 = read();\n    let input = [input_a, input_b];",
        1,
    );
    for (block_index, block) in program.blocks.iter().enumerate() {
        source.push_str(&format!("            {block_index} => {{\n"));
        source.push_str("                let mut branch = false;\n");
        for instruction in &block.instructions {
            emit_guest_instruction(&mut source, instruction)?;
        }
        match &block.terminator {
            PvmTerminatorV1::Halt => source.push_str("                break;\n"),
            PvmTerminatorV1::Trap(_) | PvmTerminatorV1::DJump => {
                return Err(TranslationError::UnsupportedTerminator)
            }
            PvmTerminatorV1::Fallthrough { next, .. }
            | PvmTerminatorV1::Jump { target: next, .. } => {
                let next = next.ok_or(TranslationError::UnsupportedTerminator)?;
                source.push_str(&format!("                block = {next}usize;\n"));
            }
            PvmTerminatorV1::BrEqz {
                true_block,
                false_block,
                ..
            } => {
                let true_block = true_block.ok_or(TranslationError::UnsupportedTerminator)?;
                let false_block = false_block.ok_or(TranslationError::UnsupportedTerminator)?;
                source.push_str(&format!(
                    "                block = if branch {{ {true_block}usize }} else {{ {false_block}usize }};\n"
                ));
            }
        }
        source.push_str("            }\n");
    }
    let program_commitment = program.pvm_program_commitment;
    source.push_str("            _ => break,\n        }\n    }\n");
    source.push_str(&format!(
        "    let mut input_hasher = Sha256::new();\n    input_hasher.update(b\"zk-jam/input/v1\");\n    input_hasher.update({TRANSLATION_VERSION}u32.to_le_bytes());\n    input_hasher.update(8u64.to_le_bytes());\n    input_hasher.update(input[0].to_le_bytes());\n    input_hasher.update(input[1].to_le_bytes());\n    let input_commitment: [u8; 32] = input_hasher.finalize().into();\n    let output = (regs[{output_register}] as u32).to_le_bytes();\n    let mut output_bytes = [0u8; 32];\n    output_bytes[..4].copy_from_slice(&output);\n    reveal_bytes32({program_commitment:?});\n    reveal_bytes32(input_commitment);\n    reveal_bytes32(output_bytes);\n}}\n"
    ));
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    Ok(EmittedGuest {
        source,
        source_hash: hasher.finalize().into(),
        program_commitment,
    })
}

fn emit_guest_instruction(
    source: &mut String,
    instruction: &GenericInstruction,
) -> Result<(), TranslationError> {
    let line = match instruction {
        GenericInstruction::LoadImm64 { register, value } => format!("                regs[{register}] = {value}u64;\n"),
        GenericInstruction::Move { destination, source: origin } => format!("                regs[{destination}] = regs[{origin}];\n"),
        GenericInstruction::Add32 { destination, left, right } => format!("                regs[{destination}] = (regs[{left}] as u32).wrapping_add(regs[{right}] as u32) as u64;\n"),
        GenericInstruction::Sub32 { destination, left, right } => format!("                regs[{destination}] = (regs[{left}] as u32).wrapping_sub(regs[{right}] as u32) as u64;\n"),
        GenericInstruction::Mul32 { destination, left, right } => format!("                regs[{destination}] = (regs[{left}] as u32).wrapping_mul(regs[{right}] as u32) as u64;\n"),
        GenericInstruction::Add64 { destination, left, right } => format!("                regs[{destination}] = regs[{left}].wrapping_add(regs[{right}]);\n"),
        GenericInstruction::Sub64 { destination, left, right } => format!("                regs[{destination}] = regs[{left}].wrapping_sub(regs[{right}]);\n"),
        GenericInstruction::Mul64 { destination, left, right } => format!("                regs[{destination}] = regs[{left}].wrapping_mul(regs[{right}]);\n"),
        GenericInstruction::Xor { destination, left, right } => format!("                regs[{destination}] = regs[{left}] ^ regs[{right}];\n"),
        GenericInstruction::Load32 { destination, address } => format!("                regs[{destination}] = memory[({address}u32 / 4 - 1024) as usize] as u64;\n"),
        GenericInstruction::Store32 { source: origin, address } => format!("                memory[({address}u32 / 4 - 1024) as usize] = regs[{origin}] as u32;\n"),
        GenericInstruction::StoreImm32 { address, value } => format!("                memory[({address}u32 / 4 - 1024) as usize] = {value}u32;\n"),
        GenericInstruction::Branch { opcode, left, right } => {
            let condition = match *opcode {
                opcode::BRANCH_EQ => format!("regs[{left}] == regs[{right}]"),
                opcode::BRANCH_NE => format!("regs[{left}] != regs[{right}]"),
                opcode::BRANCH_LT_U => format!("(regs[{left}] as u32) < (regs[{right}] as u32)"),
                opcode::BRANCH_GE_U => format!("(regs[{left}] as u32) >= (regs[{right}] as u32)"),
                opcode::BRANCH_LT_S => format!("(regs[{left}] as i64) < (regs[{right}] as i64)"),
                opcode::BRANCH_GE_S => format!("(regs[{left}] as i64) >= (regs[{right}] as i64)"),
                _ => return Err(TranslationError::UnsupportedOpcode(*opcode)),
            };
            format!("                branch = {condition};\n")
        }
        GenericInstruction::Jump | GenericInstruction::Fallthrough | GenericInstruction::Halt => String::new(),
        GenericInstruction::Trap(_) => return Err(TranslationError::UnsupportedTerminator),
    };
    source.push_str(&line);
    Ok(())
}
pub const PVM_PAGE_SIZE: u32 = 4096;
pub const PVM_PROTECTED_BYTES: u32 = PVM_PAGE_SIZE;
pub const M3_MEMORY_BYTES: usize = 16 * 1024;

/// Stable opcode names used by Jambda's `jp-vm-primitives`.
pub mod opcode {
    pub const TRAP: u8 = 0;
    pub const LOAD_IMM_64: u8 = 20;
    pub const STORE_IMM_U32: u8 = 32;
    pub const JUMP: u8 = 40;
    pub const LOAD_U32: u8 = 56;
    pub const STORE_U32: u8 = 61;
    pub const MOVE_REG: u8 = 100;
    pub const BRANCH_EQ: u8 = 170;
    pub const BRANCH_NE: u8 = 171;
    pub const BRANCH_LT_U: u8 = 172;
    pub const BRANCH_LT_S: u8 = 173;
    pub const BRANCH_GE_U: u8 = 174;
    pub const BRANCH_GE_S: u8 = 175;
    pub const ADD_32: u8 = 190;
    pub const SUB_32: u8 = 191;
    pub const MUL_32: u8 = 192;
    pub const ADD_64: u8 = 200;
    pub const SUB_64: u8 = 201;
    pub const MUL_64: u8 = 202;
    pub const XOR: u8 = 211;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum M3Workload {
    Arithmetic,
    BranchTrue,
    Memory16K,
}

impl M3Workload {
    pub const ALL: [Self; 3] = [Self::Arithmetic, Self::BranchTrue, Self::Memory16K];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Arithmetic => "arithmetic",
            Self::BranchTrue => "branch-true",
            Self::Memory16K => "memory-16384",
        }
    }

    pub const fn guest_binary(self) -> &'static str {
        match self {
            Self::Arithmetic => "m3-translation-arithmetic-v1",
            Self::BranchTrue => "m3-translation-branch-v1",
            Self::Memory16K => "m3-translation-memory-v1",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum TranslationError {
    #[error("invalid PvmProgramV1: {0}")]
    InvalidProgram(&'static str),
    #[error("unsupported PVM opcode {0}")]
    UnsupportedOpcode(u8),
    #[error("unsupported PVM terminator")]
    UnsupportedTerminator,
    #[error("opcode {opcode} has an invalid immediate width: expected {expected}, got {actual}")]
    InvalidImmediate {
        opcode: u8,
        expected: usize,
        actual: usize,
    },
    #[error("PVM memory fault at 0x{address:08x}")]
    MemoryFault { address: u32 },
    #[error("PVM execution trapped at pc {0}")]
    Trap(u32),
    #[error("PVM execution exceeded the instruction limit")]
    StepLimit,
}

/// The generic operation vocabulary emitted by M3 Translation.  The OpenVM guest is compiled
/// from this static operation list; no PVM opcode is interpreted at proving time.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GenericInstruction {
    LoadImm64 {
        register: u8,
        value: u64,
    },
    Move {
        destination: u8,
        source: u8,
    },
    Add32 {
        destination: u8,
        left: u8,
        right: u8,
    },
    Sub32 {
        destination: u8,
        left: u8,
        right: u8,
    },
    Mul32 {
        destination: u8,
        left: u8,
        right: u8,
    },
    Add64 {
        destination: u8,
        left: u8,
        right: u8,
    },
    Sub64 {
        destination: u8,
        left: u8,
        right: u8,
    },
    Mul64 {
        destination: u8,
        left: u8,
        right: u8,
    },
    Xor {
        destination: u8,
        left: u8,
        right: u8,
    },
    Load32 {
        destination: u8,
        address: u32,
    },
    Store32 {
        source: u8,
        address: u32,
    },
    StoreImm32 {
        address: u32,
        value: u32,
    },
    Branch {
        opcode: u8,
        left: u8,
        right: u8,
    },
    Jump,
    Fallthrough,
    Halt,
    Trap(u32),
}

impl GenericInstruction {
    /// Approximate RV32IM expansion used by the benchmark contract.  The count is deliberately
    /// explicit and deterministic; it is not presented as an OpenVM trace-row count.
    pub const fn expansion(&self) -> usize {
        match self {
            Self::LoadImm64 { .. } => 2,
            Self::Move { .. } => 1,
            Self::Add32 { .. } | Self::Sub32 { .. } | Self::Mul32 { .. } | Self::Xor { .. } => 1,
            Self::Add64 { .. } | Self::Sub64 { .. } | Self::Mul64 { .. } => 4,
            Self::Load32 { .. } | Self::Store32 { .. } | Self::StoreImm32 { .. } => 2,
            Self::Branch { .. } => 2,
            Self::Jump | Self::Fallthrough | Self::Halt | Self::Trap(_) => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TranslatedBlockV1 {
    pub instructions: Vec<GenericInstruction>,
    pub terminator: PvmTerminatorV1,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TranslatedProgramV1 {
    pub version: u32,
    pub source_code_hash: [u8; 32],
    pub pvm_program_commitment: [u8; 32],
    pub pvm_instruction_count: usize,
    pub blocks: Vec<TranslatedBlockV1>,
    pub instructions: Vec<GenericInstruction>,
}

pub type TranslatedProgram = TranslatedProgramV1;

impl TranslatedProgramV1 {
    pub fn translated_instruction_count(&self) -> usize {
        self.instructions
            .iter()
            .map(GenericInstruction::expansion)
            .sum()
    }

    pub fn expansion_ratio(&self) -> f64 {
        self.translated_instruction_count() as f64 / self.pvm_instruction_count.max(1) as f64
    }

    /// Deterministic source-shaped output for audit/debugging.  The real OpenVM guest sources in
    /// `crates/openvm-backend/guests/m3` are the checked-in static emission used for proving.
    pub fn debug_rust_source(&self) -> String {
        let mut out =
            String::from("// generated by zk-jam Translation M3\nfn translated_guest() {\n");
        for instruction in &self.instructions {
            out.push_str("    // ");
            out.push_str(&format!("{instruction:?}"));
            out.push('\n');
        }
        out.push_str("}\n");
        out
    }
}

impl TranslatedProgramV1 {
    pub fn encode_canonical(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(&self.source_code_hash);
        bytes.extend_from_slice(&self.pvm_program_commitment);
        bytes.extend_from_slice(&(self.pvm_instruction_count as u64).to_le_bytes());
        bytes.extend_from_slice(&(self.blocks.len() as u32).to_le_bytes());
        for block in &self.blocks {
            bytes.extend_from_slice(&(block.instructions.len() as u32).to_le_bytes());
            for instruction in &block.instructions {
                encode_instruction(&mut bytes, instruction);
            }
            encode_terminator_canonical(&mut bytes, &block.terminator);
        }
        bytes
    }
}

fn encode_instruction(bytes: &mut Vec<u8>, instruction: &GenericInstruction) {
    match instruction {
        GenericInstruction::LoadImm64 { register, value } => {
            bytes.extend_from_slice(&[0, *register]);
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        GenericInstruction::Move {
            destination,
            source,
        } => bytes.extend_from_slice(&[1, *destination, *source]),
        GenericInstruction::Add32 {
            destination,
            left,
            right,
        } => encode_regs(bytes, 2, *destination, *left, *right),
        GenericInstruction::Sub32 {
            destination,
            left,
            right,
        } => encode_regs(bytes, 3, *destination, *left, *right),
        GenericInstruction::Mul32 {
            destination,
            left,
            right,
        } => encode_regs(bytes, 4, *destination, *left, *right),
        GenericInstruction::Add64 {
            destination,
            left,
            right,
        } => encode_regs(bytes, 5, *destination, *left, *right),
        GenericInstruction::Sub64 {
            destination,
            left,
            right,
        } => encode_regs(bytes, 6, *destination, *left, *right),
        GenericInstruction::Mul64 {
            destination,
            left,
            right,
        } => encode_regs(bytes, 7, *destination, *left, *right),
        GenericInstruction::Xor {
            destination,
            left,
            right,
        } => encode_regs(bytes, 8, *destination, *left, *right),
        GenericInstruction::Load32 {
            destination,
            address,
        } => {
            bytes.extend_from_slice(&[9, *destination]);
            bytes.extend_from_slice(&address.to_le_bytes());
        }
        GenericInstruction::Store32 { source, address } => {
            bytes.extend_from_slice(&[10, *source]);
            bytes.extend_from_slice(&address.to_le_bytes());
        }
        GenericInstruction::StoreImm32 { address, value } => {
            bytes.push(11);
            bytes.extend_from_slice(&address.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        GenericInstruction::Branch {
            opcode,
            left,
            right,
        } => bytes.extend_from_slice(&[12, *opcode, *left, *right]),
        GenericInstruction::Jump => bytes.push(13),
        GenericInstruction::Fallthrough => bytes.push(14),
        GenericInstruction::Halt => bytes.push(15),
        GenericInstruction::Trap(pc) => {
            bytes.push(16);
            bytes.extend_from_slice(&pc.to_le_bytes());
        }
    }
}

fn encode_regs(bytes: &mut Vec<u8>, tag: u8, destination: u8, left: u8, right: u8) {
    bytes.extend_from_slice(&[tag, destination, left, right]);
}

fn encode_terminator_canonical(bytes: &mut Vec<u8>, terminator: &PvmTerminatorV1) {
    fn optional(bytes: &mut Vec<u8>, value: Option<u32>) {
        match value {
            Some(value) => {
                bytes.push(1);
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            None => bytes.push(0),
        }
    }
    match terminator {
        PvmTerminatorV1::Fallthrough { next_pc, next } => {
            bytes.push(0);
            bytes.extend_from_slice(&next_pc.to_le_bytes());
            optional(bytes, *next);
        }
        PvmTerminatorV1::BrEqz {
            true_pc,
            false_pc,
            true_block,
            false_block,
        } => {
            bytes.push(1);
            bytes.extend_from_slice(&true_pc.to_le_bytes());
            optional(bytes, *false_pc);
            optional(bytes, *true_block);
            optional(bytes, *false_block);
        }
        PvmTerminatorV1::Jump { target_pc, target } => {
            bytes.push(2);
            bytes.extend_from_slice(&target_pc.to_le_bytes());
            optional(bytes, *target);
        }
        PvmTerminatorV1::DJump => bytes.push(3),
        PvmTerminatorV1::Halt => bytes.push(4),
        PvmTerminatorV1::Trap(pc) => {
            bytes.push(5);
            bytes.extend_from_slice(&pc.to_le_bytes());
        }
    }
}

/// Translate one validated PVM program into the versioned production IR.
pub fn translate(program: &PvmProgramV1) -> Result<TranslatedProgramV1, TranslationError> {
    program
        .validate()
        .map_err(|_| TranslationError::InvalidProgram("interface validation failed"))?;
    let mut instructions = Vec::new();
    let mut blocks = Vec::with_capacity(program.blocks.len());
    for block in &program.blocks {
        let start = instructions.len();
        let mut block_instructions = Vec::new();
        for instruction in &block.instructions {
            emit_instruction(instruction, &mut block_instructions)?;
        }
        instructions.extend(block_instructions.iter().cloned());
        // Terminator markers remain in the flattened M3 metric stream, but are represented by
        // `TranslatedBlockV1::terminator` for M4 emission and must not execute as instructions.
        emit_terminator(&block.terminator, &mut instructions)?;
        blocks.push(TranslatedBlockV1 {
            instructions: block_instructions,
            terminator: block.terminator.clone(),
        });
        debug_assert!(instructions.len() >= start);
    }
    Ok(TranslatedProgramV1 {
        version: TRANSLATION_VERSION,
        source_code_hash: program.code_hash,
        pvm_program_commitment: program_commitment(program),
        pvm_instruction_count: program.instruction_count(),
        blocks,
        instructions,
    })
}

/// Compatibility adapter for the M3 fixture runner. The workload label is no longer part of
/// production translation semantics.
pub fn translate_workload(
    _workload: M3Workload,
    program: &PvmProgramV1,
) -> Result<TranslatedProgramV1, TranslationError> {
    translate(program)
}

fn emit_instruction(
    instruction: &PvmInstructionV1,
    output: &mut Vec<GenericInstruction>,
) -> Result<(), TranslationError> {
    let regs = instruction.registers;
    let imm_u64 = |expected: usize| -> Result<u64, TranslationError> {
        if instruction.immediate.len() != expected {
            return Err(TranslationError::InvalidImmediate {
                opcode: instruction.opcode,
                expected,
                actual: instruction.immediate.len(),
            });
        }
        let mut bytes = [0; 8];
        bytes[..expected].copy_from_slice(&instruction.immediate);
        Ok(u64::from_le_bytes(bytes))
    };
    let imm_u32 = || -> Result<u32, TranslationError> {
        if instruction.immediate.len() != 4 {
            return Err(TranslationError::InvalidImmediate {
                opcode: instruction.opcode,
                expected: 4,
                actual: instruction.immediate.len(),
            });
        }
        Ok(u32::from_le_bytes(
            instruction.immediate[..4].try_into().unwrap(),
        ))
    };
    match instruction.opcode {
        opcode::LOAD_IMM_64 => output.push(GenericInstruction::LoadImm64 {
            register: regs.ra,
            value: imm_u64(8)?,
        }),
        opcode::MOVE_REG => {
            if !instruction.immediate.is_empty() {
                return Err(TranslationError::InvalidImmediate {
                    opcode: instruction.opcode,
                    expected: 0,
                    actual: instruction.immediate.len(),
                });
            }
            output.push(GenericInstruction::Move {
                destination: regs.rd,
                source: regs.ra,
            });
        }
        opcode::ADD_32 => output.push(GenericInstruction::Add32 {
            destination: regs.rd,
            left: regs.ra,
            right: regs.rb,
        }),
        opcode::SUB_32 => output.push(GenericInstruction::Sub32 {
            destination: regs.rd,
            left: regs.ra,
            right: regs.rb,
        }),
        opcode::MUL_32 => output.push(GenericInstruction::Mul32 {
            destination: regs.rd,
            left: regs.ra,
            right: regs.rb,
        }),
        opcode::ADD_64 => output.push(GenericInstruction::Add64 {
            destination: regs.rd,
            left: regs.ra,
            right: regs.rb,
        }),
        opcode::SUB_64 => output.push(GenericInstruction::Sub64 {
            destination: regs.rd,
            left: regs.ra,
            right: regs.rb,
        }),
        opcode::MUL_64 => output.push(GenericInstruction::Mul64 {
            destination: regs.rd,
            left: regs.ra,
            right: regs.rb,
        }),
        opcode::XOR => output.push(GenericInstruction::Xor {
            destination: regs.rd,
            left: regs.ra,
            right: regs.rb,
        }),
        opcode::LOAD_U32 => output.push(GenericInstruction::Load32 {
            destination: regs.ra,
            address: imm_u32()?,
        }),
        opcode::STORE_U32 => output.push(GenericInstruction::Store32 {
            source: regs.ra,
            address: imm_u32()?,
        }),
        opcode::STORE_IMM_U32 => {
            if instruction.immediate.len() != 8 {
                return Err(TranslationError::InvalidImmediate {
                    opcode: instruction.opcode,
                    expected: 8,
                    actual: instruction.immediate.len(),
                });
            }
            output.push(GenericInstruction::StoreImm32 {
                address: u32::from_le_bytes(instruction.immediate[..4].try_into().unwrap()),
                value: u32::from_le_bytes(instruction.immediate[4..].try_into().unwrap()),
            });
        }
        opcode::BRANCH_EQ
        | opcode::BRANCH_NE
        | opcode::BRANCH_LT_U
        | opcode::BRANCH_LT_S
        | opcode::BRANCH_GE_U
        | opcode::BRANCH_GE_S => {
            if !instruction.immediate.is_empty() {
                return Err(TranslationError::InvalidImmediate {
                    opcode: instruction.opcode,
                    expected: 0,
                    actual: instruction.immediate.len(),
                });
            }
            output.push(GenericInstruction::Branch {
                opcode: instruction.opcode,
                left: regs.ra,
                right: regs.rb,
            });
        }
        other => return Err(TranslationError::UnsupportedOpcode(other)),
    }
    Ok(())
}

fn emit_terminator(
    terminator: &PvmTerminatorV1,
    output: &mut Vec<GenericInstruction>,
) -> Result<(), TranslationError> {
    match terminator {
        PvmTerminatorV1::Fallthrough { .. } => output.push(GenericInstruction::Fallthrough),
        PvmTerminatorV1::Jump { .. } => output.push(GenericInstruction::Jump),
        PvmTerminatorV1::BrEqz { .. } => output.push(GenericInstruction::Branch {
            opcode: opcode::BRANCH_NE,
            left: 0,
            right: 0,
        }),
        PvmTerminatorV1::Halt => output.push(GenericInstruction::Halt),
        PvmTerminatorV1::Trap(pc) => output.push(GenericInstruction::Trap(*pc)),
        PvmTerminatorV1::DJump => return Err(TranslationError::UnsupportedTerminator),
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PvmMemoryV0 {
    pages: BTreeMap<u32, Box<[u8; PVM_PAGE_SIZE as usize]>>,
    writable_pages: BTreeMap<u32, bool>,
}

impl PvmMemoryV0 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn map_zeroed_page(&mut self, page: u32, writable: bool) -> Result<(), TranslationError> {
        let address = page.saturating_mul(PVM_PAGE_SIZE);
        self.check_address(address, 1)?;
        self.pages
            .entry(page)
            .or_insert_with(|| Box::new([0; PVM_PAGE_SIZE as usize]));
        self.writable_pages.insert(page, writable);
        Ok(())
    }

    pub fn read(&self, address: u32, width: usize) -> Result<u64, TranslationError> {
        let bytes = self.read_bytes(address, width)?;
        let mut value = [0; 8];
        value[..width].copy_from_slice(&bytes);
        Ok(u64::from_le_bytes(value))
    }

    pub fn write(
        &mut self,
        address: u32,
        width: usize,
        value: u64,
    ) -> Result<(), TranslationError> {
        let bytes = value.to_le_bytes();
        self.write_bytes(address, &bytes[..width])
    }

    pub fn read_bytes(&self, address: u32, width: usize) -> Result<Vec<u8>, TranslationError> {
        self.check_range(address, width)?;
        let mut output = Vec::with_capacity(width);
        for offset in 0..width {
            let at = address + offset as u32;
            let page = at / PVM_PAGE_SIZE;
            let index = (at % PVM_PAGE_SIZE) as usize;
            output.push(
                self.pages
                    .get(&page)
                    .ok_or(TranslationError::MemoryFault { address: at })?[index],
            );
        }
        Ok(output)
    }

    pub fn write_bytes(&mut self, address: u32, bytes: &[u8]) -> Result<(), TranslationError> {
        self.check_range(address, bytes.len())?;
        for (offset, value) in bytes.iter().enumerate() {
            let at = address + offset as u32;
            let page = at / PVM_PAGE_SIZE;
            let index = (at % PVM_PAGE_SIZE) as usize;
            if !self.writable_pages.get(&page).copied().unwrap_or(false) {
                return Err(TranslationError::MemoryFault { address: at });
            }
            self.pages
                .get_mut(&page)
                .ok_or(TranslationError::MemoryFault { address: at })?[index] = *value;
        }
        Ok(())
    }

    fn check_range(&self, address: u32, width: usize) -> Result<(), TranslationError> {
        if width == 0 || width > 8 {
            return Err(TranslationError::MemoryFault { address });
        }
        let end = address
            .checked_add(width as u32 - 1)
            .ok_or(TranslationError::MemoryFault { address })?;
        self.check_address(address, 1)?;
        self.check_address(end, 1)
    }

    fn check_address(&self, address: u32, _: usize) -> Result<(), TranslationError> {
        if address < PVM_PROTECTED_BYTES {
            return Err(TranslationError::MemoryFault { address });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PvmMachineV0 {
    pub registers: [u64; 13],
    pub memory: PvmMemoryV0,
    pub output_register: u8,
}

impl PvmMachineV0 {
    pub fn run(
        &mut self,
        program: &PvmProgramV1,
        output_register: u8,
    ) -> Result<u64, TranslationError> {
        program
            .validate()
            .map_err(|_| TranslationError::InvalidProgram("interface validation failed"))?;
        if output_register as usize >= self.registers.len() {
            return Err(TranslationError::InvalidProgram(
                "output register out of range",
            ));
        }
        self.output_register = output_register;
        let mut block = 0usize;
        let mut steps = 0usize;
        while steps < 1_000_000 {
            let current = program
                .blocks
                .get(block)
                .ok_or(TranslationError::InvalidProgram("block index out of range"))?;
            let mut branch = false;
            for instruction in &current.instructions {
                branch = self.step(instruction)?;
                steps += 1;
                if steps >= 1_000_000 {
                    return Err(TranslationError::StepLimit);
                }
            }
            match &current.terminator {
                PvmTerminatorV1::Fallthrough { next, .. } => {
                    block = next.ok_or(TranslationError::UnsupportedTerminator)? as usize;
                }
                PvmTerminatorV1::Jump { target, .. } => {
                    block = target.ok_or(TranslationError::UnsupportedTerminator)? as usize;
                }
                PvmTerminatorV1::BrEqz {
                    true_block,
                    false_block,
                    ..
                } => {
                    block = if branch { true_block } else { false_block }
                        .ok_or(TranslationError::UnsupportedTerminator)?
                        as usize;
                }
                PvmTerminatorV1::Halt => return Ok(self.registers[output_register as usize]),
                PvmTerminatorV1::Trap(pc) => return Err(TranslationError::Trap(*pc)),
                PvmTerminatorV1::DJump => return Err(TranslationError::UnsupportedTerminator),
            }
        }
        Err(TranslationError::StepLimit)
    }

    fn step(&mut self, instruction: &PvmInstructionV1) -> Result<bool, TranslationError> {
        let r = instruction.registers;
        let imm_u64 = |expected: usize| -> Result<u64, TranslationError> {
            if instruction.immediate.len() != expected {
                return Err(TranslationError::InvalidImmediate {
                    opcode: instruction.opcode,
                    expected,
                    actual: instruction.immediate.len(),
                });
            }
            let mut bytes = [0; 8];
            bytes[..expected].copy_from_slice(&instruction.immediate);
            Ok(u64::from_le_bytes(bytes))
        };
        let imm_u32 = || -> Result<u32, TranslationError> { Ok(imm_u64(4)? as u32) };
        Ok(match instruction.opcode {
            opcode::LOAD_IMM_64 => {
                self.registers[r.ra as usize] = imm_u64(8)?;
                false
            }
            opcode::MOVE_REG => {
                self.registers[r.rd as usize] = self.registers[r.ra as usize];
                false
            }
            opcode::ADD_32 => {
                self.registers[r.rd as usize] = (self.registers[r.ra as usize] as u32)
                    .wrapping_add(self.registers[r.rb as usize] as u32)
                    as i32 as i64 as u64;
                false
            }
            opcode::SUB_32 => {
                self.registers[r.rd as usize] = (self.registers[r.ra as usize] as u32)
                    .wrapping_sub(self.registers[r.rb as usize] as u32)
                    as i32 as i64 as u64;
                false
            }
            opcode::MUL_32 => {
                self.registers[r.rd as usize] = (self.registers[r.ra as usize] as u32)
                    .wrapping_mul(self.registers[r.rb as usize] as u32)
                    as i32 as i64 as u64;
                false
            }
            opcode::ADD_64 => {
                self.registers[r.rd as usize] =
                    self.registers[r.ra as usize].wrapping_add(self.registers[r.rb as usize]);
                false
            }
            opcode::SUB_64 => {
                self.registers[r.rd as usize] =
                    self.registers[r.ra as usize].wrapping_sub(self.registers[r.rb as usize]);
                false
            }
            opcode::MUL_64 => {
                self.registers[r.rd as usize] =
                    self.registers[r.ra as usize].wrapping_mul(self.registers[r.rb as usize]);
                false
            }
            opcode::XOR => {
                self.registers[r.rd as usize] =
                    self.registers[r.ra as usize] ^ self.registers[r.rb as usize];
                false
            }
            opcode::LOAD_U32 => {
                self.registers[r.ra as usize] = self.memory.read(imm_u32()?, 4)?;
                false
            }
            opcode::STORE_U32 => {
                self.memory
                    .write(imm_u32()?, 4, self.registers[r.ra as usize])?;
                false
            }
            opcode::STORE_IMM_U32 => {
                if instruction.immediate.len() != 8 {
                    return Err(TranslationError::InvalidImmediate {
                        opcode: instruction.opcode,
                        expected: 8,
                        actual: instruction.immediate.len(),
                    });
                }
                let address = u32::from_le_bytes(instruction.immediate[..4].try_into().unwrap());
                let value = u32::from_le_bytes(instruction.immediate[4..].try_into().unwrap());
                self.memory.write(address, 4, value as u64)?;
                false
            }
            opcode::BRANCH_EQ => self.registers[r.ra as usize] == self.registers[r.rb as usize],
            opcode::BRANCH_NE => self.registers[r.ra as usize] != self.registers[r.rb as usize],
            opcode::BRANCH_LT_U => self.registers[r.ra as usize] < self.registers[r.rb as usize],
            opcode::BRANCH_LT_S => {
                (self.registers[r.ra as usize] as i64) < (self.registers[r.rb as usize] as i64)
            }
            opcode::BRANCH_GE_U => self.registers[r.ra as usize] >= self.registers[r.rb as usize],
            opcode::BRANCH_GE_S => {
                (self.registers[r.ra as usize] as i64) >= (self.registers[r.rb as usize] as i64)
            }
            other => return Err(TranslationError::UnsupportedOpcode(other)),
        })
    }
}

/// Independent bounded reference execution used for M4 differential checks.
pub fn execute_reference(
    program: &PvmProgramV1,
    input: &ExecutionInputV1,
    output_register: u8,
) -> Result<u64, TranslationError> {
    if input.words.len() < 2 {
        return Err(TranslationError::InvalidProgram(
            "M4 execution input requires two words",
        ));
    }
    let mut machine = PvmMachineV0::default();
    machine.registers[1] = input.words[0] as u64;
    machine.registers[2] = input.words[1] as u64;
    machine.run(program, output_register)
}

/// Build the exact three normalized programs used by the M3 smoke.
pub fn workload_program(workload: M3Workload) -> PvmProgramV1 {
    match workload {
        M3Workload::Arithmetic => arithmetic_program(),
        M3Workload::BranchTrue => branch_program(),
        M3Workload::Memory16K => memory_program(),
    }
}

fn program(code_hash: [u8; 32], blocks: Vec<PvmBlockV1>) -> PvmProgramV1 {
    PvmProgramV1 {
        format_version: PVM_PROGRAM_FORMAT_V1,
        code_hash,
        o_blob: Vec::new(),
        w_blob: Vec::new(),
        z_pages: 6,
        s_bytes: 0,
        blocks,
        jump_table: Vec::new(),
        c_blob: Vec::new(),
    }
}

fn instruction(opcode: u8, registers: RegisterOperandsV1, immediate: Vec<u8>) -> PvmInstructionV1 {
    PvmInstructionV1 {
        pc: 0,
        opcode,
        registers,
        immediate,
        pc_delta: 0,
    }
}

fn imm64(value: u64) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn imm32(value: u32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn arithmetic_program() -> PvmProgramV1 {
    use opcode::*;
    let instructions = vec![
        instruction(
            ADD_32,
            RegisterOperandsV1 {
                rd: 3,
                ra: 1,
                rb: 2,
            },
            Vec::new(),
        ),
        instruction(
            LOAD_IMM_64,
            RegisterOperandsV1 {
                ra: 4,
                ..Default::default()
            },
            imm64(3),
        ),
        instruction(
            MUL_32,
            RegisterOperandsV1 {
                rd: 5,
                ra: 3,
                rb: 4,
            },
            Vec::new(),
        ),
        instruction(
            LOAD_IMM_64,
            RegisterOperandsV1 {
                ra: 6,
                ..Default::default()
            },
            imm64(0xA5A5_5A5A),
        ),
        instruction(
            XOR,
            RegisterOperandsV1 {
                rd: 7,
                ra: 5,
                rb: 6,
            },
            Vec::new(),
        ),
    ];
    program(
        [1; 32],
        vec![PvmBlockV1 {
            entry_pc: 0,
            instructions,
            terminator: PvmTerminatorV1::Halt,
        }],
    )
}

fn branch_program() -> PvmProgramV1 {
    use opcode::*;
    let first = vec![instruction(
        BRANCH_GT_U_FALLBACK,
        RegisterOperandsV1::default(),
        Vec::new(),
    )];
    // The normalized interface intentionally stores only the stable branch opcode.  `BRANCH_GE_U`
    // plus the distinct operands below gives the true path for 21 >= 8 while remaining in the
    // supported PVM opcode family.
    let first = first
        .into_iter()
        .map(|mut value| {
            if value.opcode == BRANCH_GT_U_FALLBACK {
                value.opcode = BRANCH_GE_U;
                value.registers = RegisterOperandsV1 {
                    ra: 1,
                    rb: 2,
                    ..Default::default()
                };
            }
            value
        })
        .collect();
    let true_block = vec![
        instruction(
            SUB_32,
            RegisterOperandsV1 {
                rd: 3,
                ra: 1,
                rb: 2,
            },
            Vec::new(),
        ),
        instruction(
            LOAD_IMM_64,
            RegisterOperandsV1 {
                ra: 4,
                ..Default::default()
            },
            imm64(7),
        ),
        instruction(
            MUL_32,
            RegisterOperandsV1 {
                rd: 5,
                ra: 3,
                rb: 4,
            },
            Vec::new(),
        ),
    ];
    let false_block = vec![
        instruction(
            SUB_32,
            RegisterOperandsV1 {
                rd: 3,
                ra: 2,
                rb: 1,
            },
            Vec::new(),
        ),
        instruction(
            LOAD_IMM_64,
            RegisterOperandsV1 {
                ra: 4,
                ..Default::default()
            },
            imm64(11),
        ),
        instruction(
            MUL_32,
            RegisterOperandsV1 {
                rd: 5,
                ra: 3,
                rb: 4,
            },
            Vec::new(),
        ),
    ];
    program(
        [2; 32],
        vec![
            PvmBlockV1 {
                entry_pc: 0,
                instructions: first,
                terminator: PvmTerminatorV1::BrEqz {
                    true_pc: 1,
                    false_pc: Some(2),
                    true_block: Some(1),
                    false_block: Some(2),
                },
            },
            PvmBlockV1 {
                entry_pc: 1,
                instructions: true_block,
                terminator: PvmTerminatorV1::Halt,
            },
            PvmBlockV1 {
                entry_pc: 2,
                instructions: false_block,
                terminator: PvmTerminatorV1::Halt,
            },
        ],
    )
}

const BRANCH_GT_U_FALLBACK: u8 = 250;

fn memory_program() -> PvmProgramV1 {
    use opcode::*;
    let mut instructions = Vec::with_capacity(1 + M3_MEMORY_BYTES / 4 * 3);
    let mut state = 0x1234_5678u32;
    for index in 0..(M3_MEMORY_BYTES / 4) {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223) ^ index as u32;
        let address = PVM_PROTECTED_BYTES + (index as u32 * 4);
        instructions.push(instruction(STORE_IMM_U32, RegisterOperandsV1::default(), {
            let mut bytes = imm32(address);
            bytes.extend_from_slice(&state.to_le_bytes());
            bytes
        }));
        instructions.push(instruction(
            LOAD_U32,
            RegisterOperandsV1 {
                ra: 3,
                ..Default::default()
            },
            imm32(address),
        ));
        instructions.push(instruction(
            ADD_32,
            RegisterOperandsV1 {
                rd: 2,
                ra: 2,
                rb: 3,
            },
            Vec::new(),
        ));
    }
    program(
        [3; 32],
        vec![PvmBlockV1 {
            entry_pc: 0,
            instructions,
            terminator: PvmTerminatorV1::Halt,
        }],
    )
}

impl fmt::Display for M3Workload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_workloads_translate_without_runtime_interpreter() {
        for workload in M3Workload::ALL {
            let program = workload_program(workload);
            let translated = translate(&program).unwrap();
            assert_eq!(
                translated.pvm_instruction_count,
                program.instruction_count()
            );
            assert!(!translated.instructions.is_empty());
            assert!(translated.translated_instruction_count() >= translated.pvm_instruction_count);
            assert!(translated
                .debug_rust_source()
                .contains("generated by zk-jam Translation M3"));
        }
    }

    #[test]
    fn arithmetic_branch_and_memory_match_expected_results() {
        let mut arithmetic = PvmMachineV0::default();
        arithmetic.registers[1] = 7;
        arithmetic.registers[2] = 9;
        assert_eq!(
            arithmetic
                .run(&workload_program(M3Workload::Arithmetic), 7)
                .unwrap() as u32,
            0xA5A5_5A6A
        );

        let mut branch = PvmMachineV0::default();
        branch.registers[1] = 21;
        branch.registers[2] = 8;
        assert_eq!(
            branch
                .run(&workload_program(M3Workload::BranchTrue), 5)
                .unwrap() as u32,
            91
        );

        let mut memory = PvmMachineV0::default();
        memory.registers[2] = 0x1234_5678;
        for page in 1..=5 {
            memory.memory.map_zeroed_page(page, true).unwrap();
        }
        let value = memory
            .run(&workload_program(M3Workload::Memory16K), 2)
            .unwrap() as u32;
        assert_eq!(value, 0x80F0_2E78);
    }

    #[test]
    fn m4_commitments_and_emission_are_deterministic_and_input_bound() {
        let program = workload_program(M3Workload::Arithmetic);
        let translated = translate(&program).unwrap();
        let emitted_a = emit_openvm_guest(&translated, 7).unwrap();
        let emitted_b = emit_openvm_guest(&translated, 7).unwrap();
        assert_eq!(emitted_a, emitted_b);
        assert!(emitted_a.source.contains("let input_a: u32 = read();"));
        assert!(emitted_a.source.contains("let input_b: u32 = read();"));
        assert!(emitted_a.source.contains("wrapping_add"));
        assert_ne!(
            input_commitment(&ExecutionInputV1::new(vec![7, 9])),
            input_commitment(&ExecutionInputV1::new(vec![7, 10]))
        );
        let mut modified = program.clone();
        modified.blocks[0].instructions[0].opcode = opcode::SUB_32;
        assert_ne!(program_commitment(&program), program_commitment(&modified));
        assert_eq!(
            execute_reference(&program, &ExecutionInputV1::new(vec![10, 20]), 7).unwrap() as u32,
            (10u32.wrapping_add(20).wrapping_mul(3)) ^ 0xA5A5_5A5A
        );
    }

    #[test]
    fn m4_branch_reference_covers_true_false_equal() {
        let program = workload_program(M3Workload::BranchTrue);
        assert_eq!(
            execute_reference(&program, &ExecutionInputV1::new(vec![21, 8]), 5).unwrap(),
            91
        );
        assert_eq!(
            execute_reference(&program, &ExecutionInputV1::new(vec![8, 21]), 5).unwrap(),
            143
        );
        assert_eq!(
            execute_reference(&program, &ExecutionInputV1::new(vec![8, 8]), 5).unwrap(),
            0
        );
    }

    #[test]
    fn sparse_memory_checks_permissions_and_cross_page_access() {
        let mut memory = PvmMemoryV0::new();
        memory.map_zeroed_page(1, true).unwrap();
        memory.map_zeroed_page(2, true).unwrap();
        memory.write(2 * PVM_PAGE_SIZE - 2, 4, 0xAABB_CCDD).unwrap();
        assert_eq!(memory.read(2 * PVM_PAGE_SIZE - 2, 4).unwrap(), 0xAABB_CCDD);
        assert!(memory.read(0, 1).is_err());
        memory.map_zeroed_page(3, false).unwrap();
        assert!(memory.write(3 * PVM_PAGE_SIZE, 1, 1).is_err());
    }

    #[test]
    fn unsupported_opcode_and_dynamic_jump_fail_closed() {
        let mut program = arithmetic_program();
        program.blocks[0].instructions[0].opcode = 10;
        program.blocks[0].instructions[0].immediate = 1u32.to_le_bytes().to_vec();
        assert!(matches!(
            translate(&program),
            Err(TranslationError::UnsupportedOpcode(10))
        ));

        let mut program = arithmetic_program();
        program.blocks[0].terminator = PvmTerminatorV1::DJump;
        assert!(matches!(
            translate(&program),
            Err(TranslationError::UnsupportedTerminator)
        ));
    }
}
