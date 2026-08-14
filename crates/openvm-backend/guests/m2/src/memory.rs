use openvm::io::{read, reveal_u32};

pub fn main() {
    let seed: u32 = read();
    let requested_bytes = read::<u32>() as usize;
    let words = requested_bytes / core::mem::size_of::<u32>();
    let mut buffer = vec![0u32; words];
    let mut state = seed;

    for (index, slot) in buffer.iter_mut().enumerate() {
        state = state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223)
            ^ index as u32;
        *slot = state;
    }

    let reduction = buffer.iter().fold(seed, |acc, value| acc.wrapping_add(*value));
    reveal_u32(reduction, 0);
}
