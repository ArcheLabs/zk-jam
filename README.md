# zk-jam

Client-independent ZkRefine Smoke-v0 boundary for JAM 0.7.2.

The current M0/M1 implementation freezes `RefineCaseV1`, `PvmProgramV1`, the
deterministic binary fixture codec, the Smoke-v0 admission policy, statement
skeletons, and the Jambda adapter boundary. OpenVM proving, Translation
execution, and Native AIR are intentionally deferred until the contract and
reference fixture path are stable.

```text
cargo test --workspace
cargo run -p zk-jam -- make-minimal /tmp/example.case.bin
cargo run -p zk-jam -- inspect /tmp/example.case.bin
```

See [docs/smoke-v0.md](docs/smoke-v0.md) and
[docs/refine-commitments.md](docs/refine-commitments.md) for scope and the
remaining commitment audit items.
