# ADR 0001: OpenVM version and proving configuration

Status: accepted for M2

M2 pins OpenVM `v2.0.1` at revision `b820b25baab6c5d9b055f64e0286b6b1058e707c`. The CPU backend uses the official OpenVM SDK RV32 preset and the 100-bit application security parameter set (`app_params_with_100_bits_security`). CUDA is not part of the M2 acceptance path.

The pin is recorded in the workspace `Cargo.toml` and is repeated in the public proof artifact and benchmark environment record. The parent process resolves and exports `OPENVM_RUST_TOOLCHAIN` before benchmark workers start. Publication requires `nightly-2026-01-18`; an installed generic `nightly` may only be selected as a development fallback.

This is a substrate spike. The configuration does not claim security or performance for PVM Translation, PVM memory emulation, Refine Host Calls, or Native PVM proving.
