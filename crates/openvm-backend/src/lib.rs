//! OpenVM v2.0.1 integration spike for the M2 milestone.
//!
//! This crate deliberately stops at the RV32IM/OpenVM substrate boundary. It does not contain
//! PVM translation, PVM memory emulation, Refine Host Calls, or Native PVM proving.

use std::{env, path::PathBuf, process::Command, sync::Arc, time::Instant};

use blake2b_simd::Params;
use eyre::{eyre, Result, WrapErr};
use openvm_build::{GuestOptions, TargetFilter};
use openvm_sdk::{
    config::AggregationSystemParams,
    types::{ExecutableFormat, VerificationBaselineJson, VersionedVmStarkProof},
    Sdk, StdIn,
};
use openvm_stark_backend::{keygen::types::MultiStarkVerifyingKey, p3_field::PrimeField32};
use openvm_stark_sdk::config::{app_params_with_100_bits_security, MAX_APP_LOG_STACKED_HEIGHT};
use openvm_verify_stark_host::VmStarkProof;
use serde::{Deserialize, Serialize};

pub const OPENVM_VERSION: &str = "2.0.1";
pub const OPENVM_REVISION: &str = "b820b25baab6c5d9b055f64e0286b6b1058e707c";
pub const OPENVM_BACKEND: &str = "cpu";
pub const ARITHMETIC_FIXED_XOR: u32 = 0xA5A5_5A5A;

type OpenVmSdk = Sdk;
pub type OpenVmVerifyingKey = MultiStarkVerifyingKey<openvm_sdk::SC>;

/// The bounded M2 programs. These are integration probes, not PVM semantics.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum M2Benchmark {
    Arithmetic,
    Branch,
    Memory { bytes: usize },
}

impl M2Benchmark {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Arithmetic => "arithmetic",
            Self::Branch => "branch",
            Self::Memory { .. } => "memory",
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Memory { bytes } => format!("memory-{bytes}"),
            _ => self.name().to_string(),
        }
    }

    fn guest_binary(&self) -> &'static str {
        match self {
            Self::Arithmetic => "m2-arithmetic-v1",
            Self::Branch => "m2-branch-v1",
            Self::Memory { .. } => "m2-memory-v1",
        }
    }
}

/// A transpiled OpenVM executable and its M2 identity.
pub struct OpenVmProgramArtifact {
    pub benchmark: M2Benchmark,
    pub openvm_version: &'static str,
    pub openvm_revision: &'static str,
    pub emission_path: &'static str,
    pub executable_bytes: usize,
    pub build_time_ns: u128,
    pub transpile_time_ns: u128,
    exe: Arc<openvm_sdk::openvm_circuit::arch::instructions::exe::VmExe<openvm_sdk::F>>,
}

/// Result of running one M2 program.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenVmExecutionResult {
    pub benchmark: M2Benchmark,
    pub public_output: Vec<u8>,
    pub elapsed_ns: u128,
    pub executable_bytes: usize,
}

/// A reloadable OpenVM proof plus all context needed for independent verification.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenVmProofArtifact {
    pub benchmark: M2Benchmark,
    pub openvm_version: String,
    pub openvm_revision: String,
    pub backend: String,
    pub security_bits: u32,
    pub keygen_time_ns: u128,
    pub prove_time_ns: u128,
    pub context_hash: String,
    pub public_output: Vec<u8>,
    pub proof: VersionedVmStarkProof,
    pub baseline: VerificationBaselineJson,
    pub agg_vk: OpenVmVerifyingKey,
}

impl OpenVmProofArtifact {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec_pretty(self).wrap_err("serialize OpenVM proof artifact")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).wrap_err("reload OpenVM proof artifact")
    }

    /// Verify a reloaded proof and its application-level output/context binding.
    pub fn verify(&self, expected_context: &str) -> Result<()> {
        if self.context_hash != expected_context {
            return Err(eyre!("OpenVM context mismatch"));
        }

        let proof = VmStarkProof::try_from(self.proof.clone())
            .map_err(|error| eyre!("decode OpenVM proof: {error}"))?;
        let actual_output = proof
            .user_pvs_proof
            .public_values
            .iter()
            .map(|value| value.as_canonical_u32() as u8)
            .collect::<Vec<_>>();
        if actual_output != self.public_output {
            return Err(eyre!("public output mismatch"));
        }

        Sdk::verify_proof(self.agg_vk.clone(), self.baseline.clone().into(), &proof)
            .map_err(|error| eyre!("OpenVM proof verification failed: {error}"))
    }
}

/// Small façade around OpenVM's official SDK flow.
#[derive(Clone, Debug, Default)]
pub struct OpenVmBackend;

