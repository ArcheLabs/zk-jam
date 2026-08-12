//! Machine-readable M2 benchmark orchestration and report generation.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

use eyre::{eyre, Result, WrapErr};
use serde::{Deserialize, Serialize};
use zk_jam_openvm_backend::{M2Benchmark, M2Input, OpenVmBackend};

pub use zk_jam_refine_interface::RefineCaseV1;

pub const DISCLAIMER: &str = "These results measure the OpenVM proving substrate and ZK-JAM integration only. They do not yet measure PVM Translation, PVM memory emulation, Refine Host Calls, or Native PVM proving.";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnvironmentReport {
    pub schema_version: String,
    pub run_id: String,
    pub git_commit: Option<String>,
    pub os: String,
    pub arch: String,
    pub cpu: Option<String>,
    pub logical_cpus: Option<usize>,
    pub memory_bytes: Option<u64>,
    pub backend: String,
    pub openvm_version: String,
    pub openvm_revision: String,
    pub security_bits: u32,
    pub guest_toolchain: String,
    pub peak_rss_methodology: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunRecord {
    pub schema_version: String,
    pub run_id: String,
    pub benchmark: String,
    pub backend: String,
    pub sample_index: usize,
    pub warmup: bool,
    pub build_time_ns: Option<u128>,
    pub transpile_time_ns: Option<u128>,
    pub keygen_time_ns: Option<u128>,
    pub execute_time_ns: Option<u128>,
    pub prove_time_ns: Option<u128>,
    pub verify_time_ns: Option<u128>,
    pub total_time_ns: u128,
    pub executable_size_bytes: Option<usize>,
    pub proof_size_bytes: Option<usize>,
    pub peak_rss_bytes: Option<u64>,
    pub openvm_metrics: BTreeMap<String, serde_json::Value>,
    pub public_output_hex: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
pub struct BenchmarkSummary {
    pub schema_version: String,
    pub run_id: String,
    pub backend: String,
    pub disclaimer: String,
    pub metrics_ns: BTreeMap<String, MetricSummary>,
    pub sample_counts: BTreeMap<String, usize>,
}

#[derive(Clone, Debug)]
pub struct BenchmarkRun {
    pub run_id: String,
    pub result_dir: PathBuf,
    pub summary: BenchmarkSummary,
}

pub fn run_m2(
    output_root: &Path,
    selected: Option<&str>,
    backend_name: &str,
) -> Result<BenchmarkRun> {
    if backend_name != "cpu" {
        return Err(eyre!("M2 currently supports --backend cpu only"));
    }
    let backend = OpenVmBackend;
    let benchmarks = match selected {
        Some("arithmetic") => vec![M2Benchmark::Arithmetic],
        Some("branch") => vec![M2Benchmark::Branch],
        Some("memory") => vec![
            M2Benchmark::Memory { bytes: 1024 },
            M2Benchmark::Memory { bytes: 16 * 1024 },
            M2Benchmark::Memory { bytes: 256 * 1024 },
        ],
        Some(other) => return Err(eyre!("unknown M2 benchmark: {other}")),
        None => vec![
            M2Benchmark::Arithmetic,
            M2Benchmark::Branch,
            M2Benchmark::Memory { bytes: 1024 },
            M2Benchmark::Memory { bytes: 16 * 1024 },
            M2Benchmark::Memory { bytes: 256 * 1024 },
        ],
    };

    let run_id = format!("{}_{}_{}", utc_run_stamp(), git_short(), backend_name);
    let result_dir = output_root.join(&run_id);
    if result_dir.exists() {
        return Err(eyre!(
            "refusing to overwrite existing benchmark run {run_id}"
        ));
    }
    fs::create_dir_all(result_dir.join("artifacts"))?;

    let info = OpenVmBackend::info();
    let environment = EnvironmentReport {
        schema_version: "environment-v1".to_string(),
        run_id: run_id.clone(),
        git_commit: git_commit(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cpu: cpu_model(),
        logical_cpus: std::thread::available_parallelism().ok().map(usize::from),
        memory_bytes: memory_bytes(),
        backend: backend_name.to_string(),
        openvm_version: info.version,
        openvm_revision: info.revision,
        security_bits: info.security_bits,
        guest_toolchain: info.guest_toolchain,
        peak_rss_methodology:
            "Linux /proc self VmHWM sampled after each case; null when unavailable".to_string(),
    };
    write_json(result_dir.join("environment.json"), &environment)?;

    let mut records = Vec::new();
    for benchmark in benchmarks {
        let input = match &benchmark {
            M2Benchmark::Arithmetic => M2Input::arithmetic(7, 9),
            M2Benchmark::Branch => M2Input::branch(21, 8),
            M2Benchmark::Memory { bytes } => M2Input::memory(0x1234_5678, *bytes)?,
        };
        let program = backend.program(benchmark.clone())?;

        // One warmup sample is required and is never included in summary statistics.
        let _ = backend.execute(&program, input)?;
        for sample_index in 0..10 {
            let started = Instant::now();
            let execution = backend.execute(&program, input)?;
            let execute_time_ns = execution.elapsed_ns;
            let proof = backend.prove(&program, input)?;
            let verify_started = Instant::now();
            proof.verify(&input.context_hash(&benchmark))?;
            let verify_time_ns = verify_started.elapsed().as_nanos();
            let total_time_ns = started.elapsed().as_nanos();
            let proof_bytes = proof.to_bytes()?;
            let proof_size_bytes = proof_bytes.len();
            fs::write(
                result_dir
                    .join("artifacts")
                    .join(format!("{}-{sample_index}.proof.json", benchmark.label())),
                &proof_bytes,
            )?;
            records.push(RunRecord {
                schema_version: "run-v1".to_string(),
                run_id: run_id.clone(),
                benchmark: benchmark.label(),
                backend: backend_name.to_string(),
                sample_index,
                warmup: false,
                build_time_ns: Some(program.build_time_ns),
                transpile_time_ns: Some(program.transpile_time_ns),
                keygen_time_ns: Some(proof.keygen_time_ns),
                execute_time_ns: Some(execute_time_ns),
                prove_time_ns: Some(proof.prove_time_ns),
                verify_time_ns: Some(verify_time_ns),
                total_time_ns,
                executable_size_bytes: Some(execution.executable_bytes),
                proof_size_bytes: Some(proof_size_bytes),
                peak_rss_bytes: peak_rss_bytes(),
                openvm_metrics: BTreeMap::new(),
                public_output_hex: Some(hex(&execution.public_output)),
                error: None,
            });
        }
    }

    let jsonl = records
        .iter()
        .map(serde_json::to_string)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .join("\n");
    fs::write(result_dir.join("runs.jsonl"), format!("{jsonl}\n"))?;
    let summary = summarize(&run_id, backend_name, &records);
    write_json(result_dir.join("summary.json"), &summary)?;
    write_csv(result_dir.join("summary.csv"), &summary)?;
    let report = render_report(&environment, &summary);
    fs::write(result_dir.join("report.md"), report)?;

    let public_dir = output_root.join("../public").join(&run_id);
    fs::create_dir_all(&public_dir)?;
    write_json(public_dir.join("environment.json"), &environment)?;
    write_json(public_dir.join("summary.json"), &summary)?;
    write_csv(public_dir.join("summary.csv"), &summary)?;
    fs::write(
        public_dir.join("report.md"),
        render_report(&environment, &summary),
    )?;

    Ok(BenchmarkRun {
        run_id,
        result_dir,
        summary,
    })
}

pub fn report_run(results_root: &Path, run_id: &str) -> Result<String> {
    let path = results_root.join(run_id).join("report.md");
    fs::read_to_string(&path).wrap_err_with(|| format!("read report {}", path.display()))
}

fn summarize(run_id: &str, backend: &str, records: &[RunRecord]) -> BenchmarkSummary {
    let mut values: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut counts = BTreeMap::new();
    for record in records.iter().filter(|record| !record.warmup) {
        *counts.entry(record.benchmark.clone()).or_insert(0) += 1;
        values
            .entry(format!("{}.execute_time_ns", record.benchmark))
            .or_default()
            .push(record.execute_time_ns.unwrap_or_default() as f64);
        values
            .entry(format!("{}.prove_time_ns", record.benchmark))
            .or_default()
            .push(record.prove_time_ns.unwrap_or_default() as f64);
        values
            .entry(format!("{}.verify_time_ns", record.benchmark))
            .or_default()
            .push(record.verify_time_ns.unwrap_or_default() as f64);
        values
            .entry(format!("{}.total_time_ns", record.benchmark))
            .or_default()
            .push(record.total_time_ns as f64);
    }
    let metrics_ns = values
        .into_iter()
        .map(|(name, mut values)| {
            values.sort_by(f64::total_cmp);
            let n = values.len();
            let mean = values.iter().sum::<f64>() / n.max(1) as f64;
            let variance = values
                .iter()
                .map(|value| (value - mean).powi(2))
                .sum::<f64>()
                / n.max(1) as f64;
            let percentile = |p: f64| values[((n.saturating_sub(1)) as f64 * p).round() as usize];
            (
                name,
                MetricSummary {
                    n,
                    min: values.first().copied(),
                    max: values.last().copied(),
                    mean: (n > 0).then_some(mean),
                    median: (n > 0).then_some(percentile(0.5)),
                    p95: (n > 0).then_some(percentile(0.95)),
                    stddev: (n > 0).then_some(variance.sqrt()),
                },
            )
        })
        .collect();
    BenchmarkSummary {
        schema_version: "summary-v1".to_string(),
        run_id: run_id.to_string(),
        backend: backend.to_string(),
        disclaimer: DISCLAIMER.to_string(),
        metrics_ns,
        sample_counts: counts,
    }
}

fn render_report(environment: &EnvironmentReport, summary: &BenchmarkSummary) -> String {
    let mut report = format!(
        "# ZK-JAM M2 OpenVM baseline\n\nRun: `{}`\n\n",
        summary.run_id
    );
    report.push_str(&format!("- OpenVM: `{}` at `{}`\n- Backend: `{}`\n- Security target: {} bits\n- Guest toolchain: `{}`\n\n", environment.openvm_version, environment.openvm_revision, environment.backend, environment.security_bits, environment.guest_toolchain));
    report.push_str("| Metric | n | min ns | mean ns | median ns | p95 ns | max ns | stddev ns |\n|---|---:|---:|---:|---:|---:|---:|---:|\n");
    for (name, metric) in &summary.metrics_ns {
        report.push_str(&format!(
            "| {name} | {} | {} | {} | {} | {} | {} | {} |\n",
            metric.n,
            display(metric.min),
            display(metric.mean),
            display(metric.median),
            display(metric.p95),
            display(metric.max),
            display(metric.stddev)
        ));
    }
    report.push_str(&format!("\n> {}\n", DISCLAIMER));
    report
}

fn display(value: Option<f64>) -> String {
    value.map_or_else(|| "null".to_string(), |value| format!("{value:.0}"))
}

fn write_json<T: Serialize>(path: PathBuf, value: &T) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn write_csv(path: PathBuf, summary: &BenchmarkSummary) -> Result<()> {
    let mut csv = String::from("metric,n,min_ns,max_ns,mean_ns,median_ns,p95_ns,stddev_ns\n");
    for (name, metric) in &summary.metrics_ns {
        csv.push_str(&format!(
            "{name},{},{},{},{},{},{},{}\n",
            metric.n,
            display(metric.min),
            display(metric.max),
            display(metric.mean),
            display(metric.median),
            display(metric.p95),
            display(metric.stddev)
        ));
    }
    fs::write(path, csv)?;
    Ok(())
}

fn utc_run_stamp() -> String {
    let output = Command::new("date")
        .args(["-u", "+%Y%m%d-%H%M%SZ"])
        .output();
    output.map_or_else(
        |_| "19700101-000000Z".to_string(),
        |output| String::from_utf8_lossy(&output.stdout).trim().to_string(),
    )
}

fn git_commit() -> Option<String> {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        })
}

fn git_short() -> String {
    git_commit().map_or_else(
        || "unknown".to_string(),
        |commit| commit[..commit.len().min(7)].to_string(),
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
