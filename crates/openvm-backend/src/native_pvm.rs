//! Bounded Native PVM lowering for the existing M4 `PvmProgramV1` fixtures.
//!
//! This module deliberately stops at the arithmetic, branch, and memory operations used by the
//! six M4 execute-only cases. It does not implement the full PVM, Host Calls, Refine, or a new
//! OpenVM chip. The result is a direct `VmExe`, consumed by the same M4 SDK façade as the
//! frontend-translated guest.

use std::collections::BTreeMap;

use openvm_sdk::{
    openvm_circuit::arch::instructions::{
        exe::VmExe,
        instruction::Instruction,
        program::Program,
        riscv::{RV32_MEMORY_AS, RV32_REGISTER_AS},
        LocalOpcode, PhantomDiscriminant, SystemOpcode, VmOpcode,
    },
    F,
};
use openvm_stark_backend::p3_field::PrimeCharacteristicRing;
use thiserror::Error;
use zk_jam_refine_interface::{PvmProgramV1, PvmTerminatorV1};
use zk_jam_translation::{
    opcode, program_commitment, GenericInstruction, TranslatedBlockV1, M3_MEMORY_BYTES,
    PVM_PROTECTED_BYTES,
};

const BASE_ALU_ADD: usize = 0x200;
const BASE_ALU_SUB: usize = 0x201;
const BASE_ALU_XOR: usize = 0x202;
const BASE_ALU_OR: usize = 0x203;
const SHIFT_SLL: usize = 0x205;
const SHIFT_SRL: usize = 0x206;
const LOAD_STORE_LOADW: usize = 0x210;
const LOAD_STORE_STOREW: usize = 0x213;
const LOAD_STORE_STOREB: usize = 0x215;
const BRANCH_BEQ: usize = 0x220;
const BRANCH_BNE: usize = 0x221;
const BRANCH_BLT: usize = 0x225;
const BRANCH_BLTU: usize = 0x226;
const BRANCH_BGE: usize = 0x227;
const BRANCH_BGEU: usize = 0x228;
const JAL: usize = 0x230;
const LUI: usize = 0x231;
const MUL: usize = 0x250;
const HINT_STOREW: usize = 0x260;
const SHA256: usize = 0x320;
const HINT_INPUT: u16 = 0x20;

// OpenVM RV32 register-memory cells outside the thirteen PVM register pointers.
const INPUT_PTR_SLOT: u32 = 52;
const PUBLIC_INDEX_SLOT: u32 = 56;
const MEMORY_BASE_SLOT: u32 = 60;
const ZERO_SLOT: u32 = 64;
const SHA_INPUT_PTR_SLOT: u32 = 68;
const SHA_STATE_PTR_SLOT: u32 = 72;
const SHA_DIGEST_PTR_SLOT: u32 = 76;

const INPUT_BASE: u32 = PVM_PROTECTED_BYTES;
const INPUT_SHADOW_BASE: u32 = 0x5800;
const SHA_STATE_BASE: u32 = 0x6000;
const SHA_INPUT_BASE: u32 = 0x6040;
const PROGRAM_COMMITMENT_BASE: u32 = 0x7000;

/// Fixed PVM register to OpenVM RV32 register-memory pointer mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativePvmRegisterMap {
    pub pvm_to_openvm: [u32; 13],
}

impl Default for NativePvmRegisterMap {
    fn default() -> Self {
        Self {
            pvm_to_openvm: std::array::from_fn(|index| (index as u32) * 4),
        }
    }
}

impl NativePvmRegisterMap {
    pub fn pointer(&self, register: u8) -> Result<u32, NativePvmError> {
        self.pvm_to_openvm
            .get(register as usize)
            .copied()
            .ok_or(NativePvmError::InvalidRegister(register))
    }
}

