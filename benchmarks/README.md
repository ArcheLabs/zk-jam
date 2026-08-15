# Public Benchmark Layout

## PVM -> OpenVM Benchmark

The formal public benchmark is selected with the `pvm-openvm` milestone in
`.github/workflows/benchmark.yml`. It measures the cost of executing and
proving the bounded PVM fixtures through three isolated implementation paths:

- Direct OpenVM Guest
- Generated Guest
- Direct PVM Lowering

Run the semantic gate locally with:

```text
zk-jam bench pvm-openvm-preflight --output benchmarks/results
```

After the gate is complete, each workload is run on the same runner with one
fresh worker process per implementation:

```text
zk-jam bench pvm-openvm-workload --workload arithmetic --semantic-gate path/to/pvm-openvm-preflight.json --output benchmarks/results
```

The aggregate command performs no proving:

```text
zk-jam bench pvm-openvm-aggregate --semantic-gate path/to/pvm-openvm-preflight.json \
  --partial-arithmetic benchmarks/results/pvm-openvm-arithmetic.json \
  --partial-branch benchmarks/results/pvm-openvm-branch.json \
  --partial-memory benchmarks/results/pvm-openvm-memory.json \
  --output benchmarks/results
```

Formal reports use `schema/pvm-openvm-benchmark-v1.schema.json` and produce:
`pvm-openvm-benchmark.json`, `pvm-openvm-benchmark.csv`,
`pvm-openvm-comparison.csv`, and `pvm-openvm-benchmark.md`.
The semantic gate must pass all six existing cases before proof benchmarking;
worker failure is recorded on the affected side and does not discard other
completed implementation measurements. Ratios use Direct OpenVM Guest as the
1.0 baseline.

## Historical M4 / M4.1 Artifacts

M4 and M4.1 were development milestone names. Their existing schemas and
artifacts remain available as historical data, but they have been superseded
by the unified PVM -> OpenVM Benchmark.

The runner creates a unique `YYYYMMDD-HHMMSSZ_<git-short>_<backend>` directory below `benchmarks/results/` and mirrors publication-ready output below `benchmarks/public/`.

```text
benchmarks/
  schema/{environment-v1.schema.json,run-v1.schema.json,summary-v1.schema.json}
  schema/{environment-v2.schema.json,run-v2.schema.json,summary-v2.schema.json}
  schema/m3-paired-v1.schema.json
  schema/m3-paired-v2.schema.json
  schema/m4-preflight-v1.schema.json
  schema/m4-proof-partial-v1.schema.json
  schema/m4-proven-translation-v1.schema.json
  schema/m4-publication-v1.schema.json
  baselines/m2.json
  results/<run-id>/{environment.json,runs.jsonl,summary.json,summary.csv,report.md,artifacts/}
  results/<run-id>/{m3-benchmark.json,m3-benchmark.csv,m3-benchmark.md}
  results/m4-publication-<timestamp>/{m4-publication.json,m4-publication.csv,m4-comparison.csv,m4-publication.md}
  public/<run-id>/{environment.json,summary.json,summary.csv,report.md}
```

Run with `zk-jam bench m2 --backend cpu`. Numbers are machine-generated from `runs.jsonl`; a report must not be edited by hand.

M2 emits the v2 formats. Peak RSS is one scalar per isolated benchmark-case process in
`summary.json`; it is deliberately not repeated in `runs.jsonl` or presented as a sample
distribution. Publication-ready M2 runs require the pinned guest toolchain
`nightly-2026-01-18`.

Run the M3 paired smoke with `zk-jam bench m3 --jambda-repo /path/to/jambda --samples 1 --warmup 0`. It emits
`m3-benchmark.json`, `m3-benchmark.csv`, and `m3-benchmark.md` under an
isolated run directory. The JSON follows
`schema/m3-paired-v2.schema.json`; `complete: true` requires three pairs,
matching public outputs, successful proof verification, and no worker error.
Native and translated cases run in separate subprocesses so peak RSS is
case-scoped. M3 currently covers only the static arithmetic, branch-true, and
16 KiB memory fixtures. `publication_ready` additionally requires a clean
zk-jam checkout, verified Jambda provenance, and the pinned OpenVM/toolchain.

Validate a report with `zk-jam bench validate-m3 path/to/m3-benchmark.json`.

M4 local validation uses `zk-jam bench m4 --execute-only --jambda-repo /path/to/jambda
--samples 1 --warmup 0` and emits an `m4-preflight-v1` JSON/Markdown pair. The
preflight checks translation, emission, build/transpile, execution, the strict
128-byte OpenVM public-values envelope (96-byte semantic statement plus
zero-padding), and all six reference-output comparisons.

The remote proof workflow runs `zk-jam bench m4-proof --program arithmetic|branch|memory`
once per generated executable, then combines the three `m4-proof-partial-v1`
reports with `zk-jam bench aggregate-m4` into the proven-translation v1
JSON/CSV/Markdown triplet. The six cases reuse three generated executables:
two arithmetic inputs, branch true/false/equal, and memory-16KiB. Publication
readiness additionally requires every proof, program binding, input binding,
reference-output comparison, metadata match, and program-reuse check to pass.

Run the M4.0.2 publication comparison only after the M4 correctness report is
complete and publication-ready:

```text
zk-jam bench m4-publication-workload --workload arithmetic --m4-report path/to/m4-benchmark.json --output benchmarks/results
zk-jam bench aggregate-m4-publication --m4-report path/to/m4-benchmark.json \
  --partial-arithmetic benchmarks/results/m4-publication-arithmetic.json \
  --partial-branch benchmarks/results/m4-publication-branch.json \
  --partial-memory benchmarks/results/m4-publication-memory.json \
  --output benchmarks/results
zk-jam bench validate-m4-publication path/to/m4-publication.json
```

The workflow runs the three workload commands in a matrix. Within each matrix
job, Native OpenVM runs first and the generated translated guest runs second on
the same runner. The aggregate job only reads the three partial JSON artifacts;
it performs no proving. The partial artifacts are named
`m4-publication-arithmetic.json`, `m4-publication-branch.json`, and
`m4-publication-memory.json`.

The comparison uses direct Native OpenVM guests and generated translated guests
for arithmetic `[7, 9]`, branch-true `[21, 8]`, and 16 KiB memory
`[0x12345678, 16384]`. Both sides use the same 128-byte OpenVM public-values
envelope (96-byte semantic statement plus 32 zero-padding bytes), runtime input
encoding, OpenVM configuration, and runner. Build/transpile,
keygen, and translation/emission are preparation or per-program costs;
execute/prove/verify are per-execution observations. Ratios are always
`translated / native`, while `reference_execute_ns` is informational only.

The Native guest embeds the translated workload's PVM commitment only to keep
the public-values envelope comparable. This does not give the Native guest
M4's mechanical translation binding. The publication report is a
single-sample diagnostic and does not demonstrate full JAM Refine, production
proving performance, or Kusama integration. The workflow uploads the result as
`m4-publication-${sha}-${run_id}` and never commits or pushes benchmark data.
