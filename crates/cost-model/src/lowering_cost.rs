use std::collections::BTreeMap;

use openvm_sdk::openvm_circuit::arch::instructions::instruction::Instruction;
use serde::{Deserialize, Serialize};
use zk_jam_openvm_backend::native_pvm::NativePvmProgram;

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
const TERMINATE: usize = 1;
const PHANTOM: usize = 0xdead;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenVmStaticCost {
    pub total_instructions: usize,
    pub by_opcode: BTreeMap<String, usize>,
    pub by_category: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoweringCostReport {
    pub pvm_core: OpenVmStaticCost,
    pub proof_envelope: OpenVmStaticCost,
    pub total: OpenVmStaticCost,
}

pub fn analyze(program: &NativePvmProgram) -> LoweringCostReport {
    let instructions = &program.exe.program.instructions_and_debug_infos;
    let envelope_start = program
        .input_prefix_instruction_count
        .saturating_add(program.pvm_core_instruction_count);
    let core_start = program
        .input_prefix_instruction_count
        .min(instructions.len());
    let core_end = envelope_start.min(instructions.len());
    let core = summarize(instructions[core_start..core_end].iter());
    let prefix_end = program
        .input_prefix_instruction_count
        .min(instructions.len());
    let postlude = &instructions[core_end..instructions.len()];
    let envelope = summarize(instructions[..prefix_end].iter().chain(postlude.iter()));
    let total = summarize(instructions.iter());
    LoweringCostReport {
        pvm_core: core,
        proof_envelope: envelope,
        total,
    }
}

fn summarize<'a, F: 'a, D: 'a, I>(instructions: I) -> OpenVmStaticCost
where
    I: IntoIterator<Item = &'a Option<(Instruction<F>, Option<D>)>>,
{
    let mut by_opcode = BTreeMap::new();
    let mut by_category = BTreeMap::new();
    for category in [
        "alu",
        "mul",
        "branch",
        "load_store",
        "memory",
        "system",
        "hint",
        "sha256",
        "other",
    ] {
        by_category.insert(category.to_string(), 0);
    }
    let mut total_instructions = 0;
    for entry in instructions {
        let Some((instruction, _)) = entry else {
            continue;
        };
        total_instructions += 1;
        let opcode = instruction.opcode.as_usize();
        *by_opcode.entry(format!("0x{opcode:03x}")).or_insert(0) += 1;
        let category = category(opcode);
        *by_category.entry(category.to_string()).or_insert(0) += 1;
    }
    OpenVmStaticCost {
        total_instructions,
        by_opcode,
        by_category,
    }
}

fn category(opcode: usize) -> &'static str {
    match opcode {
        BASE_ALU_ADD | BASE_ALU_SUB | BASE_ALU_XOR | BASE_ALU_OR | SHIFT_SLL | SHIFT_SRL => "alu",
        MUL => "mul",
        BRANCH_BEQ | BRANCH_BNE | BRANCH_BLT | BRANCH_BLTU | BRANCH_BGE | BRANCH_BGEU | JAL => {
            "branch"
        }
        LOAD_STORE_LOADW | LOAD_STORE_STOREW | LOAD_STORE_STOREB => "load_store",
        LUI => "memory",
        HINT_STOREW | PHANTOM => "hint",
        SHA256 => "sha256",
        TERMINATE => "system",
        _ => "other",
    }
}

pub fn validate_count(program: &NativePvmProgram) -> bool {
    program.openvm_instruction_count == program.exe.program.instructions_and_debug_infos.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zk_jam_openvm_backend::native_pvm::NativePvmLowerer;
    use zk_jam_translation::{workload_program, M3Workload};

    #[test]
    fn actual_vm_exe_count_is_the_static_count() {
        let lowered = NativePvmLowerer::default()
            .lower(&workload_program(M3Workload::Arithmetic), 7)
            .unwrap();
        let report = analyze(&lowered);
        assert!(validate_count(&lowered));
        assert_eq!(
            report.total.total_instructions,
            lowered.openvm_instruction_count
        );
        assert_eq!(
            report.pvm_core.total_instructions + report.proof_envelope.total_instructions,
            report.total.total_instructions
        );
        assert!(report.total.by_category.contains_key("mul"));
    }
}
