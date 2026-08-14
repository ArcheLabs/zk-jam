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

The M2 acceptance sequence and the M3 public CPU paired benchmark are now
implemented. M3 completion is recorded by `m3-benchmark.json` with
`complete: true`; its scope is limited to the three normalized translation
fixtures described in [ADR 0004](decisions/0004-m3-translation-smoke.md).
No Jambda source is modified by this repository.
