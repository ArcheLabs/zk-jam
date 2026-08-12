use openvm::io::{read_u32, reveal_u32};

const FIXED_XOR: u32 = 0xA5A5_5A5A;

pub fn main() {
    let a = read_u32();
    let b = read_u32();
    let x = a.wrapping_add(b);
    let y = x.wrapping_mul(3);
    let z = y ^ FIXED_XOR;
    reveal_u32(z, 0);
}
