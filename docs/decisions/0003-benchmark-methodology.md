# ADR 0003: M2 benchmark methodology

Status: accepted for M2

The CPU path is the required reproducible baseline. Each benchmark performs one warmup execution, then ten measured samples for the tiny M2 probes. Build/transpile setup is measured separately from execute/prove/verify samples. Prove is the primary metric; execute and verify are reported independently.

The runner writes immutable timestamped run directories. It refuses to overwrite an existing run ID. Each raw JSONL record contains phase timings, total time, executable/proof sizes, public output, backend, and OpenVM metric names when available. Summary statistics are generated from raw records and include n, min, max, mean, median, p95, and standard deviation.

Peak RSS uses `/proc/self/status` `VmHWM` on Linux, sampled after each measured case. It is reported as null on platforms where that source is unavailable. CPU and GPU results must never be combined; M2 currently emits CPU only.
