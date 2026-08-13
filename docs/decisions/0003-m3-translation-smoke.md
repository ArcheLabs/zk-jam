# ADR 0003: M3 translation smoke boundary

Status: accepted for M3

M3 establishes a public paired benchmark for the first three translation
workloads: arithmetic, a branch-taken path, and deterministic 16 KiB memory.
The normalized `PvmProgramV1` fixtures are lowered to static RV32IM/OpenVM
guest emissions. The guest does not interpret PVM instructions at proving
time, which keeps the benchmark measuring emitted OpenVM proving rather than a
runtime interpreter.

The report pins the Jambda adapter reference at
`b850a458fa00da81e80be4cc84ddd7d2222f1edc` and pins OpenVM at `2.0.1` revision
`b820b25baab6c5d9b055f64e0286b6b1058e707c`. The checked-in M3 fixtures are the
client-independent normalized boundary for this spike; the full Jambda state
and Refine execution path is intentionally deferred to the next integration
milestone.

Every pair runs native and translated cases in isolated subprocesses with the
same backend configuration. A run is complete only when all three translated
proofs verify and their public outputs match the native outputs. Host Calls,
GAS, sub-VM, and Native AIR are not M3 acceptance criteria.
