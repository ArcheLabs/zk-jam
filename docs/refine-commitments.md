# Refine commitment mapping (M0 status)

This table records the current boundary and the items that must be frozen
before claiming a production proof relation. The hashes emitted by the M0
helpers are deterministic development hashes with explicit `zk-jam/m0/*`
domains; they are not a replacement for the JAM 0.7.2 commitments.

| Refine value | Adapter source | JAM 0.7.2 mapping | M0 status |
|---|---|---|---|
| Work Package | `WorkReportInput.work_package` | package hash / canonical Work Package encoding | adapter exports canonical bytes; exact proof mapping pending audit |
| Work Item | Work Package item at `item_index` | indirectly bound by Work Package | derived from canonical package bytes |
| payload | Work Item payload | Work Package / payload hash relation | derived from canonical package bytes |
| external data | `WorkReportInput.external_data` | work-item extrinsic hash and length relation | exported as witness bytes; exact proof path pending |
| imported segments | `WorkReportInput.import_segments` | import segment commitment/root | exported as witness bytes; exact proof path pending |
| authorization trace | authorization runner output | authorization commitment | exported as witness bytes; exact commitment pending |
| code blob | `RefineRead::load_c_blob` | service code hash and historical lookup | code hash is preserved; inclusion proof pending |
| historical lookup | `RefineRead` preimage/lookups | lookup anchor and state commitment | `FixtureStateWitness` only in M0/M1 |
| exports | reference output | exports root | deterministic M0 root helper; exact JAM tree mapping pending |

The unresolved rows are deliberate blockers for the final Smoke Gate. M0/M1
must not describe materialized fixture data as committed state evidence.
