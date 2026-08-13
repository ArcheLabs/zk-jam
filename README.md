# zk-jam

Client-independent ZkRefine Smoke-v0 boundary for JAM 0.7.2.

The M0/M1 interface remains frozen. M2 adds a bounded OpenVM v2.0.1 RV32IM
integration spike, and M3 adds a pinned Jambda-revision translation smoke for
arithmetic, branch-true, and deterministic 16 KiB memory workloads. Refine
Host Calls, GAS, sub-VM, and Native AIR remain outside this scope.

```text
cargo test --workspace
cargo run -p zk-jam -- make-minimal /tmp/example.case.bin
cargo run -p zk-jam -- inspect /tmp/example.case.bin
cargo run -p zk-jam -- openvm info
cargo run -p zk-jam -- bench m2 --backend cpu
cargo run -p zk-jam -- bench m3 --samples 1 --warmup 0
```

See [docs/smoke-v0.md](docs/smoke-v0.md) and
[docs/refine-commitments.md](docs/refine-commitments.md) for the original
boundary, and [benchmarks/README.md](benchmarks/README.md) for public M2 and
M3 benchmark output.
