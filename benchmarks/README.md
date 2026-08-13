# Public M2 benchmark layout

The runner creates a unique `YYYYMMDD-HHMMSSZ_<git-short>_<backend>` directory below `benchmarks/results/` and mirrors publication-ready output below `benchmarks/public/`.

```text
benchmarks/
  schema/{environment-v1.schema.json,run-v1.schema.json,summary-v1.schema.json}
  schema/{environment-v2.schema.json,run-v2.schema.json,summary-v2.schema.json}
  baselines/m2.json
  results/<run-id>/{environment.json,runs.jsonl,summary.json,summary.csv,report.md,artifacts/}
  public/<run-id>/{environment.json,summary.json,summary.csv,report.md}
```

Run with `zk-jam bench m2 --backend cpu`. Numbers are machine-generated from `runs.jsonl`; a report must not be edited by hand.

M2 emits the v2 formats. Peak RSS is one scalar per isolated benchmark-case process in
`summary.json`; it is deliberately not repeated in `runs.jsonl` or presented as a sample
distribution. Publication-ready M2 runs require the pinned guest toolchain
`nightly-2026-01-18`.