impl OpenVmBackend {
    pub fn info() -> OpenVmInfo {
        OpenVmInfo {
            version: OPENVM_VERSION.to_string(),
            revision: OPENVM_REVISION.to_string(),
            backend: OPENVM_BACKEND.to_string(),
            security_bits: 100,
            emission_path: "B: RV32IM ELF -> official OpenVM transpiler -> VmExe",
            guest_toolchain: env::var("OPENVM_RUST_TOOLCHAIN")
                .unwrap_or_else(|_| "OpenVM default nightly-2026-01-18".to_string()),
        }
    }

    pub fn program(&self, benchmark: M2Benchmark) -> Result<OpenVmProgramArtifact> {
        configure_guest_toolchain();
        let sdk = sdk();
        let guest_dir = guest_dir();
        let build_started = Instant::now();
        let elf = sdk
            .build(
                GuestOptions::default(),
                &guest_dir,
                &Some(TargetFilter {
                    name: benchmark.guest_binary().to_string(),
                    kind: "bin".to_string(),
                }),
                None,
            )
            .map_err(|error| eyre!("build {} guest: {error}", benchmark.name()))?;
        let build_time_ns = build_started.elapsed().as_nanos();
        let transpile_started = Instant::now();
        let exe = sdk
            .convert_to_exe(elf)
            .map_err(|error| eyre!("transpile {} guest: {error}", benchmark.name()))?;
        let transpile_time_ns = transpile_started.elapsed().as_nanos();
        let executable_bytes =
            exe.program.instructions_and_debug_infos.len() * 32 + exe.init_memory.len() * 8;
        Ok(OpenVmProgramArtifact {
            benchmark,
            openvm_version: OPENVM_VERSION,
            openvm_revision: OPENVM_REVISION,
            emission_path: "B: RV32IM ELF -> official OpenVM transpiler -> VmExe",
            executable_bytes,
            build_time_ns,
            transpile_time_ns,
            exe,
        })
    }

    pub fn execute(
        &self,
        program: &OpenVmProgramArtifact,
        input: M2Input,
    ) -> Result<OpenVmExecutionResult> {
        let stdin = input.stdin();
        let sdk = sdk();
        let started = Instant::now();
        let public_output = sdk
            .execute(ExecutableFormat::SharedVmExe(program.exe.clone()), stdin)
            .map_err(|error| eyre!("execute {}: {error}", program.benchmark.name()))?;
        Ok(OpenVmExecutionResult {
            benchmark: program.benchmark.clone(),
            public_output,
            elapsed_ns: started.elapsed().as_nanos(),
            executable_bytes: program.executable_bytes,
        })
    }

    pub fn prove(
        &self,
        program: &OpenVmProgramArtifact,
        input: M2Input,
    ) -> Result<OpenVmProofArtifact> {
        let sdk = sdk();
        let execution = self.execute(program, input)?;
        let keygen_started = Instant::now();
        let _ = sdk.app_keygen();
        let _ = sdk.agg_keygen();
        let keygen_time_ns = keygen_started.elapsed().as_nanos();
        let prove_started = Instant::now();
        let (proof, baseline) = sdk
            .prove(
                ExecutableFormat::SharedVmExe(program.exe.clone()),
                input.stdin(),
                &[],
            )
            .map_err(|error| eyre!("prove {}: {error}", program.benchmark.name()))?;
        let prove_time_ns = prove_started.elapsed().as_nanos();
        let versioned = VersionedVmStarkProof::new(proof)
            .map_err(|error| eyre!("encode OpenVM proof: {error}"))?;
        let context_hash = input.context_hash(&program.benchmark);
        let (_, agg_vk) = sdk.agg_keygen();
        Ok(OpenVmProofArtifact {
            benchmark: program.benchmark.clone(),
            openvm_version: OPENVM_VERSION.to_string(),
            openvm_revision: OPENVM_REVISION.to_string(),
            backend: OPENVM_BACKEND.to_string(),
            security_bits: 100,
            keygen_time_ns,
            prove_time_ns,
            context_hash,
            public_output: execution.public_output,
            proof: versioned,
            baseline: baseline.into(),
            agg_vk,
        })
    }

