# Architecture

JAM clients produce `RefineCaseV1` and `PvmProgramV1`. Translation and Native
consume those same values and are compared against a client reference output.
Jambda is the first adapter, pinned in M3 reports at
`b850a458fa00da81e80be4cc84ddd7d2222f1edc`.

M3 translates three bounded normalized `PvmProgramV1` fixtures into static
OpenVM guest emissions and proves each emission beside its native OpenVM
counterpart. The translation layer is deliberately a compile-time lowering:
there is no runtime PVM interpreter in the guest. The M3 boundary excludes
Refine Host Calls, GAS, sub-VM, and Native AIR.
