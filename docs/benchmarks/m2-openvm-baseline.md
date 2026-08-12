# M2 OpenVM public benchmark baseline

The machine-generated report is created by `zk-jam bench m2 --backend cpu` under a unique run ID. The checked-in `benchmarks/baselines/m2.json` is a schema/config anchor, not a manually copied timing result. This keeps public numbers tied to the exact `environment.json`, raw `runs.jsonl`, and generated `summary.json` for the run that produced them.

The public report must include OpenVM version/revision, CPU environment, security configuration, sample counts, phase timings, proof size, executable size, peak RSS methodology, and the following disclaimer:

> These results measure the OpenVM proving substrate and ZK-JAM integration only. They do not yet measure PVM Translation, PVM memory emulation, Refine Host Calls, or Native PVM proving.

M2 memory cases are 1 KiB, 16 KiB, and 256 KiB. The first measured output is never used as a warmup; the runner performs one separate warmup and ten measured samples for the tiny probes. Prove remains the primary metric.
