# zk-jam

Client-independent ZkRefine Smoke-v0 boundary for JAM 0.7.2.

The M0/M1 interface remains frozen. M2 adds a bounded OpenVM v2.0.1 RV32IM
integration spike for arithmetic, branch, and deterministic memory guests;
PVM Translation, Refine Host Calls, and Native AIR remain outside this scope.

```text
cargo test --workspace
cargo run -p zk-jam -- make-minimal /tmp/example.case.bin
cargo run -p zk-jam -- inspect /tmp/example.case.bin
cargo run -p zk-jam -- openvm info
cargo run -p zk-jam -- bench m2 --backend cpu
```

See [docs/smoke-v0.md](docs/smoke-v0.md) and
[docs/refine-commitments.md](docs/refine-commitments.md) for the original
boundary, and [benchmarks/README.md](benchmarks/README.md) for public M2
benchmark output.
