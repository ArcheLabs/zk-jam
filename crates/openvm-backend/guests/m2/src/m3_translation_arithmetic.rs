//! Statically emitted RV32IM workload for the M3 translated arithmetic case.
//! This is a program-specific lowering, not a runtime PVM interpreter.
use openvm::io::reveal_u32;

pub fn main() {
    let r1 = 7u32;
    let r2 = 9u32;
    let r3 = r1.wrapping_add(r2);
    let r4 = 3u32;
    let r5 = r3.wrapping_mul(r4);
    let r6 = 0xA5A5_5A5Au32;
    let r7 = r5 ^ r6;
    reveal_u32(r7, 0);
}
