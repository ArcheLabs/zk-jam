use openvm::io::{read, reveal_u32};

const FIXED_XOR: u32 = 0xA5A5_5A5A;

pub fn main() {
    let a: u32 = read();
    let b: u32 = read();
    let x = a.wrapping_add(b);
    let y = x.wrapping_mul(3);
    let z = y ^ FIXED_XOR;
    reveal_u32(z, 0);
}
