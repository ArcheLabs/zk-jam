# M2 OpenVM public benchmark baseline

The machine-generated report is created by `zk-jam bench m2 --backend cpu` under a unique run ID. The checked-in `benchmarks/baselines/m2.json` is a schema/config anchor, not a manually copied timing result. This keeps public numbers tied to the exact `environment.json`, raw `runs.jsonl`, and generated `summary.json` for the run that produced them. Summary JSON is case-oriented and CSV is one row per case.

The public report must include OpenVM version/revision, CPU environment, security configuration, sample counts, phase timings, proof size, executable size, peak RSS methodology, and the following disclaimer:

> These results measure the OpenVM proving substrate and ZK-JAM integration only. They do not yet measure PVM Translation, PVM memory emulation, Refine Host Calls, or Native PVM proving.

M2 cases are arithmetic/default, branch/true, branch/false, branch/equal, and memory/1024, memory/16384, memory/262144. The runner performs one separate warmup and ten measured samples by default; warmup records remain in raw JSONL and are excluded from summary statistics. Prove remains the primary metric. Publication readiness requires a release build, clean tree, all seven cases, at least five measured samples per case, and no failed samples.
