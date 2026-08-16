# M5 ZkRefine Core

M5 is a deliberately narrow, workflow-only end-to-end Refine proof profile. It
uses one real Jambda WorkPackage with one WorkItem and one 4104-byte import
segment. The only supported host calls are `ECALLI 1` (FETCH mode 6) and
`ECALLI 7` (EXPORT). Gas proofs, historical lookup, inner PVM execution,
aggregation, and local proving are outside this profile and are rejected by the
acceptance runner.

The Jambda adapter is compiled against the exact checkout selected by
`integration/jambda-m3.json`. It executes the real Refine path and emits the
canonical `RefineCaseV1`, reference result, exports, and WorkReport metadata.
The OpenVM guest receives only the canonical case bytes as private input; it
reads the import from that witness, applies the fixture PVM operation, performs
the export transformation, and reveals the 128-byte M5 statement:

```text
profile_id || case_commitment || result_commitment || exports_commitment
```

The report verifier checks the proof public values and all corresponding
Jambda-produced package/result/export bindings without re-executing PVM. It
also checks serialized proof reload and rejects case/result/export/statement
tampering.

M5 proving is intentionally unavailable from local CLI execution. Run the one
manual workflow `.github/workflows/m5-zkrefine.yml`; it fails closed if the
pinned Jambda checkout, adapter, case shape, WorkReport, or proof binding does
not match this profile.
