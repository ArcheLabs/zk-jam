# ADR 0005: M4 proven translation pipeline

Status: implemented for the bounded M4 smoke

M4 closes the M3 architectural gap by making `TranslatedProgramV1` the only
source used by the generated OpenVM guest. The pipeline is:

```text
PvmProgramV1
    ↓ validate
translate()
    ↓
TranslatedProgramV1
    ↓ deterministic emitter
generated Rust guest
    ↓ OpenVM build/transpile
ELF / VmExe
    ↓ execute / prove / verify
public program, input, and output values
```

The generated guest is a build artifact in a temporary package. The previous
M3 handwritten translated guests remain only as historical M3 regression
fixtures and are not selected by the M4 runner.

M4 uses domain-separated SHA-256 commitments over versioned canonical bytes:
the PVM program commitment includes the translation version and canonical
`PvmProgramV1`; the input commitment includes the translation version and the
canonical two-word `ExecutionInputV1`. The guest consumes both witness words,
encodes `u32 word_count || little-endian words` inside the versioned commitment
envelope, and reveals program commitment, input commitment, and output as
exactly 96 OpenVM public-value bytes. The verifier uses the strict
`M4PublicValuesV1` parser and checks those values against the expected
statement in addition to verifying the OpenVM proof.

The M4 smoke covers arithmetic with two inputs, branch true/false/equal, and
16 KiB memory. Unsupported opcodes and unsupported control flow fail closed.
M4 does not implement full JAM Refine, Host Calls, GAS, sub-VM, Native AIR,
consensus, or worker integration.

The OpenVM proof is cryptographically tied to the generated executable through
OpenVM's prepared executable, proving key, aggregation verifying key, and
public-values proof. The application-level M4 statement adds the explicit
program/input/output binding; `context_hash` remains metadata and is not the
primary security mechanism.

M4 CI separates correctness from expensive proving. The preflight job builds
and executes all three generated programs with `samples=1,warmup=0`. Three
program-specific proof jobs reuse the same source revision and pinned Jambda
metadata, and an aggregate job validates all partial schemas, case bindings,
program reuse, and the final publication gate. Local `bench m4` therefore
requires `--execute-only`; full proving is a remote acceptance workflow.

The publication benchmark is intentionally separate from this completion
gate. The workflow matrix runs `bench m4-publication-workload` once per fixed
representative workload; each job runs Native then translated on one runner.
`aggregate-m4-publication` only reads the three partial reports and renders the
raw timings and chart-friendly CSV output. Its `complete/partial/unavailable`
status cannot turn a correct M4 report into an incomplete one, and its
single-sample ratios must not be presented as full JAM or Kusama performance
claims.
