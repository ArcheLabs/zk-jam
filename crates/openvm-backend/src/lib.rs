//! OpenVM v2.0.1 integration spike for the M2 milestone.
//!
//! This crate deliberately stops at the RV32IM/OpenVM substrate boundary. It does not contain
//! PVM translation, PVM memory emulation, Refine Host Calls, or Native PVM proving.

use std::{env, path::PathBuf, process::Command, sync::Arc, time::Instant};

use blake2b_simd::Params;
use eyre::{eyre, Result, WrapErr};
use openvm_build::{GuestOptions, TargetFilter};
use openvm_sdk::{
    config::{AggregationSystemParams, AppConfig},
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
pub const OPENVM_PINNED_GUEST_TOOLCHAIN: &str = "nightly-2026-01-18";
pub const ARITHMETIC_FIXED_XOR: u32 = 0xA5A5_5A5A;
pub const M4_SEMANTIC_PUBLIC_VALUES_LEN: usize = 96;
pub const M4_OPENVM_PUBLIC_VALUES_LEN: usize = 128;
pub const M4_PUBLIC_VALUES_MERKLE_CHUNK_BYTES: usize = 8;

type OpenVmSdk = Sdk;
pub type OpenVmVerifyingKey = MultiStarkVerifyingKey<openvm_sdk::SC>;

/// The bounded M2 programs. These are integration probes, not PVM semantics.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum M2Benchmark {
    Arithmetic,
    Branch,
    Memory { bytes: usize },
    M3TranslationArithmetic,
    M3TranslationBranchTrue,
    M3TranslationMemory16K,
    M4GeneratedArithmetic,
    M4GeneratedBranch,
    M4GeneratedMemory16K,
    M4NativeArithmetic,
    M4NativeBranch,
    M4NativeMemory16K,
}

impl M2Benchmark {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Arithmetic => "arithmetic",
            Self::Branch => "branch",
            Self::Memory { .. } => "memory",
            Self::M3TranslationArithmetic => "m3-translation-arithmetic",
            Self::M3TranslationBranchTrue => "m3-translation-branch-true",
            Self::M3TranslationMemory16K => "m3-translation-memory-16384",
            Self::M4GeneratedArithmetic => "m4-generated-arithmetic",
            Self::M4GeneratedBranch => "m4-generated-branch",
            Self::M4GeneratedMemory16K => "m4-generated-memory-16384",
            Self::M4NativeArithmetic => "m4-native-arithmetic",
            Self::M4NativeBranch => "m4-native-branch",
            Self::M4NativeMemory16K => "m4-native-memory-16384",
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Memory { bytes } => format!("memory-{bytes}"),
            Self::M3TranslationArithmetic => "m3-translation-arithmetic".to_string(),
            Self::M3TranslationBranchTrue => "m3-translation-branch-true".to_string(),
            Self::M3TranslationMemory16K => "m3-translation-memory-16384".to_string(),
            Self::M4GeneratedArithmetic => "m4-generated-arithmetic".to_string(),
            Self::M4GeneratedBranch => "m4-generated-branch".to_string(),
            Self::M4GeneratedMemory16K => "m4-generated-memory-16384".to_string(),
            _ => self.name().to_string(),
        }
    }

    fn guest_binary(&self) -> &'static str {
        match self {
            Self::Arithmetic => "m2-arithmetic-v1",
            Self::Branch => "m2-branch-v1",
            Self::Memory { .. } => "m2-memory-v1",
            Self::M3TranslationArithmetic => "m3-translation-arithmetic-v1",
            Self::M3TranslationBranchTrue => "m3-translation-branch-v1",
            Self::M3TranslationMemory16K => "m3-translation-memory-v1",
            Self::M4GeneratedArithmetic | Self::M4GeneratedBranch | Self::M4GeneratedMemory16K => {
                "m4-generated-v1"
            }
            Self::M4NativeArithmetic => "m4-native-arithmetic-v1",
            Self::M4NativeBranch => "m4-native-branch-v1",
            Self::M4NativeMemory16K => "m4-native-memory-16384-v1",
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
    pub serialized_executable_size_bytes: usize,
    pub build_time_ns: u128,
    pub transpile_time_ns: u128,
    exe: Arc<openvm_sdk::openvm_circuit::arch::instructions::exe::VmExe<openvm_sdk::F>>,
}

