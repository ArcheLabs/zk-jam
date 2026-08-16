# zk-jam

ZkRefine Import/Export Profile v1 for JAM 0.7.2.

The public path consumes the checked-in reference vector at
`fixtures/refine-import-export-v1`, then runs OpenVM execute, prove, verify,
serialized reload, verify, and four binding tamper checks. It covers one
WorkPackage and one WorkItem, FETCH host-call 1 mode 6, and EXPORT host-call 7
for one 4104-byte segment. The reference program uses `0x20000` as its
writable buffer.

```text
cargo test --workspace
cargo run -p zk-jam -- inspect fixtures/refine-import-export-v1/case.bin
cargo run -p zk-jam -- zkrefine --fixture fixtures/refine-import-export-v1 --output artifacts/zkrefine
```

OpenVM proving is resource intensive; the reproducible proving run is
performed by [the ZkRefine workflow](.github/workflows/zkrefine.yml).

Gas accounting, historical lookup, inner PVM, other FETCH modes, arbitrary
programs, aggregation, and recursive wrapping are outside this profile.
