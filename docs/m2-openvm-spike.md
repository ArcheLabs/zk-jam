# M2 OpenVM integration spike

The M2 boundary is implemented in `zk-jam-openvm-backend`:

- `OpenVmProgramArtifact` holds an official RV32IM-transpiled `VmExe`.
- `OpenVmExecutionResult` records public output and execution timing.
- `OpenVmProofArtifact` is JSON-reloadable and carries the versioned proof, verification baseline, aggregation VK, context hash, and public output binding.
- The proof test matrix covers output tampering, proof corruption, wrong context, branch output mismatch, and memory output tampering.

The acceptance test is intentionally ignored by default because it requires the guest toolchain and CPU proving resources. Run it with:

```text
OPENVM_RUST_TOOLCHAIN=nightly-2026-01-18 cargo test -p zk-jam-openvm-backend m2_proof_round_trip_and_tamper_matrix -- --ignored --nocapture
```

M3 readiness is `NOT READY` until that sequence and a public CPU benchmark run complete successfully. No Jambda source is modified by M2.