impl OpenVmProgramArtifact {
    fn clone_for_proving(&self) -> Self {
        Self {
            benchmark: self.benchmark.clone(),
            openvm_version: self.openvm_version,
            openvm_revision: self.openvm_revision,
            emission_path: self.emission_path,
            executable_bytes: self.executable_bytes,
            serialized_executable_size_bytes: self.serialized_executable_size_bytes,
            build_time_ns: self.build_time_ns,
            transpile_time_ns: self.transpile_time_ns,
            exe: self.exe.clone(),
        }
    }
}

/// A program with OpenVM proving and verification keys prepared once for reuse.
pub struct OpenVmPreparedProgram {
    pub program: OpenVmProgramArtifact,
    pub app_keygen_time_ns: u128,
    pub agg_keygen_time_ns: u128,
    pub keygen_time_ns: u128,
    sdk: OpenVmSdk,
    agg_vk: OpenVmVerifyingKey,
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

    pub fn proof_payload_size_bytes(&self) -> usize {
        self.proof.proof.len()
            + self.proof.user_pvs_proof.len()
            + self
                .proof
                .deferral_merkle_proofs
                .as_ref()
                .map_or(0, Vec::len)
    }

    pub fn artifact_size_bytes(&self) -> Result<usize> {
        Ok(self.to_bytes()?.len())
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct M4ExpectedStatement {
    pub program_commitment: [u8; 32],
    pub input_commitment: [u8; 32],
    pub public_output: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct M4PublicValuesV1 {
    pub program_commitment: [u8; 32],
    pub input_commitment: [u8; 32],
    pub output: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum M4PublicValuesError {
    #[error("M4 semantic public values must be exactly 96 bytes, got {0}")]
    InvalidLength(usize),
    #[error("M4 OpenVM public values must be exactly 128 bytes, got {0}")]
    InvalidOpenVmLength(usize),
    #[error("M4 OpenVM public values contain non-zero reserved padding")]
    NonZeroPadding,
}

impl M4PublicValuesV1 {
    pub const SEMANTIC_LEN: usize = M4_SEMANTIC_PUBLIC_VALUES_LEN;
    pub const OPENVM_LEN: usize = M4_OPENVM_PUBLIC_VALUES_LEN;
    pub const LEN: usize = Self::SEMANTIC_LEN;

    pub fn encode(&self) -> [u8; M4_SEMANTIC_PUBLIC_VALUES_LEN] {
        let mut bytes = [0u8; M4_SEMANTIC_PUBLIC_VALUES_LEN];
        bytes[..32].copy_from_slice(&self.program_commitment);
        bytes[32..64].copy_from_slice(&self.input_commitment);
        bytes[64..].copy_from_slice(&self.output);
        bytes
    }

    pub fn encode_openvm(&self) -> [u8; M4_OPENVM_PUBLIC_VALUES_LEN] {
        let mut bytes = [0u8; M4_OPENVM_PUBLIC_VALUES_LEN];
        bytes[..M4_SEMANTIC_PUBLIC_VALUES_LEN].copy_from_slice(&self.encode());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, M4PublicValuesError> {
        if bytes.len() != M4_SEMANTIC_PUBLIC_VALUES_LEN {
            return Err(M4PublicValuesError::InvalidLength(bytes.len()));
        }
        Ok(Self::decode_semantic_bytes(bytes))
    }

    pub fn decode_openvm(bytes: &[u8]) -> Result<Self, M4PublicValuesError> {
        if bytes.len() != M4_OPENVM_PUBLIC_VALUES_LEN {
            return Err(M4PublicValuesError::InvalidOpenVmLength(bytes.len()));
        }
        if bytes[M4_SEMANTIC_PUBLIC_VALUES_LEN..]
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err(M4PublicValuesError::NonZeroPadding);
        }
        Ok(Self::decode_semantic_bytes(
            &bytes[..M4_SEMANTIC_PUBLIC_VALUES_LEN],
        ))
    }

    fn decode_semantic_bytes(bytes: &[u8]) -> Self {
        let mut program_commitment = [0u8; 32];
        let mut input_commitment = [0u8; 32];
        let mut output = [0u8; 32];
        program_commitment.copy_from_slice(&bytes[..32]);
        input_commitment.copy_from_slice(&bytes[32..64]);
        output.copy_from_slice(&bytes[64..]);
        Self {
            program_commitment,
            input_commitment,
            output,
        }
    }
}

/// M4 application statement wrapped around the normal OpenVM proof artifact. The three public
/// values are emitted by the guest and therefore are part of the cryptographically verified
/// OpenVM public-values proof, not trusted host metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct M4ProofArtifact {
    pub schema_version: u32,
    pub program_commitment: [u8; 32],
    pub input_commitment: [u8; 32],
    pub public_output: [u8; 32],
    pub proof: OpenVmProofArtifact,
}

impl M4ProofArtifact {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec_pretty(self).wrap_err("serialize M4 proof artifact")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).wrap_err("reload M4 proof artifact")
    }

    pub fn verify_m4(&self, expected: &M4ExpectedStatement, input: M2Input) -> Result<()> {
        if self.schema_version != 1 {
            return Err(eyre!("unsupported M4 proof artifact version"));
        }
        if self.program_commitment != expected.program_commitment
            || self.input_commitment != expected.input_commitment
            || self.public_output != expected.public_output
        {
            return Err(eyre!("M4 expected statement mismatch"));
        }
        let canonical_input = zk_jam_translation::ExecutionInputV1::new(vec![input.a, input.b]);
        if self.input_commitment != zk_jam_translation::input_commitment(&canonical_input) {
            return Err(eyre!("M4 input commitment does not match supplied input"));
        }
        let expected_public_values = M4PublicValuesV1 {
            program_commitment: expected.program_commitment,
            input_commitment: expected.input_commitment,
            output: expected.public_output,
        };
        let actual_public_values = M4PublicValuesV1::decode_openvm(&self.proof.public_output)
            .map_err(|error| eyre!("decode M4 proof public values: {error}"))?;
        if actual_public_values != expected_public_values
            || self.proof.public_output != expected_public_values.encode_openvm()
        {
            return Err(eyre!("M4 proof public values mismatch"));
        }
        self.proof
            .verify(&input.context_hash(&self.proof.benchmark))
    }
}

/// Small façade around OpenVM's official SDK flow.
#[derive(Clone, Debug, Default)]
pub struct OpenVmBackend;

impl OpenVmBackend {
    /// Select the guest toolchain once in the parent process so workers inherit the exact value.
    pub fn configure_guest_toolchain() -> String {
        configure_guest_toolchain()
    }

