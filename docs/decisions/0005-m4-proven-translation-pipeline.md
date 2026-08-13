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
computes the input commitment, and reveals program commitment, input
commitment, and output as OpenVM public values. The verifier checks those
values against the expected statement in addition to verifying the OpenVM
proof.

The M4 smoke covers arithmetic with two inputs, branch true/false/equal, and
16 KiB memory. Unsupported opcodes and unsupported control flow fail closed.
M4 does not implement full JAM Refine, Host Calls, GAS, sub-VM, Native AIR,
consensus, or worker integration.

The OpenVM proof is cryptographically tied to the generated executable through
OpenVM's prepared executable, proving key, aggregation verifying key, and
public-values proof. The application-level M4 statement adds the explicit
program/input/output binding; `context_hash` remains metadata and is not the
primary security mechanism.
