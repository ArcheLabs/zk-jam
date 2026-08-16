use zk_jam_openvm_backend::{M2Benchmark, M2Input};
use zk_jam_refine_interface::{
    PvmBlockV1, PvmInstructionV1, PvmProgramV1, PvmTerminatorV1, RegisterOperandsV1,
    PVM_PROGRAM_FORMAT_V1,
};
use zk_jam_translation::{
    opcode, workload_program, M3Workload, M3_MEMORY_BYTES, PVM_PROTECTED_BYTES,
};

#[derive(Clone)]
pub struct CostWorkload {
    pub name: &'static str,
    pub pattern: &'static str,
    pub program: PvmProgramV1,
    pub input: M2Input,
    pub output_register: u8,
    pub benchmark: M2Benchmark,
}

pub fn all() -> Vec<CostWorkload> {
    vec![
        existing(
            "arithmetic",
            "arithmetic",
            M3Workload::Arithmetic,
            M2Input::arithmetic(7, 9),
            7,
            M2Benchmark::M4NativeArithmetic,
        ),
        existing(
            "branch-true",
            "branch_taken",
            M3Workload::BranchTrue,
            M2Input::branch(21, 8),
            5,
            M2Benchmark::M4NativeBranch,
        ),
        existing(
            "memory-16384",
            "memory_mixed",
            M3Workload::Memory16K,
            M2Input {
                a: 0x1234_5678,
                b: M3_MEMORY_BYTES as u32,
            },
            2,
            M2Benchmark::M4NativeMemory16K,
        ),
        small(
            "arithmetic-dependent",
            "alu_dependent",
            vec![
                inst(opcode::ADD_32, 3, 1, 2, vec![]),
                inst(opcode::SUB_32, 4, 3, 1, vec![]),
                inst(opcode::MUL_32, 5, 4, 2, vec![]),
                inst(opcode::XOR, 6, 5, 3, vec![]),
            ],
            M2Input::arithmetic(7, 9),
            6,
            [4; 32],
        ),
        small(
            "arithmetic-independent",
            "alu_independent",
            vec![imm(3, 3), imm(4, 5), inst(opcode::ADD_32, 5, 3, 4, vec![])],
            M2Input::arithmetic(7, 9),
            5,
            [5; 32],
        ),
        small(
            "mul-heavy",
            "mul",
            vec![
                imm(3, 3),
                imm(4, 5),
                inst(opcode::MUL_32, 5, 3, 4, vec![]),
                inst(opcode::MUL_32, 6, 5, 4, vec![]),
                inst(opcode::MUL_32, 7, 6, 3, vec![]),
            ],
            M2Input::arithmetic(7, 9),
            7,
            [6; 32],
        ),
        existing(
            "branch-false",
            "branch_not_taken",
            M3Workload::BranchTrue,
            M2Input::branch(8, 21),
            5,
            M2Benchmark::M4NativeBranch,
        ),
        small(
            "load-heavy",
            "load",
            vec![
                load(3, 0x100),
                load(4, 0x104),
                load(5, 0x108),
                inst(opcode::ADD_32, 2, 3, 4, vec![]),
            ],
            M2Input::arithmetic(7, 9),
            2,
            [7; 32],
        ),
        existing(
            "store-heavy",
            "store",
            M3Workload::Memory16K,
            M2Input {
                a: 1,
                b: M3_MEMORY_BYTES as u32,
            },
            2,
            M2Benchmark::M4NativeMemory16K,
        ),
        existing(
            "mixed-arithmetic-memory",
            "mixed",
            M3Workload::Memory16K,
            M2Input {
                a: 2,
                b: M3_MEMORY_BYTES as u32,
            },
            2,
            M2Benchmark::M4NativeMemory16K,
        ),
    ]
}

fn existing(
    name: &'static str,
    pattern: &'static str,
    workload: M3Workload,
    input: M2Input,
    output_register: u8,
    benchmark: M2Benchmark,
) -> CostWorkload {
    CostWorkload {
        name,
        pattern,
        program: workload_program(workload),
        input,
        output_register,
        benchmark,
    }
}

fn small(
    name: &'static str,
    pattern: &'static str,
    instructions: Vec<PvmInstructionV1>,
    input: M2Input,
    output_register: u8,
    hash: [u8; 32],
) -> CostWorkload {
    CostWorkload {
        name,
        pattern,
        program: program(hash, instructions),
        input,
        output_register,
        benchmark: M2Benchmark::M4NativeArithmetic,
    }
}

fn program(code_hash: [u8; 32], instructions: Vec<PvmInstructionV1>) -> PvmProgramV1 {
    PvmProgramV1 {
        format_version: PVM_PROGRAM_FORMAT_V1,
        code_hash,
        o_blob: Vec::new(),
        w_blob: Vec::new(),
        z_pages: 6,
        s_bytes: 0,
        blocks: vec![PvmBlockV1 {
            entry_pc: 0,
            instructions,
            terminator: PvmTerminatorV1::Halt,
        }],
        jump_table: Vec::new(),
        c_blob: Vec::new(),
    }
}

fn inst(op: u8, rd: u8, ra: u8, rb: u8, immediate: Vec<u8>) -> PvmInstructionV1 {
    PvmInstructionV1 {
        pc: 0,
        opcode: op,
        registers: RegisterOperandsV1 { rd, ra, rb },
        immediate,
        pc_delta: 0,
    }
}

fn imm(register: u8, value: u64) -> PvmInstructionV1 {
    inst(
        opcode::LOAD_IMM_64,
        0,
        register,
        0,
        value.to_le_bytes().to_vec(),
    )
}
fn load(register: u8, offset: u32) -> PvmInstructionV1 {
    inst(
        opcode::LOAD_U32,
        register,
        0,
        0,
        (PVM_PROTECTED_BYTES + offset).to_le_bytes().to_vec(),
    )
}