    pub fn info() -> OpenVmInfo {
        OpenVmInfo {
            version: OPENVM_VERSION.to_string(),
            revision: OPENVM_REVISION.to_string(),
            backend: OPENVM_BACKEND.to_string(),
            security_bits: 100,
            emission_path: "B: RV32IM ELF -> official OpenVM transpiler -> VmExe".to_string(),
            guest_toolchain: selected_guest_toolchain(),
        }
    }

    pub fn program(&self, benchmark: M2Benchmark) -> Result<OpenVmProgramArtifact> {
        let binary = benchmark.guest_binary();
        self.program_from_guest_dir(benchmark, &guest_dir(), binary)
    }

    /// Build an OpenVM executable from a caller-provided generated guest package. M4 uses this
    /// path so the translated IR, rather than a checked-in workload selector, determines the ELF.
    pub fn program_from_guest_dir(
        &self,
        benchmark: M2Benchmark,
        guest_dir: &std::path::Path,
        guest_binary: &str,
    ) -> Result<OpenVmProgramArtifact> {
        configure_guest_toolchain();
        let sdk = sdk();
        let build_started = Instant::now();
        let elf = sdk
            .build(
                GuestOptions::default(),
                guest_dir,
                &Some(TargetFilter {
                    name: guest_binary.to_string(),
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
        // `VmExe::init_memory` uses tuple keys, which JSON cannot encode as object keys. Keep the
        // measurement deterministic by serializing the same executable components with the sparse
        // memory image represented as ordered triples.
        let init_memory = exe
            .init_memory
            .iter()
            .map(|((address_space, address), value)| (*address_space, *address, *value))
            .collect::<Vec<_>>();
        let serialized_executable_size_bytes =
            serde_json::to_vec(&(&exe.program, exe.pc_start, init_memory, &exe.fn_bounds))
                .wrap_err("serialize OpenVM executable for size measurement")?
                .len();
        Ok(OpenVmProgramArtifact {
            benchmark,
            openvm_version: OPENVM_VERSION,
            openvm_revision: OPENVM_REVISION,
            emission_path: "B: RV32IM ELF -> official OpenVM transpiler -> VmExe",
            executable_bytes,
            serialized_executable_size_bytes,
            build_time_ns,
            transpile_time_ns,
            exe,
        })
    }

    pub fn m4_native_program(&self, benchmark: M2Benchmark) -> Result<OpenVmProgramArtifact> {
        let guest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("guests/m4-native");
        let binary = benchmark.guest_binary();
        self.program_from_guest_dir(benchmark, &guest_dir, binary)
    }

    /// Prepare OpenVM proving and verifying keys once for a program/configuration.
    pub fn prepare(&self, program: OpenVmProgramArtifact) -> Result<OpenVmPreparedProgram> {
        let sdk = sdk_for_benchmark(&program.benchmark);
        let keygen_started = Instant::now();
        let app_keygen_started = Instant::now();
        let _ = sdk.app_keygen();
        let app_keygen_time_ns = app_keygen_started.elapsed().as_nanos();
        let agg_keygen_started = Instant::now();
        let (_, agg_vk) = sdk.agg_keygen();
        let agg_keygen_time_ns = agg_keygen_started.elapsed().as_nanos();
        Ok(OpenVmPreparedProgram {
            program,
            app_keygen_time_ns,
            agg_keygen_time_ns,
            keygen_time_ns: keygen_started.elapsed().as_nanos(),
            sdk,
            agg_vk,
        })
    }

    pub fn execute(
        &self,
        program: &OpenVmProgramArtifact,
        input: M2Input,
    ) -> Result<OpenVmExecutionResult> {
        let stdin = input.stdin();
        let sdk = sdk_for_benchmark(&program.benchmark);
        let started = Instant::now();
        let public_output = sdk
            .execute(ExecutableFormat::SharedVmExe(program.exe.clone()), stdin)
            .map_err(|error| eyre!("execute {}: {error}", program.benchmark.name()))?;
        Ok(OpenVmExecutionResult {
            benchmark: program.benchmark.clone(),
            public_output,
            elapsed_ns: started.elapsed().as_nanos(),
            executable_bytes: program.serialized_executable_size_bytes,
        })
    }

    pub fn execute_prepared(
        &self,
        prepared: &OpenVmPreparedProgram,
        input: M2Input,
    ) -> Result<OpenVmExecutionResult> {
        let stdin = input.stdin();
        let started = Instant::now();
        let public_output = prepared
            .sdk
            .execute(
                ExecutableFormat::SharedVmExe(prepared.program.exe.clone()),
                stdin,
            )
            .map_err(|error| eyre!("execute {}: {error}", prepared.program.benchmark.name()))?;
        Ok(OpenVmExecutionResult {
            benchmark: prepared.program.benchmark.clone(),
            public_output,
            elapsed_ns: started.elapsed().as_nanos(),
            executable_bytes: prepared.program.serialized_executable_size_bytes,
        })
    }

    pub fn prove(
        &self,
        program: &OpenVmProgramArtifact,
        input: M2Input,
    ) -> Result<OpenVmProofArtifact> {
        let prepared = self.prepare(program.clone_for_proving());
        let prepared = prepared?;
        self.prove_prepared(&prepared, input)
    }

    pub fn prove_prepared(
        &self,
        prepared: &OpenVmPreparedProgram,
        input: M2Input,
    ) -> Result<OpenVmProofArtifact> {
        let prove_started = Instant::now();
        let (proof, baseline) = prepared
            .sdk
            .prove(
                ExecutableFormat::SharedVmExe(prepared.program.exe.clone()),
                input.stdin(),
                &[],
            )
            .map_err(|error| eyre!("prove {}: {error}", prepared.program.benchmark.name()))?;
        let prove_time_ns = prove_started.elapsed().as_nanos();
        let public_output = proof
            .user_pvs_proof
            .public_values
            .iter()
            .map(|value| value.as_canonical_u32() as u8)
            .collect::<Vec<_>>();
        let versioned = VersionedVmStarkProof::new(proof)
            .map_err(|error| eyre!("encode OpenVM proof: {error}"))?;
        let context_hash = input.context_hash(&prepared.program.benchmark);
        Ok(OpenVmProofArtifact {
            benchmark: prepared.program.benchmark.clone(),
            openvm_version: OPENVM_VERSION.to_string(),
            openvm_revision: OPENVM_REVISION.to_string(),
            backend: OPENVM_BACKEND.to_string(),
            security_bits: 100,
            keygen_time_ns: prepared.keygen_time_ns,
            prove_time_ns,
            context_hash,
            public_output,
            proof: versioned,
            baseline: baseline.into(),
            agg_vk: prepared.agg_vk.clone(),
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

fn sdk_for_benchmark(benchmark: &M2Benchmark) -> OpenVmSdk {
    if matches!(
        benchmark,
        M2Benchmark::M4GeneratedArithmetic
            | M2Benchmark::M4GeneratedBranch
            | M2Benchmark::M4GeneratedMemory16K
            | M2Benchmark::M4NativeArithmetic
            | M2Benchmark::M4NativeBranch
            | M2Benchmark::M4NativeMemory16K
    ) {
        let app_params = app_params_with_100_bits_security(MAX_APP_LOG_STACKED_HEIGHT);
        let mut app_config = AppConfig::riscv32(app_params);
        app_config.app_vm_config.system.config = app_config
            .app_vm_config
            .system
            .config
            .clone()
            .with_public_values(M4_OPENVM_PUBLIC_VALUES_LEN);
        return Sdk::new(app_config, AggregationSystemParams::default())
            .expect("valid M4 OpenVM SDK configuration");
    }
    sdk()
}

fn guest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("guests/m2")
}

fn selected_guest_toolchain() -> String {
    env::var("OPENVM_RUST_TOOLCHAIN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| OPENVM_PINNED_GUEST_TOOLCHAIN.to_string())
}

fn configure_guest_toolchain() -> String {
    if let Ok(configured) = env::var("OPENVM_RUST_TOOLCHAIN") {
        if !configured.trim().is_empty() {
            return configured;
        }
    }
    let installed = Command::new("rustup")
        .args(["toolchain", "list"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    let selected = select_guest_toolchain(&installed);
    env::set_var("OPENVM_RUST_TOOLCHAIN", selected);
    selected.to_string()
}

fn select_guest_toolchain(installed: &str) -> &'static str {
    if installed
        .lines()
        .any(|line| line.starts_with(OPENVM_PINNED_GUEST_TOOLCHAIN))
    {
        OPENVM_PINNED_GUEST_TOOLCHAIN
    } else if installed.lines().any(|line| {
        line.starts_with("nightly-x86_64")
            || line.starts_with("nightly-aarch64")
            || line.trim() == "nightly"
    }) {
        // Generic nightly is a development-only fallback. Publication readiness rejects it.
        "nightly"
    } else {
        // Let rustup/OpenVM install the pinned toolchain when no usable nightly exists locally.
        OPENVM_PINNED_GUEST_TOOLCHAIN
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
    fn m4_public_values_roundtrip_and_fail_closed() {
        let values = M4PublicValuesV1 {
            program_commitment: [1; 32],
            input_commitment: [2; 32],
            output: [3; 32],
        };
        assert_eq!(M4PublicValuesV1::decode(&values.encode()).unwrap(), values);
        assert!(M4PublicValuesV1::decode(&values.encode()[..95]).is_err());
        let mut oversized = values.encode().to_vec();
        oversized.push(4);
        assert!(M4PublicValuesV1::decode(&oversized).is_err());
        let mut tampered = values.encode();
        tampered[0] ^= 1;
        assert_ne!(M4PublicValuesV1::decode(&tampered).unwrap(), values);
        tampered[32] ^= 1;
        assert_ne!(M4PublicValuesV1::decode(&tampered).unwrap(), values);
        tampered[64] ^= 1;
        assert_ne!(M4PublicValuesV1::decode(&tampered).unwrap(), values);

        assert_eq!(M4_OPENVM_PUBLIC_VALUES_LEN, 128);
        assert_eq!(M4_SEMANTIC_PUBLIC_VALUES_LEN, 96);
        assert_eq!(
            M4_OPENVM_PUBLIC_VALUES_LEN % M4_PUBLIC_VALUES_MERKLE_CHUNK_BYTES,
            0
        );
        assert!(
            (M4_OPENVM_PUBLIC_VALUES_LEN / M4_PUBLIC_VALUES_MERKLE_CHUNK_BYTES).is_power_of_two()
        );
        let openvm = values.encode_openvm();
        assert_eq!(openvm.len(), 128);
        assert_eq!(M4PublicValuesV1::decode_openvm(&openvm).unwrap(), values);
        let mut short = openvm.to_vec();
        short.pop();
        assert!(M4PublicValuesV1::decode_openvm(&short).is_err());
        let mut long = openvm.to_vec();
        long.push(0);
        assert!(M4PublicValuesV1::decode_openvm(&long).is_err());
        let mut non_zero_padding = openvm;
        non_zero_padding[96] = 1;
        assert!(matches!(
            M4PublicValuesV1::decode_openvm(&non_zero_padding),
            Err(M4PublicValuesError::NonZeroPadding)
        ));
    }

    #[test]
    fn guest_toolchain_selection_prefers_the_pin() {
        assert_eq!(
            select_guest_toolchain(
                "nightly-x86_64-unknown-linux-gnu\nnightly-2026-01-18-x86_64-unknown-linux-gnu\n"
            ),
            OPENVM_PINNED_GUEST_TOOLCHAIN
        );
        assert_eq!(
            select_guest_toolchain("nightly-x86_64-unknown-linux-gnu\n"),
            "nightly"
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
