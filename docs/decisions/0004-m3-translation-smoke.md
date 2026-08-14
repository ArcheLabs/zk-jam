# ADR 0004: M3 translation smoke boundary

Status: accepted for M3

M3 establishes a public paired benchmark for the first three translation
workloads: arithmetic, a branch-taken path, and deterministic 16 KiB memory.
The normalized `PvmProgramV1` fixtures are lowered to static RV32IM/OpenVM
guest emissions. The guest does not interpret PVM instructions at proving
time, which keeps the benchmark measuring emitted OpenVM proving rather than a
runtime interpreter.

The report pins the Jambda adapter reference through the machine-readable
`integration/jambda-m3.json` manifest and pins OpenVM at `2.0.1` revision
`b820b25baab6c5d9b055f64e0286b6b1058e707c`. The checked-in M3 fixtures are the
client-independent normalized boundary for this spike; the full Jambda state
and Refine execution path is intentionally deferred to the next integration
milestone.

The Jambda source repository remains private. M3 publication runs verify the
pinned Jambda revision by checking out the exact commit in GitHub Actions
using read-only credentials. The repository and full revision are recorded in
every publication-ready M3 report. Jambda source code and credentials are
never embedded into benchmark artifacts. Jambda provenance only proves that
M3 references a concrete source revision; it does not prove that all Jambda
PVM semantics have been translated correctly or establish full JAM Refine
equivalence.

Every pair runs native and translated cases in isolated subprocesses with the
same backend configuration. A run is complete only when all three translated
proofs verify and their public outputs match the native outputs. M3 proves the
checked-in statically emitted OpenVM guest programs corresponding to the three
bounded translation workloads. M3 does not yet mechanically bind the
`translate()` output to the OpenVM guest executable being proved. Host Calls,
GAS, sub-VM, and Native AIR are not M3 acceptance criteria.

M4 requirement:

```text
PvmProgramV1
    ↓
translate()
    ↓
emitted representation
    ↓
OpenVM executable
    ↓
proof
```

M4 must replace the current manually synchronized relationship between
Translation output and the checked-in static guest. M3.1 deliberately does
not implement that binding.
