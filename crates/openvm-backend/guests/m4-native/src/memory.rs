mod common;

use common::{reveal_statement, MEMORY_PROGRAM_COMMITMENT};
use openvm::io::read;

openvm::entry!(main);

pub fn main() {
    let seed: u32 = read();
    let requested_bytes: u32 = read();
    let words = (requested_bytes / core::mem::size_of::<u32>() as u32) as usize;
    let mut memory = [0u32; 4096];
    let mut state = seed;
    for (index, slot) in memory[..words].iter_mut().enumerate() {
        state = state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223)
            ^ index as u32;
        *slot = state;
    }
    let output = memory[..words]
        .iter()
        .fold(seed, |acc, value| acc.wrapping_add(*value));
    reveal_statement(MEMORY_PROGRAM_COMMITMENT, seed, requested_bytes, output);
}
