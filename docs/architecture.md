# Architecture

JAM clients produce `RefineCaseV1` and `PvmProgramV1`. Translation and Native
consume those same values and are compared against a client reference output.
Jambda is the first adapter, pinned by the machine-readable
`integration/jambda-m3.json` manifest and recorded in M3 reports.

M3 translates three bounded normalized `PvmProgramV1` fixtures into a static
translation representation and proves checked-in OpenVM guest emissions
beside their native OpenVM counterparts. The translation layer is deliberately
a compile-time lowering: there is no runtime PVM interpreter in the guest.
M3 does not yet mechanically bind `translate()` output to the OpenVM guest
executable being proved; that is an M4 requirement. The M3 boundary excludes
Refine Host Calls, GAS, sub-VM, and Native AIR.

M4 closes that boundary for a bounded subset:

```text
PvmProgramV1 → validate → translate() → TranslatedProgramV1
             → deterministic Rust emitter → OpenVM executable → proof
```

The generated guest consumes runtime input and reveals program commitment,
input commitment, and output as proof public values. Full Refine remains a
later milestone.