/// Explicit mapping from normalized PVM blocks to direct OpenVM instruction PCs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativePcMap {
    pub pvm_block_to_openvm_pc: Vec<u32>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum NativePvmError {
    #[error("PVM register {0} is out of range")]
    InvalidRegister(u8),
    #[error("PVM translation failed: {0}")]
    Translation(#[from] zk_jam_translation::TranslationError),
    #[error("Native PVM does not support opcode {0}")]
    UnsupportedOpcode(u8),
    #[error("Native PVM does not support this PVM terminator")]
    UnsupportedTerminator,
    #[error("Native PVM does not support 64-bit immediate 0x{0:016x} in the M4 lowering")]
    UnsupportedImmediate(u64),
    #[error("PVM memory address 0x{0:08x} is outside the bounded M4 memory fixture")]
    InvalidMemoryAddress(u32),
    #[error("PVM branch target block {0} is invalid")]
    InvalidBlock(u32),
    #[error("PVM branch displacement {0} is not representable")]
    InvalidBranchDisplacement(i64),
}

/// A direct OpenVM executable plus static lowering measurements used by M4.1.
pub struct NativePvmProgram {
    pub exe: VmExe<F>,
    pub pvm_instruction_count: usize,
    pub input_prefix_instruction_count: usize,
    pub pvm_core_instruction_count: usize,
    pub proof_envelope_instruction_count: usize,
    pub openvm_instruction_count: usize,
    pub pc_map: NativePcMap,
}

/// Bounded `PvmProgramV1 -> OpenVM Instructions` lowerer for the existing M4 fixtures.
#[derive(Clone, Debug, Default)]
pub struct NativePvmLowerer {
    pub register_map: NativePvmRegisterMap,
}

impl NativePvmLowerer {
    /// Lower an existing M4 `PvmProgramV1` directly into an OpenVM executable.
    pub fn lower(
        &self,
        program: &PvmProgramV1,
        output_register: u8,
    ) -> Result<NativePvmProgram, NativePvmError> {
        let translated = zk_jam_translation::translate(program)?;
        self.validate_fixture(program, output_register)?;

        let mut instructions = self.input_prefix()?;
        let input_prefix_instruction_count = instructions.len();
        let block_counts = translated
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| {
                self.block_instruction_count(block, index + 1 == translated.blocks.len())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut pc_map = Vec::with_capacity(translated.blocks.len());
        let mut pc = (instructions.len() as u32) * 4;
        for count in &block_counts {
            pc_map.push(pc);
            pc = pc
                .checked_add((*count as u32) * 4)
                .ok_or(NativePvmError::InvalidBranchDisplacement(i64::MAX))?;
        }
        let postlude_pc = pc;
        for (index, block) in translated.blocks.iter().enumerate() {
            self.emit_block(
                &mut instructions,
                block,
                index,
                &pc_map,
                &block_counts,
                postlude_pc,
            )?;
        }
        let pvm_core_instruction_count = instructions.len() - input_prefix_instruction_count;
        let proof_envelope_start = instructions.len();
        instructions.extend(self.commitment_and_public_values(program, output_register)?);
        let proof_envelope_instruction_count = instructions.len() - proof_envelope_start;

        let mut init_memory = BTreeMap::new();
        write_u32(
            &mut init_memory,
            RV32_REGISTER_AS,
            INPUT_PTR_SLOT,
            INPUT_BASE,
        );
        write_u32(&mut init_memory, RV32_REGISTER_AS, PUBLIC_INDEX_SLOT, 0);
        write_u32(&mut init_memory, RV32_REGISTER_AS, MEMORY_BASE_SLOT, 0);
        write_u32(&mut init_memory, RV32_REGISTER_AS, ZERO_SLOT, 0);
        write_u32(
            &mut init_memory,
            RV32_REGISTER_AS,
            SHA_INPUT_PTR_SLOT,
            SHA_INPUT_BASE,
        );
        write_u32(
            &mut init_memory,
            RV32_REGISTER_AS,
            SHA_STATE_PTR_SLOT,
            SHA_STATE_BASE,
        );
        write_u32(
            &mut init_memory,
            RV32_REGISTER_AS,
            SHA_DIGEST_PTR_SLOT,
            SHA_STATE_BASE,
        );
        self.initialize_sha_memory(&mut init_memory, program);

        Ok(NativePvmProgram {
            exe: VmExe::new(Program::from_instructions(&instructions))
                .with_init_memory(init_memory),
            pvm_instruction_count: program.instruction_count(),
            input_prefix_instruction_count,
            pvm_core_instruction_count,
            proof_envelope_instruction_count: input_prefix_instruction_count
                + proof_envelope_instruction_count,
            openvm_instruction_count: instructions.len(),
            pc_map: NativePcMap {
                pvm_block_to_openvm_pc: pc_map,
            },
        })
    }

    fn validate_fixture(
        &self,
        program: &PvmProgramV1,
        output_register: u8,
    ) -> Result<(), NativePvmError> {
        program
            .validate()
            .map_err(|_| NativePvmError::UnsupportedTerminator)?;
        self.register_map.pointer(output_register)?;
        if program.blocks.len() > 3 || program.z_pages as usize * 4096 < M3_MEMORY_BYTES {
            return Err(NativePvmError::InvalidMemoryAddress(u32::MAX));
        }
        for block in &program.blocks {
            for instruction in &block.instructions {
                match instruction.opcode {
                    opcode::LOAD_IMM_64 => {
                        if instruction.immediate.len() != 8 {
                            return Err(NativePvmError::UnsupportedOpcode(instruction.opcode));
                        }
                        let value =
                            u64::from_le_bytes(instruction.immediate[..8].try_into().unwrap());
                        if value > u32::MAX as u64 {
                            return Err(NativePvmError::UnsupportedImmediate(value));
                        }
                    }
                    opcode::LOAD_U32 | opcode::STORE_U32 => {
                        if instruction.immediate.len() != 4 {
                            return Err(NativePvmError::UnsupportedOpcode(instruction.opcode));
                        }
                        self.validate_memory(u32::from_le_bytes(
                            instruction.immediate[..4].try_into().unwrap(),
                        ))?;
                    }
                    opcode::STORE_IMM_U32 => {
                        if instruction.immediate.len() != 8 {
                            return Err(NativePvmError::UnsupportedOpcode(instruction.opcode));
                        }
                        self.validate_memory(u32::from_le_bytes(
                            instruction.immediate[..4].try_into().unwrap(),
                        ))?;
                    }
                    opcode::MOVE_REG
                    | opcode::ADD_32
                    | opcode::SUB_32
                    | opcode::MUL_32
                    | opcode::XOR
                    | opcode::BRANCH_EQ
                    | opcode::BRANCH_NE
                    | opcode::BRANCH_LT_U
                    | opcode::BRANCH_LT_S
                    | opcode::BRANCH_GE_U
                    | opcode::BRANCH_GE_S => {}
                    other => return Err(NativePvmError::UnsupportedOpcode(other)),
                }
            }
        }
        Ok(())
    }

    fn validate_memory(&self, address: u32) -> Result<(), NativePvmError> {
        if address < PVM_PROTECTED_BYTES
            || !address.is_multiple_of(4)
            || address >= PVM_PROTECTED_BYTES + M3_MEMORY_BYTES as u32
        {
            return Err(NativePvmError::InvalidMemoryAddress(address));
        }
        Ok(())
    }

    fn block_instruction_count(
        &self,
        block: &TranslatedBlockV1,
        is_last_block: bool,
    ) -> Result<usize, NativePvmError> {
        let mut count = 0;
        for instruction in &block.instructions {
            count += match instruction {
                GenericInstruction::LoadImm64 { .. } => 2,
                GenericInstruction::Move { .. }
                | GenericInstruction::Add32 { .. }
                | GenericInstruction::Sub32 { .. }
                | GenericInstruction::Mul32 { .. }
                | GenericInstruction::Xor { .. }
                | GenericInstruction::Load32 { .. }
                | GenericInstruction::Store32 { .. }
                | GenericInstruction::StoreImm32 { .. } => 1,
                GenericInstruction::Branch { .. } => 0,
                GenericInstruction::Add64 { .. }
                | GenericInstruction::Sub64 { .. }
                | GenericInstruction::Mul64 { .. }
                | GenericInstruction::Jump
                | GenericInstruction::Fallthrough
                | GenericInstruction::Halt
                | GenericInstruction::Trap(_)
                | GenericInstruction::HostCall { .. } => {
                    return Err(NativePvmError::UnsupportedOpcode(opcode::ECALLI))
                }
            };
        }
        count += match block.terminator {
            PvmTerminatorV1::Halt if is_last_block => 0,
            PvmTerminatorV1::Halt
            | PvmTerminatorV1::Fallthrough { .. }
            | PvmTerminatorV1::Jump { .. } => 1,
            PvmTerminatorV1::BrEqz { .. } => 2,
            PvmTerminatorV1::DJump | PvmTerminatorV1::Trap(_) => {
                return Err(NativePvmError::UnsupportedTerminator)
            }
        };
        Ok(count)
    }

    fn emit_block(
        &self,
        output: &mut Vec<Instruction<F>>,
        block: &TranslatedBlockV1,
        block_index: usize,
        pc_map: &[u32],
        block_counts: &[usize],
        postlude_pc: u32,
    ) -> Result<(), NativePvmError> {
        for instruction in &block.instructions {
            self.emit_instruction(output, instruction)?;
        }
        let current_pc = pc_map[block_index];
        let terminator_count = match block.terminator {
            PvmTerminatorV1::Halt if block_index + 1 == pc_map.len() => 0,
            PvmTerminatorV1::BrEqz { .. } => 2,
            _ => 1,
        };
        let terminator_pc = current_pc + (block_counts[block_index] - terminator_count) as u32 * 4;
        match &block.terminator {
            PvmTerminatorV1::Halt => {
                if block_index + 1 != pc_map.len() {
                    output.push(jump_instruction(branch_displacement(
                        postlude_pc,
                        terminator_pc + 4,
                    )?));
                }
            }
            PvmTerminatorV1::Fallthrough { next, .. } => {
                self.emit_jump(
                    output,
                    terminator_pc,
                    next.ok_or(NativePvmError::UnsupportedTerminator)?,
                    pc_map,
                )?;
            }
            PvmTerminatorV1::Jump { target, .. } => {
                self.emit_jump(
                    output,
                    terminator_pc,
                    target.ok_or(NativePvmError::UnsupportedTerminator)?,
                    pc_map,
                )?;
            }
            PvmTerminatorV1::BrEqz {
                true_block,
                false_block,
                ..
            } => {
                let (opcode, left, right) = block
                    .instructions
                    .last()
                    .and_then(|instruction| match instruction {
                        GenericInstruction::Branch {
                            opcode,
                            left,
                            right,
                        } => Some((*opcode, *left, *right)),
                        _ => None,
                    })
                    .ok_or(NativePvmError::UnsupportedTerminator)?;
                let true_block = true_block.ok_or(NativePvmError::UnsupportedTerminator)?;
                let false_block = false_block.ok_or(NativePvmError::UnsupportedTerminator)?;
                let true_pc = *pc_map
                    .get(true_block as usize)
                    .ok_or(NativePvmError::InvalidBlock(true_block))?;
                let false_pc = *pc_map
                    .get(false_block as usize)
                    .ok_or(NativePvmError::InvalidBlock(false_block))?;
                output.push(branch_instruction(
                    branch_opcode(opcode)?,
                    self.register_map.pointer(left)?,
                    self.register_map.pointer(right)?,
                    branch_displacement(true_pc, terminator_pc)?,
                ));
                output.push(jump_instruction(branch_displacement(
                    false_pc,
                    terminator_pc + 4,
                )?));
            }
            PvmTerminatorV1::DJump | PvmTerminatorV1::Trap(_) => {
                return Err(NativePvmError::UnsupportedTerminator)
            }
        }
        Ok(())
    }

    fn emit_jump(
        &self,
        output: &mut Vec<Instruction<F>>,
        current_pc: u32,
        target: u32,
        pc_map: &[u32],
    ) -> Result<(), NativePvmError> {
        let target_pc = *pc_map
            .get(target as usize)
            .ok_or(NativePvmError::InvalidBlock(target))?;
        output.push(jump_instruction(branch_displacement(
            target_pc,
            current_pc + 4,
        )?));
        Ok(())
    }

    fn emit_instruction(
        &self,
        output: &mut Vec<Instruction<F>>,
        instruction: &GenericInstruction,
    ) -> Result<(), NativePvmError> {
        match instruction {
            GenericInstruction::LoadImm64 { register, value } => {
                let value = u32::try_from(*value)
                    .map_err(|_| NativePvmError::UnsupportedImmediate(*value))?;
                output.extend(load_u32_instructions(
                    self.register_map.pointer(*register)?,
                    value,
                ));
            }
            GenericInstruction::Move {
                destination,
                source,
            } => output.push(base_alu_immediate(
                BASE_ALU_ADD,
                self.register_map.pointer(*destination)?,
                self.register_map.pointer(*source)?,
                0,
            )),
            GenericInstruction::Add32 {
                destination,
                left,
                right,
            } => output.push(base_alu_register(
                BASE_ALU_ADD,
                self.register_map.pointer(*destination)?,
                self.register_map.pointer(*left)?,
                self.register_map.pointer(*right)?,
            )),
            GenericInstruction::Sub32 {
                destination,
                left,
                right,
            } => output.push(base_alu_register(
                BASE_ALU_SUB,
                self.register_map.pointer(*destination)?,
                self.register_map.pointer(*left)?,
                self.register_map.pointer(*right)?,
            )),
            GenericInstruction::Mul32 {
                destination,
                left,
                right,
            } => output.push(mul_register(
                self.register_map.pointer(*destination)?,
                self.register_map.pointer(*left)?,
                self.register_map.pointer(*right)?,
            )),
            GenericInstruction::Xor {
                destination,
                left,
                right,
            } => output.push(base_alu_register(
                BASE_ALU_XOR,
                self.register_map.pointer(*destination)?,
                self.register_map.pointer(*left)?,
                self.register_map.pointer(*right)?,
            )),
            GenericInstruction::Load32 {
                destination,
                address,
            } => {
                self.validate_memory(*address)?;
                output.push(memory_instruction(
                    LOAD_STORE_LOADW,
                    self.register_map.pointer(*destination)?,
                    MEMORY_BASE_SLOT,
                    *address,
                    2,
                    1,
                ));
            }
            GenericInstruction::Store32 { source, address } => {
                self.validate_memory(*address)?;
                output.push(memory_instruction(
                    LOAD_STORE_STOREW,
                    self.register_map.pointer(*source)?,
                    MEMORY_BASE_SLOT,
                    *address,
                    2,
                    1,
                ));
            }
            GenericInstruction::StoreImm32 { address, value } => {
                self.validate_memory(*address)?;
                output.extend(load_u32_instructions(ZERO_SLOT, *value));
                output.push(memory_instruction(
                    LOAD_STORE_STOREW,
                    ZERO_SLOT,
                    MEMORY_BASE_SLOT,
                    *address,
                    2,
                    1,
                ));
            }
            GenericInstruction::Branch { .. } => {}
            GenericInstruction::Add64 { .. }
            | GenericInstruction::Sub64 { .. }
            | GenericInstruction::Mul64 { .. }
            | GenericInstruction::Jump
            | GenericInstruction::Fallthrough
            | GenericInstruction::Halt
            | GenericInstruction::Trap(_)
            | GenericInstruction::HostCall { .. } => {
                return Err(NativePvmError::UnsupportedOpcode(opcode::ECALLI))
            }
        }
        Ok(())
    }

    fn input_prefix(&self) -> Result<Vec<Instruction<F>>, NativePvmError> {
        let mut output = Vec::new();
        for (index, destination) in [self.register_map.pointer(1)?, self.register_map.pointer(2)?]
            .into_iter()
            .enumerate()
        {
            output.push(Instruction::phantom(
                PhantomDiscriminant(HINT_INPUT),
                F::ZERO,
                F::ZERO,
                0,
            ));
            output.push(hint_store(INPUT_PTR_SLOT));
            // StdIn::write serializes a u32 as a length word followed by the value word.
            output.push(hint_store(INPUT_PTR_SLOT));
            output.push(memory_instruction(
                LOAD_STORE_LOADW,
                destination,
                INPUT_PTR_SLOT,
                0,
                2,
                1,
            ));
            output.push(memory_instruction(
                LOAD_STORE_STOREW,
                destination,
                MEMORY_BASE_SLOT,
                INPUT_SHADOW_BASE + index as u32 * 4,
                2,
                1,
            ));
            if index == 0 {
                output.push(base_alu_immediate(
                    BASE_ALU_ADD,
                    INPUT_PTR_SLOT,
                    INPUT_PTR_SLOT,
                    4,
                ));
            }
        }
        Ok(output)
    }

    fn commitment_and_public_values(
        &self,
        program: &PvmProgramV1,
        output_register: u8,
    ) -> Result<Vec<Instruction<F>>, NativePvmError> {
        let mut output = Vec::new();
        for (index, _) in program_commitment(program).chunks_exact(4).enumerate() {
            output.push(memory_instruction(
                LOAD_STORE_LOADW,
                ZERO_SLOT,
                MEMORY_BASE_SLOT,
                PROGRAM_COMMITMENT_BASE + index as u32 * 4,
                2,
                1,
            ));
            output.push(reveal(ZERO_SLOT, PUBLIC_INDEX_SLOT, (index * 4) as u32));
            output.push(advance_public_index());
        }
        self.emit_input_commitment(&mut output)?;
        let output_pointer = self.register_map.pointer(output_register)?;
        for index in 0..1 {
            output.push(reveal(
                output_pointer,
                PUBLIC_INDEX_SLOT,
                64 + (index * 4) as u32,
            ));
            output.push(advance_public_index());
        }
        output.extend(load_u32_instructions(ZERO_SLOT, 0));
        for index in 1..16 {
            output.push(reveal(
                ZERO_SLOT,
                PUBLIC_INDEX_SLOT,
                64 + (index * 4) as u32,
            ));
            if index != 15 {
                output.push(advance_public_index());
            }
        }
        output.push(terminate());
        Ok(output)
    }

    fn emit_input_commitment(
        &self,
        output: &mut Vec<Instruction<F>>,
    ) -> Result<(), NativePvmError> {
        output.push(memory_instruction(
            LOAD_STORE_LOADW,
            32,
            MEMORY_BASE_SLOT,
            INPUT_SHADOW_BASE,
            2,
            1,
        ));
        output.push(memory_instruction(
            LOAD_STORE_LOADW,
            36,
            MEMORY_BASE_SLOT,
            INPUT_SHADOW_BASE + 4,
            2,
            1,
        ));
        for (source, base) in [(32u32, SHA_INPUT_BASE + 31), (36u32, SHA_INPUT_BASE + 35)] {
            for offset in 0..4 {
                output.push(memory_instruction(
                    LOAD_STORE_STOREB,
                    source,
                    MEMORY_BASE_SLOT,
                    base + offset,
                    2,
                    1,
                ));
                output.push(base_alu_immediate(SHIFT_SRL, source, source, 8));
            }
        }
        output.push(sha256_instruction(
            SHA_DIGEST_PTR_SLOT,
            SHA_STATE_PTR_SLOT,
            SHA_INPUT_PTR_SLOT,
        ));
        for index in 0..8 {
            output.push(memory_instruction(
                LOAD_STORE_LOADW,
                32,
                MEMORY_BASE_SLOT,
                SHA_STATE_BASE + index * 4,
                2,
                1,
            ));
            output.extend(byte_swap_word(32, 36, 40));
            output.push(reveal(36, PUBLIC_INDEX_SLOT, 32 + index * 4));
            output.push(advance_public_index());
        }
        Ok(())
    }

    fn initialize_sha_memory(&self, image: &mut BTreeMap<(u32, u32), u8>, program: &PvmProgramV1) {
        let mut message = [0u8; 64];
        let domain = b"zk-jam/input/v1";
        message[..domain.len()].copy_from_slice(domain);
        message[15..19].copy_from_slice(&1u32.to_le_bytes());
        message[19..27].copy_from_slice(&12u64.to_le_bytes());
        message[27..31].copy_from_slice(&2u32.to_le_bytes());
        message[39] = 0x80;
        message[56..64].copy_from_slice(&(39u64 * 8).to_be_bytes());
        for (offset, byte) in message.into_iter().enumerate() {
            image.insert((RV32_MEMORY_AS, SHA_INPUT_BASE + offset as u32), byte);
        }
        let initial_state: [u32; 8] = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
            0x5be0cd19,
        ];
        for (index, word) in initial_state.into_iter().enumerate() {
            write_u32(
                image,
                RV32_MEMORY_AS,
                SHA_STATE_BASE + index as u32 * 4,
                word,
            );
        }
        for (index, chunk) in program_commitment(program).chunks_exact(4).enumerate() {
            write_u32(
                image,
                RV32_MEMORY_AS,
                PROGRAM_COMMITMENT_BASE + index as u32 * 4,
                u32::from_le_bytes(chunk.try_into().unwrap()),
            );
        }
    }
}

fn branch_opcode(opcode: u8) -> Result<usize, NativePvmError> {
    Ok(match opcode {
        opcode::BRANCH_EQ => BRANCH_BEQ,
        opcode::BRANCH_NE => BRANCH_BNE,
        opcode::BRANCH_LT_U => BRANCH_BLTU,
        opcode::BRANCH_LT_S => BRANCH_BLT,
        opcode::BRANCH_GE_U => BRANCH_BGEU,
        opcode::BRANCH_GE_S => BRANCH_BGE,
        other => return Err(NativePvmError::UnsupportedOpcode(other)),
    })
}

fn branch_displacement(target_pc: u32, current_pc: u32) -> Result<isize, NativePvmError> {
    let displacement = target_pc as i64 - current_pc as i64;
    if displacement < i32::MIN as i64 || displacement > i32::MAX as i64 {
        return Err(NativePvmError::InvalidBranchDisplacement(displacement));
    }
    Ok(displacement as isize)
}

fn base_alu_register(opcode: usize, destination: u32, left: u32, right: u32) -> Instruction<F> {
    Instruction::large_from_isize(
        VmOpcode::from_usize(opcode),
        destination as isize,
        left as isize,
        right as isize,
        1,
        1,
        0,
        0,
    )
}

fn base_alu_immediate(
    opcode: usize,
    destination: u32,
    left: u32,
    immediate: i32,
) -> Instruction<F> {
    Instruction::large_from_isize(
        VmOpcode::from_usize(opcode),
        destination as isize,
        left as isize,
        imm24(immediate as i64),
        1,
        0,
        0,
        0,
    )
}

fn mul_register(destination: u32, left: u32, right: u32) -> Instruction<F> {
    Instruction::large_from_isize(
        VmOpcode::from_usize(MUL),
        destination as isize,
        left as isize,
        right as isize,
        1,
        0,
        0,
        0,
    )
}

fn load_u32_instructions(destination: u32, value: u32) -> Vec<Instruction<F>> {
    let upper = ((value as i64 + 0x800) >> 12) as i32;
    let lower = value as i32 - (upper << 12);
    vec![
        lui_instruction(destination, upper),
        base_alu_immediate(BASE_ALU_ADD, destination, destination, lower),
    ]
}

fn lui_instruction(destination: u32, immediate: i32) -> Instruction<F> {
    Instruction::large_from_isize(
        VmOpcode::from_usize(LUI),
        destination as isize,
        0,
        immediate as isize,
        1,
        0,
        1,
        0,
    )
}

fn memory_instruction(
    opcode: usize,
    data: u32,
    base: u32,
    immediate: u32,
    memory_as: u32,
    enabled: u32,
) -> Instruction<F> {
    Instruction::large_from_isize(
        VmOpcode::from_usize(opcode),
        data as isize,
        base as isize,
        immediate as isize,
        1,
        memory_as as isize,
        enabled as isize,
        0,
    )
}

fn hint_store(pointer_slot: u32) -> Instruction<F> {
    Instruction::large_from_isize(
        VmOpcode::from_usize(HINT_STOREW),
        0,
        pointer_slot as isize,
        0,
        1,
        RV32_MEMORY_AS as isize,
        0,
        0,
    )
}

fn sha256_instruction(destination: u32, state: u32, input: u32) -> Instruction<F> {
    Instruction::large_from_isize(
        VmOpcode::from_usize(SHA256),
        destination as isize,
        state as isize,
        input as isize,
        1,
        RV32_MEMORY_AS as isize,
        0,
        0,
    )
}

fn reveal(source: u32, index_slot: u32, _offset: u32) -> Instruction<F> {
    Instruction::large_from_isize(
        VmOpcode::from_usize(LOAD_STORE_STOREW),
        source as isize,
        index_slot as isize,
        0,
        1,
        3,
        1,
        0,
    )
}

fn advance_public_index() -> Instruction<F> {
    base_alu_immediate(BASE_ALU_ADD, PUBLIC_INDEX_SLOT, PUBLIC_INDEX_SLOT, 4)
}

fn jump_instruction(displacement: isize) -> Instruction<F> {
    Instruction::large_from_isize(VmOpcode::from_usize(JAL), 0, 0, displacement, 1, 0, 0, 0)
}

fn branch_instruction(opcode: usize, left: u32, right: u32, displacement: isize) -> Instruction<F> {
    Instruction::large_from_isize(
        VmOpcode::from_usize(opcode),
        left as isize,
        right as isize,
        displacement,
        1,
        1,
        0,
        0,
    )
}

fn imm24(value: i64) -> isize {
    (value & 0x00ff_ffff) as isize
}

fn terminate() -> Instruction<F> {
    Instruction::from_isize(SystemOpcode::TERMINATE.global_opcode(), 0, 0, 0, 0, 0)
}

fn byte_swap_word(source: u32, accumulator: u32, temporary: u32) -> Vec<Instruction<F>> {
    let mut output = vec![base_alu_immediate(BASE_ALU_ADD, accumulator, 0, 0)];
    for (left_shift, result_shift) in [(24, 24), (16, 16), (8, 8), (0, 0)] {
        output.push(base_alu_immediate(SHIFT_SLL, temporary, source, left_shift));
        output.push(base_alu_immediate(SHIFT_SRL, temporary, temporary, 24));
        output.push(base_alu_immediate(
            SHIFT_SLL,
            temporary,
            temporary,
            result_shift,
        ));
        output.push(base_alu_register(
            BASE_ALU_OR,
            accumulator,
            accumulator,
            temporary,
        ));
    }
    output
}

fn write_u32(image: &mut BTreeMap<(u32, u32), u8>, address_space: u32, address: u32, value: u32) {
    for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
        image.insert((address_space, address + offset as u32), byte);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openvm_stark_backend::p3_field::PrimeField32;
    use zk_jam_translation::{workload_program, M3Workload};

    fn assert_encoding(instruction: Instruction<F>, opcode: usize, operands: [u32; 7]) {
        assert_eq!(instruction.opcode, VmOpcode::from_usize(opcode));
        let actual = instruction
            .operands()
            .into_iter()
            .map(|value| value.as_canonical_u32())
            .collect::<Vec<_>>();
        assert_eq!(actual, operands);
    }

    #[test]
    fn instruction_helpers_match_pinned_openvm_encoding() {
        assert_encoding(
            base_alu_register(BASE_ALU_ADD, 4, 8, 12),
            BASE_ALU_ADD,
            [4, 8, 12, 1, 1, 0, 0],
        );
        assert_encoding(
            base_alu_register(BASE_ALU_SUB, 4, 8, 12),
            BASE_ALU_SUB,
            [4, 8, 12, 1, 1, 0, 0],
        );
        assert_encoding(
            base_alu_register(BASE_ALU_XOR, 4, 8, 12),
            BASE_ALU_XOR,
            [4, 8, 12, 1, 1, 0, 0],
        );
        assert_encoding(
            base_alu_register(BASE_ALU_OR, 4, 8, 12),
            BASE_ALU_OR,
            [4, 8, 12, 1, 1, 0, 0],
        );
        assert_encoding(mul_register(4, 8, 12), MUL, [4, 8, 12, 1, 0, 0, 0]);
        assert_encoding(
            base_alu_immediate(BASE_ALU_ADD, 4, 8, 0x123),
            BASE_ALU_ADD,
            [4, 8, 0x123, 1, 0, 0, 0],
        );
        assert_encoding(
            lui_instruction(4, 0x12345),
            LUI,
            [4, 0, 0x12345, 1, 0, 1, 0],
        );
        assert_encoding(
            memory_instruction(LOAD_STORE_LOADW, 4, 60, 0x1234, 2, 1),
            LOAD_STORE_LOADW,
            [4, 60, 0x1234, 1, 2, 1, 0],
        );
        assert_encoding(
            memory_instruction(LOAD_STORE_STOREW, 4, 60, 0x1234, 2, 1),
            LOAD_STORE_STOREW,
            [4, 60, 0x1234, 1, 2, 1, 0],
        );
        assert_encoding(
            memory_instruction(LOAD_STORE_STOREB, 4, 60, 0x1234, 2, 1),
            LOAD_STORE_STOREB,
            [4, 60, 0x1234, 1, 2, 1, 0],
        );
        for opcode in [
            BRANCH_BEQ,
            BRANCH_BNE,
            BRANCH_BLT,
            BRANCH_BLTU,
            BRANCH_BGE,
            BRANCH_BGEU,
        ] {
            assert_encoding(
                branch_instruction(opcode, 4, 8, 16),
                opcode,
                [4, 8, 16, 1, 1, 0, 0],
            );
        }
        assert_encoding(jump_instruction(16), JAL, [0, 0, 16, 1, 0, 0, 0]);
        assert_encoding(
            hint_store(INPUT_PTR_SLOT),
            HINT_STOREW,
            [0, 52, 0, 1, 2, 0, 0],
        );
        assert_encoding(
            sha256_instruction(SHA_DIGEST_PTR_SLOT, SHA_STATE_PTR_SLOT, SHA_INPUT_PTR_SLOT),
            SHA256,
            [76, 72, 68, 1, 2, 0, 0],
        );
        assert_encoding(
            reveal(4, PUBLIC_INDEX_SLOT, 0),
            LOAD_STORE_STOREW,
            [4, 56, 0, 1, 3, 1, 0],
        );
    }

    #[test]
    fn register_mapping_is_fixed_and_deterministic() {
        let first = NativePvmRegisterMap::default();
        let second = NativePvmRegisterMap::default();
        assert_eq!(first, second);
        assert_eq!(first.pointer(0), Ok(0));
        assert_eq!(first.pointer(12), Ok(48));
        assert_eq!(first.pointer(13), Err(NativePvmError::InvalidRegister(13)));
    }

    #[test]
    fn lowers_all_existing_m4_fixture_programs() {
        for (workload, output_register) in [
            (M3Workload::Arithmetic, 7),
            (M3Workload::BranchTrue, 5),
            (M3Workload::Memory16K, 2),
        ] {
            let source = workload_program(workload);
            let program = NativePvmLowerer::default()
                .lower(&source, output_register)
                .unwrap();
            assert_eq!(
                program.pc_map.pvm_block_to_openvm_pc.len(),
                source.blocks.len()
            );
            assert!(program.openvm_instruction_count > program.pvm_instruction_count);
            assert!(program.exe.program.num_defined_instructions() > 0);
        }
    }
}
