//! Statically emitted RV32IM workload for the M3 translated sparse-memory case.
//! The fixed array represents the generic RV32 memory accesses emitted by Translation.
use openvm::io::reveal_u32;

pub fn main() {
    let seed = 0x1234_5678u32;
    let mut memory = [0u32; 4096];
    let mut state = seed;
    for (index, slot) in memory.iter_mut().enumerate() {
        state = state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223)
            ^ index as u32;
        *slot = state;
    }
    let mut reduction = seed;
    for value in memory {
        reduction = reduction.wrapping_add(value);
    }
    reveal_u32(reduction, 0);
}
