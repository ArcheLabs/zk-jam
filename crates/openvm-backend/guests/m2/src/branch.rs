use openvm::io::{read, reveal_u32};

pub fn main() {
    let a: u32 = read();
    let b: u32 = read();
    let result = if a > b {
        a.wrapping_sub(b).wrapping_mul(7)
    } else {
        b.wrapping_sub(a).wrapping_mul(11)
    };
    reveal_u32(result, 0);
}
