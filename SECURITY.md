# Security boundary

ZkRefine Profile v1 proves only the checked-in single-item import/export
profile described in [the profile specification](docs/zkrefine-profile-v1.md).
The proof does not claim full JAM execution, gas correctness, historical state
lookup, inner PVM execution, aggregation, or general client compatibility.

Release acceptance fails closed unless the OpenVM proof and serialized reload
verify, the reference result and exports match, the WorkReport bindings pass,
and all four statement tamper checks reject.
