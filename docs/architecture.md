# Architecture

JAM clients produce `RefineCaseV1` and `PvmProgramV1`. Translation and Native
consume those same values and are compared against a client reference output.
Jambda is the first adapter, not a public dependency of this repository.
