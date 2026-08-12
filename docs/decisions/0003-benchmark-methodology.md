# ADR 0003: M2 benchmark methodology

Status: accepted for M2

The CPU path is the required reproducible baseline. Each benchmark performs one warmup proof, then ten measured samples by default. Build/transpile/keygen setup is measured once per executable and is reported separately from execute/prove/verify samples. Prove is the primary metric; execute and verify are reported independently. The prepared OpenVM context reuses application and aggregation keys across warmup and measured samples.

The runner writes immutable timestamped run directories. It refuses to overwrite an existing run ID. Each raw JSONL record contains phase timings, total time, case identity, success/error state, executable/proof sizes, public output, backend, and OpenVM metric names when available. Summary statistics are case-oriented and include n, min, max, mean, median, p95, and standard deviation. Proof payload bytes and serialized artifact bytes are separate metrics; serialized VmExe bytes and the structural estimate are separate metrics.

Peak RSS uses `/proc/self/status` `VmHWM` inside one isolated child process per benchmark case. The scope is `benchmark-case`, and it includes setup/keygen plus warmup and measured samples. It is reported as null on platforms where that source is unavailable. CPU and GPU results must never be combined; M2 currently emits CPU only. Long-lived parent-process VmHWM is never labeled as per-case RAM.
