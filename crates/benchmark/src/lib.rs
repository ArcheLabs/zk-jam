//! M2.8 benchmark orchestration and publication-safe reporting.

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

use eyre::{eyre, Result, WrapErr};
use serde::{Deserialize, Serialize};
use zk_jam_openvm_backend::{M2Benchmark, M2Input, OpenVmBackend, OpenVmPreparedProgram};

pub use zk_jam_refine_interface::RefineCaseV1;

pub const DISCLAIMER: &str = "These results measure the OpenVM proving substrate and ZK-JAM integration only. They do not yet measure PVM Translation, PVM memory emulation, Refine Host Calls, or Native PVM proving.";
pub const MANDATORY_CASES: usize = 7;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnvironmentReport {
    pub schema_version: String,
    pub run_id: String,
    pub git_commit: Option<String>,
    pub git_dirty: bool,
    pub os: String,
    pub arch: String,
    pub cpu: Option<String>,
    pub logical_cpus: Option<usize>,
    pub memory_bytes: Option<u64>,
    pub kernel: Option<String>,
    pub rustc_version: Option<String>,
    pub cargo_version: Option<String>,
    pub build_profile: String,
    pub backend: String,
    pub openvm_version: String,
    pub openvm_revision: String,
    pub security_bits: u32,
    pub openvm_config_hash: Option<String>,
    pub guest_toolchain: String,
    pub rss_scope: String,
    pub rss_includes_keygen: bool,
    pub rss_method: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunRecord {
    pub schema_version: String,
    pub run_id: String,
    pub benchmark: String,
    pub case: String,
    pub backend: String,
    pub sample_index: usize,
    pub warmup: bool,
    pub success: bool,
    pub build_time_ns: Option<u128>,
    pub transpile_time_ns: Option<u128>,
    pub keygen_time_ns: Option<u128>,
    pub execute_time_ns: Option<u128>,
    pub prove_time_ns: Option<u128>,
    pub verify_time_ns: Option<u128>,
    pub total_time_ns: u128,
    pub proof_payload_size_bytes: Option<usize>,
    pub artifact_size_bytes: Option<usize>,
    pub serialized_executable_size_bytes: Option<usize>,
    pub estimated_executable_size_bytes: Option<usize>,
    pub peak_rss_bytes: Option<u64>,
    pub openvm_metrics: BTreeMap<String, serde_json::Value>,
    pub public_output_hex: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MetricSummary {
    pub n: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
    pub median: Option<f64>,
    pub p95: Option<f64>,
    pub stddev: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchmarkCaseSummary {
    pub benchmark: String,
    pub case: String,
    pub samples: usize,
    pub failed_samples: usize,
    pub execute_time_ns: Option<MetricSummary>,
    pub prove_time_ns: Option<MetricSummary>,
    pub verify_time_ns: Option<MetricSummary>,
    pub total_time_ns: Option<MetricSummary>,
    pub peak_rss_bytes: Option<MetricSummary>,
    pub proof_payload_size_bytes: Option<MetricSummary>,
    pub artifact_size_bytes: Option<MetricSummary>,
    pub serialized_executable_size_bytes: Option<MetricSummary>,
    pub estimated_executable_size_bytes: Option<MetricSummary>,
    pub build_time_ns: Option<u128>,
    pub transpile_time_ns: Option<u128>,
    pub keygen_time_ns: Option<u128>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchmarkSummary {
    pub schema_version: String,
    pub run_id: String,
    pub backend: String,
    pub disclaimer: String,
    pub publication_ready: bool,
    pub publication_reasons: Vec<String>,
    pub warmup_samples: usize,
    pub measured_samples: usize,
    pub cases: Vec<BenchmarkCaseSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchmarkOptions {
    pub samples: usize,
    pub warmup: usize,
    pub quick: bool,
}

impl Default for BenchmarkOptions {
    fn default() -> Self {
        Self {
            samples: 10,
            warmup: 1,
            quick: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BenchmarkRun {
    pub run_id: String,
    pub result_dir: PathBuf,
    pub summary: BenchmarkSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CaseWorkerOutput {
    records: Vec<RunRecord>,
}

#[derive(Clone, Debug)]
struct CaseSpec {
    benchmark: M2Benchmark,
    case: String,
    input: M2Input,
}

pub fn branch_cases() -> Vec<(&'static str, M2Input, u32)> {
    vec![
        ("true", M2Input::branch(21, 8), 91),
        ("false", M2Input::branch(8, 21), 143),
        ("equal", M2Input::branch(8, 8), 0),
    ]
}

fn case_specs(selected: Option<&str>, quick: bool) -> Result<Vec<CaseSpec>> {
    let mut cases = Vec::new();
    if selected.is_none() || selected == Some("arithmetic") {
        cases.push(CaseSpec {
            benchmark: M2Benchmark::Arithmetic,
            case: "default".to_string(),
            input: M2Input::arithmetic(7, 9),
        });
    }
    if selected.is_none() || selected == Some("branch") {
        for (case, input, _) in branch_cases() {
            cases.push(CaseSpec {
                benchmark: M2Benchmark::Branch,
                case: case.to_string(),
                input,
            });
        }
    }
    if selected.is_none() || selected == Some("memory") {
        let sizes = if quick {
            vec![1024]
        } else {
            vec![1024, 16 * 1024, 256 * 1024]
        };
        for bytes in sizes {
            cases.push(CaseSpec {
                benchmark: M2Benchmark::Memory { bytes },
                case: bytes.to_string(),
                input: M2Input::memory(0x1234_5678, bytes)?,
            });
        }
    }
    if cases.is_empty() {
        return Err(eyre!("unknown M2 benchmark: {}", selected.unwrap_or("")));
    }
    Ok(cases)
}

pub fn run_m2(
    output_root: &Path,
    selected: Option<&str>,
    backend_name: &str,
    options: BenchmarkOptions,
) -> Result<BenchmarkRun> {
    if backend_name != "cpu" {
        return Err(eyre!("M2 currently supports --backend cpu only"));
    }
    if options.samples == 0 {
        return Err(eyre!("--samples must be at least 1"));
    }
    let cases = case_specs(selected, options.quick)?;
    let run_id = format!("{}_{}_{}", utc_run_stamp(), git_short(), backend_name);
    let result_dir = output_root.join(&run_id);
    if result_dir.exists() {
        return Err(eyre!(
            "refusing to overwrite existing benchmark run {run_id}"
        ));
    }
    fs::create_dir_all(result_dir.join("artifacts"))?;
    let environment = environment_report(&run_id, backend_name)?;
    write_json(result_dir.join("environment.json"), &environment)?;

    let mut records = Vec::new();
    for spec in &cases {
        let worker_path = result_dir.join(format!(
            ".worker-{}-{}.json",
            spec.benchmark.name(),
            spec.case
        ));
        let sample_text = options.samples.to_string();
        let warmup_text = options.warmup.to_string();
        let worker_path_text = worker_path
            .to_str()
            .ok_or_else(|| eyre!("invalid worker path"))?
            .to_string();
        let executable = env::current_exe().wrap_err("locate zk-jam benchmark worker")?;
        let status = Command::new(executable)
            .args([
                "__bench-worker",
                "m2",
                "--benchmark",
                spec.benchmark.name(),
                "--case",
                &spec.case,
                "--samples",
                &sample_text,
                "--warmup",
                &warmup_text,
                "--output",
                &worker_path_text,
            ])
            .status()
            .wrap_err("spawn benchmark case worker")?;
        if !status.success() {
            let _ = fs::remove_file(&worker_path);
            return Err(eyre!(
                "benchmark worker failed for {}/{}",
                spec.benchmark.name(),
                spec.case
            ));
        }
        let worker: CaseWorkerOutput = read_json(&worker_path)?;
        fs::remove_file(worker_path)?;
        records.extend(worker.records.into_iter().map(|mut record| {
            record.run_id = run_id.clone();
            record
        }));
    }

    let jsonl = records
        .iter()
        .map(serde_json::to_string)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .join("\n");
    fs::write(result_dir.join("runs.jsonl"), format!("{jsonl}\n"))?;
    let summary = summarize(
        &run_id,
        backend_name,
        &records,
        &environment,
        &options,
        cases.len(),
    );
    write_json(result_dir.join("summary.json"), &summary)?;
    write_csv(result_dir.join("summary.csv"), &summary)?;
    fs::write(
        result_dir.join("report.md"),
        render_report(&environment, &summary),
    )?;

    if summary.publication_ready {
        let public_dir = output_root.join("../public").join(&run_id);
        fs::create_dir_all(&public_dir)?;
        write_json(public_dir.join("environment.json"), &environment)?;
        write_json(public_dir.join("summary.json"), &summary)?;
        write_csv(public_dir.join("summary.csv"), &summary)?;
        fs::write(
            public_dir.join("report.md"),
            render_report(&environment, &summary),
        )?;
    }
    Ok(BenchmarkRun {
        run_id,
        result_dir,
        summary,
    })
}

pub fn run_worker(
    benchmark: &str,
    case: &str,
    samples: usize,
    warmup: usize,
    output: &Path,
) -> Result<()> {
    let spec = case_specs(Some(benchmark), false)?
        .into_iter()
        .find(|candidate| candidate.case == case && candidate.benchmark.name() == benchmark)
        .ok_or_else(|| eyre!("unknown benchmark case {benchmark}/{case}"))?;
    let backend = OpenVmBackend;
    let program = backend.program(spec.benchmark.clone())?;
    let prepared = backend.prepare(program)?;
    let mut records = Vec::with_capacity(warmup + samples);
    for index in 0..warmup {
        records.push(measure_sample(&backend, &prepared, &spec, index, true)?);
    }
    for index in 0..samples {
        records.push(measure_sample(&backend, &prepared, &spec, index, false)?);
    }
    let peak_rss = peak_rss_bytes();
    for record in &mut records {
        record.peak_rss_bytes = peak_rss;
    }
    write_json(output.to_path_buf(), &CaseWorkerOutput { records })
}

fn measure_sample(
    backend: &OpenVmBackend,
    prepared: &OpenVmPreparedProgram,
    spec: &CaseSpec,
    sample_index: usize,
    warmup: bool,
) -> Result<RunRecord> {
    let started = Instant::now();
    let mut record = RunRecord {
        schema_version: "run-v2".to_string(),
        run_id: String::new(),
        benchmark: spec.benchmark.name().to_string(),
        case: spec.case.clone(),
        backend: "cpu".to_string(),
        sample_index,
        warmup,
        success: false,
        build_time_ns: Some(prepared.program.build_time_ns),
        transpile_time_ns: Some(prepared.program.transpile_time_ns),
        keygen_time_ns: Some(prepared.keygen_time_ns),
        execute_time_ns: None,
        prove_time_ns: None,
        verify_time_ns: None,
        total_time_ns: 0,
        proof_payload_size_bytes: None,
        artifact_size_bytes: None,
        serialized_executable_size_bytes: Some(prepared.program.serialized_executable_size_bytes),
        estimated_executable_size_bytes: Some(prepared.program.executable_bytes),
        peak_rss_bytes: None,
        openvm_metrics: BTreeMap::new(),
        public_output_hex: None,
        error: None,
    };
    let result = (|| -> Result<()> {
        let execution = backend.execute_prepared(prepared, spec.input)?;
        record.execute_time_ns = Some(execution.elapsed_ns);
        record.public_output_hex = Some(hex(&execution.public_output));
        let proof = backend.prove_prepared(prepared, spec.input)?;
        record.prove_time_ns = Some(proof.prove_time_ns);
        let verify_started = Instant::now();
        proof.verify(&spec.input.context_hash(&spec.benchmark))?;
        record.verify_time_ns = Some(verify_started.elapsed().as_nanos());
        record.proof_payload_size_bytes = Some(proof.proof_payload_size_bytes());
        record.artifact_size_bytes = Some(proof.artifact_size_bytes()?);
        record.success = true;
        Ok(())
    })();
    if let Err(error) = result {
        record.error = Some(error.to_string());
    }
    record.total_time_ns = started.elapsed().as_nanos();
    Ok(record)
}

pub fn report_run(results_root: &Path, run_id: &str) -> Result<String> {
    let path = results_root.join(run_id).join("report.md");
    fs::read_to_string(&path).wrap_err_with(|| format!("read report {}", path.display()))
}

fn summarize(
    run_id: &str,
    backend: &str,
    records: &[RunRecord],
    environment: &EnvironmentReport,
    options: &BenchmarkOptions,
    case_count: usize,
) -> BenchmarkSummary {
    let mut grouped: BTreeMap<(String, String), Vec<&RunRecord>> = BTreeMap::new();
    for record in records {
        grouped
            .entry((record.benchmark.clone(), record.case.clone()))
            .or_default()
            .push(record);
    }
    let cases = grouped
        .into_iter()
        .map(|((benchmark, case), records)| summarize_case(&benchmark, &case, &records))
        .collect::<Vec<_>>();
    let warmup_samples = records.iter().filter(|record| record.warmup).count();
    let measured_samples = records.iter().filter(|record| !record.warmup).count();
    let failed_samples = records.iter().filter(|record| !record.success).count();
    let mut reasons = Vec::new();
    if environment.build_profile != "release" {
        reasons.push("build profile is not release".to_string());
    }
    if environment.git_dirty {
        reasons.push("git working tree is dirty".to_string());
    }
    if environment.openvm_version != "2.0.1" {
        reasons.push("unexpected OpenVM version".to_string());
    }
    if environment.openvm_revision != "b820b25baab6c5d9b055f64e0286b6b1058e707c" {
        reasons.push("unexpected OpenVM revision".to_string());
    }
    if environment.security_bits != 100 {
        reasons.push("unexpected security configuration".to_string());
    }
    if environment.backend != "cpu" {
        reasons.push("backend is not cpu".to_string());
    }
    if options.warmup < 1 || warmup_samples < 1 {
        reasons.push("at least one warmup is required".to_string());
    }
    if options.samples < 5 {
        reasons.push("publication requires at least five measured samples".to_string());
    }
    if case_count != MANDATORY_CASES {
        reasons.push("not all mandatory M2 cases were selected".to_string());
    }
    if cases.iter().any(|case| case.samples < 5) {
        reasons.push("one or more cases have fewer than five measured samples".to_string());
    }
    if failed_samples > 0 {
        reasons.push(format!("{failed_samples} sample(s) failed"));
    }
    if options.quick {
        reasons.push("quick mode is for development only".to_string());
    }
    BenchmarkSummary {
        schema_version: "summary-v2".to_string(),
        run_id: run_id.to_string(),
        backend: backend.to_string(),
        disclaimer: DISCLAIMER.to_string(),
        publication_ready: reasons.is_empty(),
        publication_reasons: reasons,
        warmup_samples,
        measured_samples,
        cases,
    }
}

fn summarize_case(benchmark: &str, case: &str, records: &[&RunRecord]) -> BenchmarkCaseSummary {
    let measured = records
        .iter()
        .filter(|record| !record.warmup)
        .copied()
        .collect::<Vec<_>>();
    let successful = measured
        .iter()
        .filter(|record| record.success)
        .copied()
        .collect::<Vec<_>>();
    let first = records.first().copied();
    let metric = |values: Vec<f64>| (!values.is_empty()).then(|| metric_summary(values));
    BenchmarkCaseSummary {
        benchmark: benchmark.to_string(),
        case: case.to_string(),
        samples: measured.len(),
        failed_samples: measured.iter().filter(|record| !record.success).count(),
        execute_time_ns: metric(
            successful
                .iter()
                .filter_map(|r| r.execute_time_ns.map(|v| v as f64))
                .collect(),
        ),
        prove_time_ns: metric(
            successful
                .iter()
                .filter_map(|r| r.prove_time_ns.map(|v| v as f64))
                .collect(),
        ),
        verify_time_ns: metric(
            successful
                .iter()
                .filter_map(|r| r.verify_time_ns.map(|v| v as f64))
                .collect(),
        ),
        total_time_ns: metric(successful.iter().map(|r| r.total_time_ns as f64).collect()),
        peak_rss_bytes: metric(
            successful
                .iter()
                .filter_map(|r| r.peak_rss_bytes.map(|v| v as f64))
                .collect(),
        ),
        proof_payload_size_bytes: metric(
            successful
                .iter()
                .filter_map(|r| r.proof_payload_size_bytes.map(|v| v as f64))
                .collect(),
        ),
        artifact_size_bytes: metric(
            successful
                .iter()
                .filter_map(|r| r.artifact_size_bytes.map(|v| v as f64))
                .collect(),
        ),
        serialized_executable_size_bytes: metric(
            successful
                .iter()
                .filter_map(|r| r.serialized_executable_size_bytes.map(|v| v as f64))
                .collect(),
        ),
        estimated_executable_size_bytes: metric(
            successful
                .iter()
                .filter_map(|r| r.estimated_executable_size_bytes.map(|v| v as f64))
                .collect(),
        ),
        build_time_ns: first.and_then(|r| r.build_time_ns),
        transpile_time_ns: first.and_then(|r| r.transpile_time_ns),
        keygen_time_ns: first.and_then(|r| r.keygen_time_ns),
    }
}

pub fn metric_summary(mut values: Vec<f64>) -> MetricSummary {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n == 0 {
        return MetricSummary {
            n: 0,
            min: None,
            max: None,
            mean: None,
            median: None,
            p95: None,
            stddev: None,
        };
    }
    let mean = values.iter().sum::<f64>() / n as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / n as f64;
    let percentile = |p: f64| values[((n - 1) as f64 * p).round() as usize];
    MetricSummary {
        n,
        min: values.first().copied(),
        max: values.last().copied(),
        mean: Some(mean),
        median: Some(percentile(0.5)),
        p95: Some(percentile(0.95)),
        stddev: Some(variance.sqrt()),
    }
}

fn render_report(environment: &EnvironmentReport, summary: &BenchmarkSummary) -> String {
    let status = if summary.publication_ready {
        "PUBLICATION READY"
    } else {
        "Development benchmark — not publication ready"
    };
    let mut report = format!(
        "# ZK-JAM M2 OpenVM Baseline\n\n## Scope\n\n{DISCLAIMER}\n\nStatus: **{status}**\n\n"
    );
    if environment.git_dirty {
        report.push_str("> WARNING: git working tree was dirty when this run was collected.\n\n");
    }
    report.push_str("## Environment\n\n");
    report.push_str(&format!("- Run ID: {}\n- Git commit: {} (dirty: {})\n- OS/arch: {}/{}\n- CPU: {}\n- Kernel: {}\n- Rust/Cargo: {} / {}\n- Build profile: {}\n- RSS: {}; scope {}; includes keygen {}\n\n", summary.run_id, environment.git_commit.as_deref().unwrap_or("null"), environment.git_dirty, environment.os, environment.arch, environment.cpu.as_deref().unwrap_or("null"), environment.kernel.as_deref().unwrap_or("null"), environment.rustc_version.as_deref().unwrap_or("null"), environment.cargo_version.as_deref().unwrap_or("null"), environment.build_profile, environment.rss_method, environment.rss_scope, environment.rss_includes_keygen));
    report.push_str("## OpenVM Configuration\n\n");
    report.push_str(&format!("- Version: {}\n- Revision: {}\n- Backend: {}\n- Security target: {} bits\n- Config hash: {}\n- Emission: RV32IM ELF -> official OpenVM transpiler -> VmExe\n\n", environment.openvm_version, environment.openvm_revision, environment.backend, environment.security_bits, environment.openvm_config_hash.as_deref().unwrap_or("null")));
    report.push_str(
        "## One-time Setup\n\n| Test | Build | Transpile | Keygen |\n|---|---:|---:|---:|\n",
    );
    for case in &summary.cases {
        report.push_str(&format!(
            "| {}/{} | {} | {} | {} |\n",
            case.benchmark,
            case.case,
            human_duration(case.build_time_ns.map(|v| v as f64)),
            human_duration(case.transpile_time_ns.map(|v| v as f64)),
            human_duration(case.keygen_time_ns.map(|v| v as f64))
        ));
    }
    report.push_str("\n## Results\n\n");
    report.push_str(&format!("Measured samples: **{}**; excluded warmups: **{}**. Primary values are medians; ranges show min-max.\n\n", summary.measured_samples, summary.warmup_samples));
    report.push_str("| Test | Execute | Prove | Verify | Total | Peak RAM | Proof Payload | Artifact | Executable |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for case in &summary.cases {
        report.push_str(&format!(
            "| {} {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            case.benchmark,
            case.case,
            metric_duration(&case.execute_time_ns),
            metric_duration(&case.prove_time_ns),
            metric_duration(&case.verify_time_ns),
            metric_duration(&case.total_time_ns),
            metric_bytes(&case.peak_rss_bytes),
            metric_bytes(&case.proof_payload_size_bytes),
            metric_bytes(&case.artifact_size_bytes),
            metric_bytes(&case.serialized_executable_size_bytes)
        ));
    }
    report.push_str("\n## Arithmetic\n\nThe arithmetic row uses a=7, b=9.\n\n## Branch\n\nThe true, false, and equal rows use the same executable with (21,8), (8,21), and (8,8).\n\n## Memory Scaling\n\nThe memory rows use 1 KiB, 16 KiB, and 256 KiB deterministic buffers.\n\n## Observations\n\n- Proof payload is the encoded OpenVM proof components only; serialized artifact additionally includes verification context and bindings.\n- Executable is reported from serialized VmExe bytes. The structural estimate is retained separately in raw records.\n- Peak RAM is a benchmark-case child-process high-water mark and includes setup/keygen.\n\n## What This Does Not Measure\n\n> {DISCLAIMER}\n\n## Reproducibility\n\nRun cargo run --release -p zk-jam -- bench m2 --backend cpu --samples 10 --warmup 1 --output benchmarks/results with the expected OpenVM toolchain.\n");
    if !summary.publication_reasons.is_empty() {
        report.push_str("\nPublication readiness reasons:\n");
        for reason in &summary.publication_reasons {
            report.push_str(&format!("- {reason}\n"));
        }
    }
    report
}

fn write_csv(path: PathBuf, summary: &BenchmarkSummary) -> Result<()> {
    let mut csv = String::from("run_id,benchmark,case,backend,samples,build_ms,transpile_ms,keygen_ms,execute_median_ms,prove_median_ms,verify_median_ms,prove_min_ms,prove_max_ms,prove_p95_ms,peak_rss_median_mib,peak_rss_max_mib,proof_payload_bytes,artifact_bytes,executable_bytes,publication_ready\n");
    for case in &summary.cases {
        let field =
            |value: Option<f64>| value.map_or_else(String::new, |value| format!("{value:.3}"));
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            summary.run_id,
            case.benchmark,
            case.case,
            summary.backend,
            case.samples,
            field(case.build_time_ns.map(|v| v as f64 / 1e6)),
            field(case.transpile_time_ns.map(|v| v as f64 / 1e6)),
            field(case.keygen_time_ns.map(|v| v as f64 / 1e6)),
            field(
                case.execute_time_ns
                    .as_ref()
                    .and_then(|v| v.median)
                    .map(|v| v / 1e6)
            ),
            field(
                case.prove_time_ns
                    .as_ref()
                    .and_then(|v| v.median)
                    .map(|v| v / 1e6)
            ),
            field(
                case.verify_time_ns
                    .as_ref()
                    .and_then(|v| v.median)
                    .map(|v| v / 1e6)
            ),
            field(
                case.prove_time_ns
                    .as_ref()
                    .and_then(|v| v.min)
                    .map(|v| v / 1e6)
            ),
            field(
                case.prove_time_ns
                    .as_ref()
                    .and_then(|v| v.max)
                    .map(|v| v / 1e6)
            ),
            field(
                case.prove_time_ns
                    .as_ref()
                    .and_then(|v| v.p95)
                    .map(|v| v / 1e6)
            ),
            field(
                case.peak_rss_bytes
                    .as_ref()
                    .and_then(|v| v.median)
                    .map(|v| v / 1024.0 / 1024.0)
            ),
            field(
                case.peak_rss_bytes
                    .as_ref()
                    .and_then(|v| v.max)
                    .map(|v| v / 1024.0 / 1024.0)
            ),
            field(
                case.proof_payload_size_bytes
                    .as_ref()
                    .and_then(|v| v.median)
            ),
            field(case.artifact_size_bytes.as_ref().and_then(|v| v.median)),
            field(
                case.serialized_executable_size_bytes
                    .as_ref()
                    .and_then(|v| v.median)
            ),
            summary.publication_ready
        ));
    }
    fs::write(path, csv)?;
    Ok(())
}

fn metric_display(metric: &Option<MetricSummary>, formatter: fn(f64) -> String) -> String {
    metric.as_ref().map_or_else(
        || "n/a".to_string(),
        |metric| match (metric.median, metric.min, metric.max) {
            (Some(median), Some(min), Some(max)) => format!(
                "{} ({}-{}, n={})",
                formatter(median),
                formatter(min),
                formatter(max),
                metric.n
            ),
            _ => "n/a".to_string(),
        },
    )
}

fn metric_duration(metric: &Option<MetricSummary>) -> String {
    metric_display(metric, human_duration_value)
}
fn metric_bytes(metric: &Option<MetricSummary>) -> String {
    metric_display(metric, human_bytes_value)
}
fn human_duration(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_string(), human_duration_value)
}

fn human_duration_value(ns: f64) -> String {
    let (value, unit) = if ns >= 1e9 {
        (ns / 1e9, "s")
    } else if ns >= 1e6 {
        (ns / 1e6, "ms")
    } else {
        (ns / 1e3, "µs")
    };
    format!("{value:.3} {unit}")
}

fn human_bytes_value(bytes: f64) -> String {
    let (value, unit) = if bytes >= 1024.0 * 1024.0 * 1024.0 {
        (bytes / 1024.0 / 1024.0 / 1024.0, "GiB")
    } else if bytes >= 1024.0 * 1024.0 {
        (bytes / 1024.0 / 1024.0, "MiB")
    } else {
        (bytes / 1024.0, "KiB")
    };
    format!("{value:.2} {unit}")
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn write_json<T: Serialize>(path: PathBuf, value: &T) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn environment_report(run_id: &str, backend: &str) -> Result<EnvironmentReport> {
    let info = OpenVmBackend::info();
    Ok(EnvironmentReport {
        schema_version: "environment-v2".to_string(),
        run_id: run_id.to_string(),
        git_commit: git_commit(),
        git_dirty: git_dirty(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cpu: cpu_model(),
        logical_cpus: std::thread::available_parallelism().ok().map(usize::from),
        memory_bytes: memory_bytes(),
        kernel: command_version("uname", &["-sr"]),
        rustc_version: command_version("rustc", &["--version"]),
        cargo_version: command_version("cargo", &["--version"]),
        build_profile: build_profile().to_string(),
        backend: backend.to_string(),
        openvm_version: info.version,
        openvm_revision: info.revision,
        security_bits: info.security_bits,
        openvm_config_hash: config_hash(),
        guest_toolchain: info.guest_toolchain,
        rss_scope: "benchmark-case".to_string(),
        rss_includes_keygen: true,
        rss_method: "proc-vmhwm in isolated child process".to_string(),
    })
}

fn build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn command_version(command: &str, args: &[&str]) -> Option<String> {
    Command::new(command)
        .args(args)
        .output()
        .ok()
        .and_then(|output| {
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        })
}
fn config_hash() -> Option<String> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../openvm-backend/guests/m2/openvm.toml");
    let bytes = fs::read(path).ok()?;
    Some(
        blake2b_simd::Params::new()
            .hash_length(32)
            .hash(&bytes)
            .to_hex()
            .to_string(),
    )
}
fn git_commit() -> Option<String> {
    command_version("git", &["rev-parse", "HEAD"])
}
fn git_short() -> String {
    git_commit().map_or_else(
        || "unknown".to_string(),
        |commit| commit[..commit.len().min(7)].to_string(),
    )
}
fn git_dirty() -> bool {
    Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .map_or(true, |output| {
            !output.status.success() || !output.stdout.is_empty()
        })
}
fn utc_run_stamp() -> String {
    Command::new("date")
        .args(["-u", "+%Y%m%d-%H%M%SZ"])
        .output()
        .map_or_else(
            |_| "19700101-000000Z".to_string(),
            |output| String::from_utf8_lossy(&output.stdout).trim().to_string(),
        )
}
fn cpu_model() -> Option<String> {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            contents
                .lines()
                .find(|line| line.starts_with("model name"))
                .map(|line| {
                    line.split_once(':')
                        .map_or(line.to_string(), |(_, value)| value.trim().to_string())
                })
        })
}
fn memory_bytes() -> Option<u64> {
    fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|contents| {
            contents
                .lines()
                .find(|line| line.starts_with("MemTotal:"))
                .and_then(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
                .map(|kb| kb * 1024)
        })
}
fn peak_rss_bytes() -> Option<u64> {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|contents| {
            contents
                .lines()
                .find(|line| line.starts_with("VmHWM:"))
                .and_then(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
                .map(|kb| kb * 1024)
        })
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_environment(dirty: bool) -> EnvironmentReport {
        EnvironmentReport {
            schema_version: "environment-v2".to_string(),
            run_id: "run".to_string(),
            git_commit: Some("commit".to_string()),
            git_dirty: dirty,
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            cpu: None,
            logical_cpus: Some(1),
            memory_bytes: None,
            kernel: None,
            rustc_version: None,
            cargo_version: None,
            build_profile: "release".to_string(),
            backend: "cpu".to_string(),
            openvm_version: "2.0.1".to_string(),
            openvm_revision: "b820b25baab6c5d9b055f64e0286b6b1058e707c".to_string(),
            security_bits: 100,
            openvm_config_hash: Some("hash".to_string()),
            guest_toolchain: "nightly".to_string(),
            rss_scope: "benchmark-case".to_string(),
            rss_includes_keygen: true,
            rss_method: "proc-vmhwm in isolated child process".to_string(),
        }
    }

    fn test_record(sample_index: usize, warmup: bool, success: bool) -> RunRecord {
        RunRecord {
            schema_version: "run-v2".to_string(),
            run_id: "run".to_string(),
            benchmark: "arithmetic".to_string(),
            case: "default".to_string(),
            backend: "cpu".to_string(),
            sample_index,
            warmup,
            success,
            build_time_ns: Some(10),
            transpile_time_ns: Some(20),
            keygen_time_ns: Some(30),
            execute_time_ns: success.then_some(100 + sample_index as u128),
            prove_time_ns: success.then_some(200 + sample_index as u128),
            verify_time_ns: success.then_some(300 + sample_index as u128),
            total_time_ns: 600 + sample_index as u128,
            proof_payload_size_bytes: success.then_some(1_000 + sample_index),
            artifact_size_bytes: success.then_some(2_000 + sample_index),
            serialized_executable_size_bytes: Some(3_000),
            estimated_executable_size_bytes: Some(4_000),
            peak_rss_bytes: success.then_some(5_000),
            openvm_metrics: BTreeMap::new(),
            public_output_hex: success.then(|| "00".to_string()),
            error: (!success).then(|| "failed".to_string()),
        }
    }

    #[test]
    fn median_and_p95_are_deterministic() {
        let metric = metric_summary(vec![1.0, 2.0, 3.0, 4.0, 100.0]);
        assert_eq!(metric.median, Some(3.0));
        assert_eq!(metric.p95, Some(100.0));
    }

    #[test]
    fn branch_case_enumeration_has_all_paths() {
        assert_eq!(
            branch_cases()
                .iter()
                .map(|(case, _, _)| *case)
                .collect::<Vec<_>>(),
            vec!["true", "false", "equal"]
        );
    }

    #[test]
    fn human_units_are_not_raw_ns_or_bytes() {
        assert_eq!(human_duration_value(1_500_000.0), "1.500 ms");
        assert_eq!(human_bytes_value(1024.0 * 1024.0), "1.00 MiB");
    }

    #[test]
    fn summary_aggregates_sizes_and_excludes_warmups() {
        let records = (0..5)
            .map(|index| test_record(index, false, true))
            .chain(std::iter::once(test_record(0, true, true)))
            .collect::<Vec<_>>();
        let refs = records.iter().collect::<Vec<_>>();
        let summary = summarize_case("arithmetic", "default", &refs);
        assert_eq!(summary.samples, 5);
        assert_eq!(
            summary.proof_payload_size_bytes.unwrap().median,
            Some(1_002.0)
        );
        assert_eq!(summary.artifact_size_bytes.unwrap().max, Some(2_004.0));
    }

    #[test]
    fn publication_readiness_rejects_dirty_git_and_failed_samples() {
        let records = (0..5)
            .map(|index| test_record(index, false, index != 4))
            .collect::<Vec<_>>();
        let summary = summarize(
            "run",
            "cpu",
            &records,
            &test_environment(true),
            &BenchmarkOptions::default(),
            MANDATORY_CASES,
        );
        assert!(!summary.publication_ready);
        assert!(summary
            .publication_reasons
            .iter()
            .any(|reason| reason.contains("dirty")));
        assert!(summary
            .publication_reasons
            .iter()
            .any(|reason| reason.contains("failed")));
    }

    #[test]
    fn report_declares_rss_scope_and_size_definitions() {
        let records = (0..5)
            .map(|index| test_record(index, false, true))
            .collect::<Vec<_>>();
        let refs = records.iter().collect::<Vec<_>>();
        let case = summarize_case("arithmetic", "default", &refs);
        let summary = BenchmarkSummary {
            schema_version: "summary-v2".to_string(),
            run_id: "run".to_string(),
            backend: "cpu".to_string(),
            disclaimer: DISCLAIMER.to_string(),
            publication_ready: false,
            publication_reasons: vec![],
            warmup_samples: 0,
            measured_samples: 5,
            cases: vec![case],
        };
        let report = render_report(&test_environment(false), &summary);
        assert!(report.contains("benchmark-case"));
        assert!(report.contains("includes keygen true"));
        assert!(report.contains("Proof Payload"));
        assert!(report.contains("serialized artifact"));
    }

    #[test]
    fn csv_is_one_row_per_case() {
        let summary = BenchmarkSummary {
            schema_version: "summary-v2".to_string(),
            run_id: "run".to_string(),
            backend: "cpu".to_string(),
            disclaimer: DISCLAIMER.to_string(),
            publication_ready: false,
            publication_reasons: vec!["test".to_string()],
            warmup_samples: 1,
            measured_samples: 1,
            cases: vec![],
        };
        let path = std::env::temp_dir().join(format!("zk-jam-csv-{}.csv", std::process::id()));
        write_csv(path.clone(), &summary).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 1);
        fs::remove_file(path).unwrap();
    }
}
