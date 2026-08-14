# ADR 0002: M2 translation/emission path

Status: accepted for M2

M2 selects Path B: a minimal RV32IM guest ELF is built by the official OpenVM guest build flow and converted with `VmExe::from_elf` plus the official RV32I, RV32M, and IO transpiler extensions. The three guest programs are checked into `crates/openvm-backend/guests/m2`.

Path A, direct construction of an OpenVM executable, remains a bounded negative spike only. It is useful for testing the VM circuit in isolation, but it would bypass the guest compiler/linker and would not exercise the emission boundary that the integration needs. No direct-instruction Path A implementation is accepted as M2 evidence.

The selected path is intentionally limited to arithmetic, branch, and bounded memory behavior. M2 itself is not PVM translation and does not introduce a Jambda or Native semantics change. M3 builds on this path with a compile-time lowering of three normalized `PvmProgramV1` fixtures; it records the pinned Jambda adapter revision and keeps the generated guest emissions checked in. Host Calls, GAS, sub-VM, and Native AIR remain outside the smoke boundary.
