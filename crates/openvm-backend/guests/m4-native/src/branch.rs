mod common;

use common::{reveal_statement, BRANCH_PROGRAM_COMMITMENT};
use openvm::io::read;

openvm::entry!(main);

pub fn main() {
    let a: u32 = read();
    let b: u32 = read();
    let output = if a >= b {
        a.wrapping_sub(b).wrapping_mul(7)
    } else {
        b.wrapping_sub(a).wrapping_mul(11)
    };
    reveal_statement(BRANCH_PROGRAM_COMMITMENT, a, b, output);
}