    pub fn verify(&self, artifact: &OpenVmProofArtifact, input: M2Input) -> Result<()> {
        artifact.verify(&input.context_hash(&artifact.benchmark))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenVmInfo {
    pub version: String,
    pub revision: String,
    pub backend: String,
    pub security_bits: u32,
    pub emission_path: String,
    pub guest_toolchain: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct M2Input {
    pub a: u32,
    pub b: u32,
}

impl M2Input {
    pub fn arithmetic(a: u32, b: u32) -> Self {
        Self { a, b }
    }

    pub fn branch(a: u32, b: u32) -> Self {
        Self { a, b }
    }

    pub fn memory(seed: u32, bytes: usize) -> Result<Self> {
        if !bytes.is_multiple_of(4) || bytes == 0 {
            return Err(eyre!(
                "memory benchmark size must be a non-zero multiple of 4"
            ));
        }
        Ok(Self {
            a: seed,
            b: bytes as u32,
        })
    }

    fn stdin(self) -> StdIn {
        let mut stdin = StdIn::default();
        stdin.write(&self.a);
        stdin.write(&self.b);
        stdin
    }

    pub fn context_hash(self, benchmark: &M2Benchmark) -> String {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(benchmark.name().as_bytes());
        bytes.extend_from_slice(&self.a.to_le_bytes());
        bytes.extend_from_slice(&self.b.to_le_bytes());
        let digest = Params::new().hash_length(32).hash(&bytes);
        digest.to_hex().to_string()
    }
}

fn sdk() -> OpenVmSdk {
    let app_params = app_params_with_100_bits_security(MAX_APP_LOG_STACKED_HEIGHT);
    Sdk::riscv32(app_params, AggregationSystemParams::default())
}

fn guest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("guests/m2")
}

fn configure_guest_toolchain() {
    if env::var_os("OPENVM_RUST_TOOLCHAIN").is_some() {
        return;
    }
    let Ok(output) = Command::new("rustup").args(["toolchain", "list"]).output() else {
        return;
    };
    if output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.starts_with("nightly"))
    {
        // The OpenVM default nightly is not installed in every environment. The generic nightly
        // toolchain is used only as a local fallback; callers can pin an exact toolchain through
        // OPENVM_RUST_TOOLCHAIN and the selected value is recorded in benchmark environment.json.
        env::set_var("OPENVM_RUST_TOOLCHAIN", "nightly");
    }
}

pub fn expected_arithmetic(a: u32, b: u32) -> u32 {
    a.wrapping_add(b).wrapping_mul(3) ^ ARITHMETIC_FIXED_XOR
}

pub fn expected_branch(a: u32, b: u32) -> u32 {
    if a > b {
        a.wrapping_sub(b).wrapping_mul(7)
    } else {
        b.wrapping_sub(a).wrapping_mul(11)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_vector_matches_guest_formula() {
        assert_eq!(expected_arithmetic(7, 9), 0xA5A5_5A6A);
        assert_eq!(expected_arithmetic(u32::MAX, 1), 0xA5A5_5A5A);
    }

    #[test]
    fn branch_covers_true_false_and_equal() {
        assert_eq!(expected_branch(21, 8), 91);
        assert_eq!(expected_branch(8, 21), 143);
        assert_eq!(expected_branch(8, 8), 0);
    }

    #[test]
    fn memory_input_is_bounded_to_word_sizes() {
        assert!(M2Input::memory(1, 1024).is_ok());
        assert!(M2Input::memory(1, 16 * 1024).is_ok());
        assert!(M2Input::memory(1, 256 * 1024).is_ok());
        assert!(M2Input::memory(1, 3).is_err());
        assert!(M2Input::memory(1, 0).is_err());
    }

    #[test]
    fn context_changes_when_input_changes() {
        let benchmark = M2Benchmark::Arithmetic;
        assert_ne!(
            M2Input::arithmetic(1, 2).context_hash(&benchmark),
            M2Input::arithmetic(1, 3).context_hash(&benchmark)
        );
    }

    #[test]
    #[ignore = "requires the OpenVM guest toolchain and CPU proving"]
    fn m2_proof_round_trip_and_tamper_matrix() {
        let backend = OpenVmBackend;
        for (benchmark, input) in [
            (M2Benchmark::Arithmetic, M2Input::arithmetic(7, 9)),
            (M2Benchmark::Branch, M2Input::branch(21, 8)),
            (
                M2Benchmark::Memory { bytes: 1024 },
                M2Input::memory(1, 1024).unwrap(),
            ),
        ] {
            let program = backend.program(benchmark.clone()).unwrap();
            let artifact = backend.prove(&program, input).unwrap();
            let reloaded = OpenVmProofArtifact::from_bytes(&artifact.to_bytes().unwrap()).unwrap();
            backend.verify(&reloaded, input).unwrap();

            let mut public_output_tampered = reloaded.clone();
            public_output_tampered.public_output[0] ^= 1;
            assert!(backend.verify(&public_output_tampered, input).is_err());

            let mut corrupt_proof = reloaded.clone();
            corrupt_proof.proof.proof[0] ^= 1;
            assert!(backend.verify(&corrupt_proof, input).is_err());

            assert!(reloaded.verify("wrong-context").is_err());
        }
    }
}
