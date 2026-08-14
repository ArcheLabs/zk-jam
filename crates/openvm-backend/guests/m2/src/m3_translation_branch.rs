//! Statically emitted RV32IM workload for the M3 translated branch case.
use openvm::io::reveal_u32;

pub fn main() {
    let r1 = 21u32;
    let r2 = 8u32;
    let r3 = if r1 >= r2 {
        r1.wrapping_sub(r2)
    } else {
        r2.wrapping_sub(r1)
    };
    let r4 = if r1 >= r2 { 7 } else { 11 };
    reveal_u32(r3.wrapping_mul(r4), 0);
}
