use openvm::io::{read_u32, reveal_u32};

pub fn main() {
    let a = read_u32();
    let b = read_u32();
    let result = if a > b {
        a.wrapping_sub(b).wrapping_mul(7)
    } else {
        b.wrapping_sub(a).wrapping_mul(11)
    };
    reveal_u32(result, 0);
}
