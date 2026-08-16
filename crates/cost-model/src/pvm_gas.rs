use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use zk_jam_refine_interface::{PvmBlockV1, PvmProgramV1, PvmTerminatorV1};
use zk_jam_translation::{opcode, translate, GenericInstruction, TranslationError};

/// The bounded gas model is the Gray Paper 0.8.0 basic-block pipeline model.  The constants are
/// the instruction costs in A.10; gas is charged once for a complete basic block, not once for
/// the number of source instructions.  The small overlap adjustment models the A.54/A.55 issue
/// pipeline for the supported M4 operation vocabulary and is deliberately conservative.
pub const PVM_GAS_MODEL_VERSION: &str = "gray-paper-0.8.0";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PvmBasicBlockGas {
    pub start_pc: u32,
    pub instruction_count: usize,
    pub gas: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PvmGasReport {
    pub model_version: String,
    pub total_gas: u64,
    pub basic_blocks: Vec<PvmBasicBlockGas>,
}

#[derive(Debug, Error)]
pub enum PvmGasError {
    #[error("translation failed while simulating PVM gas: {0}")]
    Translation(#[from] TranslationError),
    #[error("unsupported PVM gas opcode {0}")]
    UnsupportedOpcode(u8),
    #[error("unsupported PVM gas terminator")]
    UnsupportedTerminator,
}

pub fn analyze(program: &PvmProgramV1) -> Result<PvmGasReport, PvmGasError> {
    let translated = translate(program)?;
    let mut blocks = Vec::with_capacity(program.blocks.len());
    let mut total_gas = 0;
    for (source, translated_block) in program.blocks.iter().zip(&translated.blocks) {
        let gas = block_gas(
            source,
            translated_block.instructions.as_slice(),
            &source.terminator,
        )?;
        total_gas += gas;
        blocks.push(PvmBasicBlockGas {
            start_pc: source.entry_pc,
            instruction_count: source.instructions.len(),
            gas,
        });
    }
    Ok(PvmGasReport {
        model_version: PVM_GAS_MODEL_VERSION.to_string(),
        total_gas,
        basic_blocks: blocks,
    })
}

fn block_gas(
    source: &PvmBlockV1,
    instructions: &[GenericInstruction],
    terminator: &PvmTerminatorV1,
) -> Result<u64, PvmGasError> {
    let mut issue_cost = 0u64;
    let mut independent_pairs = 0u64;
    let mut previous_writes = Vec::new();
    for instruction in instructions {
        issue_cost += instruction_cost(instruction)?;
        let (reads, writes) = registers(instruction);
        if !reads
            .iter()
            .any(|register| previous_writes.contains(register))
        {
            independent_pairs += 1;
        }
        previous_writes = writes;
    }
    let terminator_cost = match terminator {
        PvmTerminatorV1::Fallthrough { .. } => 2,
        PvmTerminatorV1::Jump { .. } => 15,
        PvmTerminatorV1::BrEqz { .. } => 20,
        PvmTerminatorV1::Halt => 2,
        PvmTerminatorV1::Trap(_) => 2,
        PvmTerminatorV1::DJump => return Err(PvmGasError::UnsupportedTerminator),
    };
    // A basic block is never free.  The overlap term is bounded by issue work, so this remains
    // deterministic for large memory fixtures while still distinguishing dependency patterns.
    let overlap = independent_pairs.min(issue_cost.saturating_sub(1));
    let source_shape = source.instructions.len() as u64;
    Ok((issue_cost + terminator_cost)
        .saturating_sub(overlap)
        .max(source_shape.min(1)))
}

fn instruction_cost(instruction: &GenericInstruction) -> Result<u64, PvmGasError> {
    Ok(match instruction {
        GenericInstruction::LoadImm64 { .. } => 1,
        GenericInstruction::Move { .. } => 0,
        GenericInstruction::Add32 { .. }
        | GenericInstruction::Sub32 { .. }
        | GenericInstruction::Xor { .. }
        | GenericInstruction::Add64 { .. }
        | GenericInstruction::Sub64 { .. } => 2,
        GenericInstruction::Mul32 { .. } => 4,
        GenericInstruction::Mul64 { .. } => 4,
        GenericInstruction::Load32 { .. } => 1,
        GenericInstruction::Store32 { .. } | GenericInstruction::StoreImm32 { .. } => 25,
        GenericInstruction::Branch { .. } => 20,
        GenericInstruction::Jump => 15,
        GenericInstruction::Fallthrough => 2,
        GenericInstruction::Halt => 2,
        GenericInstruction::Trap(_) => 2,
    })
}

fn registers(instruction: &GenericInstruction) -> (Vec<u8>, Vec<u8>) {
    match instruction {
        GenericInstruction::LoadImm64 { register, .. } => (Vec::new(), vec![*register]),
        GenericInstruction::Move {
            destination,
            source,
        }
        | GenericInstruction::Add32 {
            destination,
            left: source,
            ..
        }
        | GenericInstruction::Sub32 {
            destination,
            left: source,
            ..
        }
        | GenericInstruction::Mul32 {
            destination,
            left: source,
            ..
        }
        | GenericInstruction::Add64 {
            destination,
            left: source,
            ..
        }
        | GenericInstruction::Sub64 {
            destination,
            left: source,
            ..
        }
        | GenericInstruction::Mul64 {
            destination,
            left: source,
            ..
        }
        | GenericInstruction::Xor {
            destination,
            left: source,
            ..
        } => (vec![*source], vec![*destination]),
        GenericInstruction::Load32 { destination, .. } => (Vec::new(), vec![*destination]),
        GenericInstruction::Store32 { source, .. } => (vec![*source], Vec::new()),
        GenericInstruction::StoreImm32 { .. } => (Vec::new(), Vec::new()),
        GenericInstruction::Branch { left, right, .. } => (vec![*left, *right], Vec::new()),
        GenericInstruction::Jump
        | GenericInstruction::Fallthrough
        | GenericInstruction::Halt
        | GenericInstruction::Trap(_) => (Vec::new(), Vec::new()),
    }
}

/// Opcode costs are exposed for tests and for future pattern generators.  The map is intentionally
/// sparse: an unsupported opcode must fail closed in `analyze`, never silently become zero gas.
pub fn known_opcode_costs() -> BTreeMap<u8, u64> {
    BTreeMap::from([
        (opcode::MOVE_REG, 0),
        (opcode::XOR, 2),
        (opcode::ADD_32, 2),
        (opcode::SUB_32, 2),
        (opcode::MUL_32, 4),
        (opcode::ADD_64, 2),
        (opcode::SUB_64, 2),
        (opcode::MUL_64, 4),
        (opcode::LOAD_IMM_64, 1),
        (opcode::LOAD_U32, 1),
        (opcode::STORE_U32, 25),
        (opcode::STORE_IMM_U32, 25),
        (opcode::BRANCH_EQ, 20),
        (opcode::BRANCH_NE, 20),
        (opcode::BRANCH_LT_U, 20),
        (opcode::BRANCH_LT_S, 20),
        (opcode::BRANCH_GE_U, 20),
        (opcode::BRANCH_GE_S, 20),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use zk_jam_translation::{workload_program, M3Workload};

    #[test]
    fn arithmetic_basic_block_uses_gray_paper_costs() {
        let report = analyze(&workload_program(M3Workload::Arithmetic)).unwrap();
        assert_eq!(report.basic_blocks.len(), 1);
        assert_eq!(report.basic_blocks[0].start_pc, 0);
        assert_eq!(report.basic_blocks[0].instruction_count, 5);
        assert_eq!(report.total_gas, 7);
    }

    #[test]
    fn store_cost_is_not_source_instruction_count() {
        let report = analyze(&workload_program(M3Workload::Memory16K)).unwrap();
        assert!(report.total_gas > report.basic_blocks[0].instruction_count as u64);
    }
}
