//! M4.1 direct OpenVM backend injection spike.
//!
//! M4.1 does not implement a new PVM proving circuit. It tests whether existing PVM semantics can
//! bypass the Rust/RV32/ELF/OpenVM transpiler frontend and be lowered directly into the OpenVM
//! executable representation while reusing the existing OpenVM execution and proving backend.
//!
//! This module intentionally starts with a tiny arithmetic probe. It is the Phase 0 feasibility
//! gate before adding normalized PVM lowering.

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

const BASE_ALU_ADD: usize = 0x200;
const LOAD_STORE_LOADW: usize = 0x210;
const LOAD_STORE_STOREW: usize = 0x213;
const HINT_STOREW: usize = 0x260;
const HINT_INPUT: u16 = 0x20;

/// Fixed PVM register to RV32 register-memory pointer mapping.
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
}

/// A direct OpenVM executable plus the static lowering measurements used by M4.1.
pub struct NativePvmProgram {
    pub exe: VmExe<F>,
    pub pvm_instruction_count: usize,
    pub openvm_instruction_count: usize,
    pub pc_map: NativePcMap,
}

/// Phase 0 direct-injection builder.
#[derive(Clone, Debug, Default)]
pub struct NativePvmLowerer {
    pub register_map: NativePvmRegisterMap,
}

impl NativePvmLowerer {
    /// Build `r3 = r1 + r2; reveal r3; halt` directly as OpenVM instructions.
    ///
    /// Inputs are read through the existing OpenVM hint/public-value machinery. No Rust guest,
    /// LLVM, RV32 ELF, or `convert_to_exe` call is involved in this path.
    pub fn phase0_arithmetic_probe(&self) -> Result<NativePvmProgram, NativePvmError> {
        let input_ptr_slot = 52;
        let public_index_slot = 56;
        let r1 = self.register_map.pointer(1)?;
        let r2 = self.register_map.pointer(2)?;
        let r3 = self.register_map.pointer(3)?;
        let mut instructions = Vec::new();

        for destination in [r1, r2] {
            instructions.push(Instruction::phantom(
                PhantomDiscriminant(HINT_INPUT),
                F::ZERO,
                F::ZERO,
                0,
            ));
            instructions.push(Instruction::from_isize(
                VmOpcode::from_usize(HINT_STOREW),
                0,
                input_ptr_slot as isize,
                0,
                RV32_REGISTER_AS as isize,
                RV32_MEMORY_AS as isize,
            ));
            // `StdIn::write` exposes a serialized byte length before the payload, matching the
            // existing `openvm::io::read` reader. Consume that framing word before the value.
            instructions.push(Instruction::from_isize(
                VmOpcode::from_usize(HINT_STOREW),
                0,
                input_ptr_slot as isize,
                0,
                RV32_REGISTER_AS as isize,
                RV32_MEMORY_AS as isize,
            ));
            instructions.push(Instruction::large_from_isize(
                VmOpcode::from_usize(LOAD_STORE_LOADW),
                destination as isize,
                input_ptr_slot as isize,
                0,
                RV32_REGISTER_AS as isize,
                RV32_MEMORY_AS as isize,
                1,
                0,
            ));
        }

        instructions.push(Instruction::large_from_isize(
            VmOpcode::from_usize(BASE_ALU_ADD),
            r3 as isize,
            r1 as isize,
            r2 as isize,
            RV32_REGISTER_AS as isize,
            RV32_REGISTER_AS as isize,
            0,
            0,
        ));
        instructions.push(Instruction::large_from_isize(
            VmOpcode::from_usize(LOAD_STORE_STOREW),
            r3 as isize,
            public_index_slot as isize,
            0,
            RV32_REGISTER_AS as isize,
            3,
            1,
            0,
        ));
        instructions.push(Instruction::from_isize(
            SystemOpcode::TERMINATE.global_opcode(),
            0,
            0,
            0,
            0,
            0,
        ));

        let mut init_memory = BTreeMap::new();
        write_u32(&mut init_memory, RV32_REGISTER_AS, input_ptr_slot, 0x1000);
        write_u32(&mut init_memory, RV32_REGISTER_AS, public_index_slot, 0);

        Ok(NativePvmProgram {
            exe: VmExe::new(Program::from_instructions(&instructions))
                .with_init_memory(init_memory),
            pvm_instruction_count: 1,
            openvm_instruction_count: instructions.len(),
            pc_map: NativePcMap {
                pvm_block_to_openvm_pc: vec![0],
            },
        })
    }
}

fn write_u32(image: &mut BTreeMap<(u32, u32), u8>, address_space: u32, address: u32, value: u32) {
    for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
        image.insert((address_space, address + offset as u32), byte);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn phase0_constructs_direct_vm_exe_without_frontend_artifacts() {
        let program = NativePvmLowerer::default()
            .phase0_arithmetic_probe()
            .unwrap();
        assert_eq!(program.pvm_instruction_count, 1);
        assert_eq!(program.pc_map.pvm_block_to_openvm_pc, vec![0]);
        assert_eq!(program.exe.pc_start, 0);
        assert_eq!(program.exe.program.num_defined_instructions(), 11);
        assert_eq!(program.exe.init_memory.len(), 8);
    }
}
