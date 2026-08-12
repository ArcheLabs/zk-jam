# Smoke-v0

This repository currently freezes the M0/M1 client boundary for JAM 0.7.2:

- non-subVM Refine only;
- no Gas proof and no `GAS` host call;
- only `FETCH`, `HISTORICAL_LOOKUP`, and `EXPORT` are admitted;
- `RefineCaseV1` and `PvmProgramV1` are client-independent;
- canonical binary encoding is used for fixtures and interchange;
- JSON is a debug view only.

`FixtureStateWitness` is a bring-up artifact. It is not a protocol commitment
proof. A production Smoke Gate must replace it with a witness bound to the JAM
state commitment.
