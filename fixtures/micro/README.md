# M0 micro fixture

The canonical fixture format is `RefineCaseV1` binary. Generate a case from a
client adapter, then inspect it with:

```text
zk-jam inspect fixtures/refine/example/case.bin
zk-jam inspect fixtures/refine/example/case.bin --json
```

The JSON form is for debugging only and is never used as a commitment input.
