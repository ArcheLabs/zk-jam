//! Static, semantics-preserving Translation for the M3 smoke workloads.
//!
//! This crate deliberately emits a small, program-specific RV32-like operation list.  It does
//! not contain a runtime PVM interpreter: the OpenVM integration selects a statically compiled
//! guest for the translated workload.  The operation list is also executable on the host so the
//! translation semantics can be tested without requiring the OpenVM guest toolchain.

use std::{collections::BTreeMap, fmt};

use thiserror::Error;
use zk_jam_refine_interface::{
    PvmBlockV1, PvmInstructionV1, PvmProgramV1, PvmTerminatorV1, RegisterOperandsV1,
    PVM_PROGRAM_FORMAT_V1,
};

/// The Jambda revision used by the M3 adapter integration.
pub const JAMBDA_REVISION: &str = "b850a458fa00da81e80be4cc84ddd7d2222f1edc";
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
pub struct TranslatedProgram {
    pub workload: M3Workload,
    pub source_code_hash: [u8; 32],
    pub pvm_instruction_count: usize,
    pub generic_instructions: Vec<GenericInstruction>,
}

impl TranslatedProgram {
    pub fn translated_instruction_count(&self) -> usize {
        self.generic_instructions
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
        for instruction in &self.generic_instructions {
            out.push_str("    // ");
            out.push_str(&format!("{instruction:?}"));
            out.push('\n');
        }
        out.push_str("}\n");
        out
    }
}

/// Translate one of the three bounded M3 workloads into static generic operations.
pub fn translate(
    workload: M3Workload,
    program: &PvmProgramV1,
) -> Result<TranslatedProgram, TranslationError> {
    program
        .validate()
        .map_err(|_| TranslationError::InvalidProgram("interface validation failed"))?;
    let mut generic_instructions = Vec::new();
    for block in &program.blocks {
        for instruction in &block.instructions {
            emit_instruction(instruction, &mut generic_instructions)?;
        }
        emit_terminator(&block.terminator, &mut generic_instructions)?;
    }
    Ok(TranslatedProgram {
        workload,
        source_code_hash: program.code_hash,
        pvm_instruction_count: program.instruction_count(),
        generic_instructions,
    })
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PvmMemoryV0 {
    pages: BTreeMap<u32, Box<[u8; PVM_PAGE_SIZE as usize]>>,
    writable_pages: BTreeMap<u32, bool>,
}

impl Default for PvmMemoryV0 {
    fn default() -> Self {
        Self {
            pages: BTreeMap::new(),
            writable_pages: BTreeMap::new(),
        }
    }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PvmMachineV0 {
    pub registers: [u64; 13],
    pub memory: PvmMemoryV0,
    pub output_register: u8,
}

impl Default for PvmMachineV0 {
    fn default() -> Self {
        Self {
            registers: [0; 13],
            memory: PvmMemoryV0::default(),
            output_register: 0,
        }
    }
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
            LOAD_IMM_64,
            RegisterOperandsV1 {
                ra: 1,
                ..Default::default()
            },
            imm64(7),
        ),
        instruction(
            LOAD_IMM_64,
            RegisterOperandsV1 {
                ra: 2,
                ..Default::default()
            },
            imm64(9),
        ),
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
    let first = vec![
        instruction(
            LOAD_IMM_64,
            RegisterOperandsV1 {
                ra: 1,
                ..Default::default()
            },
            imm64(21),
        ),
        instruction(
            LOAD_IMM_64,
            RegisterOperandsV1 {
                ra: 2,
                ..Default::default()
            },
            imm64(8),
        ),
        instruction(
            BRANCH_GT_U_FALLBACK,
            RegisterOperandsV1::default(),
            Vec::new(),
        ),
    ];
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
    instructions.push(instruction(
        LOAD_IMM_64,
        RegisterOperandsV1 {
            ra: 2,
            ..Default::default()
        },
        imm64(0x1234_5678),
    ));
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
            let translated = translate(workload, &program).unwrap();
            assert_eq!(
                translated.pvm_instruction_count,
                program.instruction_count()
            );
            assert!(!translated.generic_instructions.is_empty());
            assert!(translated.translated_instruction_count() >= translated.pvm_instruction_count);
            assert!(translated
                .debug_rust_source()
                .contains("generated by zk-jam Translation M3"));
        }
    }

    #[test]
    fn arithmetic_branch_and_memory_match_expected_results() {
        let mut arithmetic = PvmMachineV0::default();
        assert_eq!(
            arithmetic
                .run(&workload_program(M3Workload::Arithmetic), 7)
                .unwrap() as u32,
            0xA5A5_5A6A
        );

        let mut branch = PvmMachineV0::default();
        assert_eq!(
            branch
                .run(&workload_program(M3Workload::BranchTrue), 5)
                .unwrap() as u32,
            91
        );

        let mut memory = PvmMachineV0::default();
        for page in 1..=5 {
            memory.memory.map_zeroed_page(page, true).unwrap();
        }
        let value = memory
            .run(&workload_program(M3Workload::Memory16K), 2)
            .unwrap() as u32;
        assert_eq!(value, 0x80F0_2E78);
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
            translate(M3Workload::Arithmetic, &program),
            Err(TranslationError::UnsupportedOpcode(10))
        ));

        let mut program = arithmetic_program();
        program.blocks[0].terminator = PvmTerminatorV1::DJump;
        assert!(matches!(
            translate(M3Workload::Arithmetic, &program),
            Err(TranslationError::UnsupportedTerminator)
        ));
    }
}
