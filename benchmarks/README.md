# Public M2 and M3 benchmark layout

The runner creates a unique `YYYYMMDD-HHMMSSZ_<git-short>_<backend>` directory below `benchmarks/results/` and mirrors publication-ready output below `benchmarks/public/`.

```text
benchmarks/
  schema/{environment-v1.schema.json,run-v1.schema.json,summary-v1.schema.json}
  schema/{environment-v2.schema.json,run-v2.schema.json,summary-v2.schema.json}
  schema/m3-paired-v1.schema.json
  baselines/m2.json
  results/<run-id>/{environment.json,runs.jsonl,summary.json,summary.csv,report.md,artifacts/}
  results/<run-id>/{m3-benchmark.json,m3-benchmark.csv,m3-benchmark.md}
  public/<run-id>/{environment.json,summary.json,summary.csv,report.md}
```

Run with `zk-jam bench m2 --backend cpu`. Numbers are machine-generated from `runs.jsonl`; a report must not be edited by hand.

M2 emits the v2 formats. Peak RSS is one scalar per isolated benchmark-case process in
`summary.json`; it is deliberately not repeated in `runs.jsonl` or presented as a sample
distribution. Publication-ready M2 runs require the pinned guest toolchain
`nightly-2026-01-18`.

Run the M3 paired smoke with `zk-jam bench m3 --samples 1 --warmup 0`. It emits
`m3-benchmark.json`, `m3-benchmark.csv`, and `m3-benchmark.md` under an
isolated run directory. The JSON follows
`schema/m3-paired-v1.schema.json`; `complete: true` requires three pairs,
matching public outputs, successful proof verification, and no worker error.
Native and translated cases run in separate subprocesses so peak RSS is
case-scoped. M3 currently covers only the static arithmetic, branch-true, and
16 KiB memory fixtures.
