mod common;

use common::{reveal_statement, ARITHMETIC_PROGRAM_COMMITMENT};
use openvm::io::read;

openvm::entry!(main);

pub fn main() {
    let a: u32 = read();
    let b: u32 = read();
    let output = a.wrapping_add(b).wrapping_mul(3) ^ 0xA5A5_5A5A;
    reveal_statement(ARITHMETIC_PROGRAM_COMMITMENT, a, b, output);
}
