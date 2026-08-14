# zk-jam

Client-independent ZkRefine Smoke-v0 boundary for JAM 0.7.2.

The M0/M1 interface remains frozen. M2 adds a bounded OpenVM v2.0.1 RV32IM
integration spike, and M3 adds a pinned Jambda-revision translation smoke for
arithmetic, branch-true, and deterministic 16 KiB memory workloads. Refine
Host Calls, GAS, sub-VM, and Native AIR remain outside this scope.

The M3 publication benchmark requires read-only access to the private pinned
Jambda revision. Local development and normal CI do not require Jambda access.

```text
cargo test --workspace
cargo run -p zk-jam -- make-minimal /tmp/example.case.bin
cargo run -p zk-jam -- inspect /tmp/example.case.bin
cargo run -p zk-jam -- openvm info
cargo run -p zk-jam -- bench m2 --backend cpu
cargo run --release -p zk-jam -- bench m3 --jambda-repo /path/to/jambda --samples 1 --warmup 0
cargo run --release -p zk-jam -- bench m4 --execute-only --jambda-repo /path/to/jambda --samples 1 --warmup 0
# after the M4 correctness artifact is available, run one workload comparison:
cargo run --release -p zk-jam -- bench m4-publication-workload --workload arithmetic --m4-report /path/to/m4-benchmark.json --output benchmarks/results
```

See [docs/smoke-v0.md](docs/smoke-v0.md) and
[docs/refine-commitments.md](docs/refine-commitments.md) for the original
boundary, and [benchmarks/README.md](benchmarks/README.md) for public M2 and
M3 and M4 benchmark output.

M4 local runs are execute-only preflight runs. The full proof benchmark is
split into one preflight job, three parallel program-specific proof jobs, and
an aggregate job in GitHub Actions; it is intentionally not run as a local
default because OpenVM key generation and proving are expensive. The separate
M4 publication comparison runs three same-runner Native OpenVM versus
translated-guest workloads and emits raw measurements, ratios, and a claims
boundary; it is diagnostic data, not a new correctness gate.
