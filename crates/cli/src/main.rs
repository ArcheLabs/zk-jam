use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};
use zk_jam_benchmark::{
    aggregate_m4_1_publication, aggregate_m4_publication, aggregate_m4_reports,
    aggregate_pvm_openvm, report_run, run_m2, run_m3, run_m3_worker, run_m4_1_preflight,
    run_m4_1_publication_workload, run_m4_preflight, run_m4_proof_program, run_m4_publication,
    run_m4_publication_worker, run_m4_publication_workload, run_pvm_openvm_preflight,
    run_pvm_openvm_worker, run_pvm_openvm_workload_filtered, run_worker, validate_m3_report,
    validate_m4_preflight_report, validate_m4_proof_partial_report, validate_m4_publication_report,
    validate_m4_report, validate_pvm_openvm_report, verify_jambda_provenance, BenchmarkOptions,
    M4ProgramId,
};
use zk_jam_openvm_backend::{M2Benchmark, M2Input, OpenVmBackend};
use zk_jam_refine_interface::{
    CanonicalCodec, PvmBlockV1, PvmInstructionV1, PvmProgramV1, PvmTerminatorV1, RefineCaseV1,
    RefineStateWitnessV1, RegisterOperandsV1, SmokeProfile, StateWitnessBindingV1,
    PVM_PROGRAM_FORMAT_V1, REFINE_CASE_FORMAT_V1,
};

fn usage() {
    eprintln!("usage: zk-jam inspect <case.bin> [--json]");
    eprintln!("       zk-jam make-minimal <case.bin>");
    eprintln!("       zk-jam openvm info");
    eprintln!("       zk-jam openvm execute arithmetic|branch|memory");
    eprintln!("       zk-jam openvm prove arithmetic|branch|memory");
    eprintln!("       zk-jam openvm verify <artifact.json>");
    eprintln!("       zk-jam bench m2 --backend cpu [--benchmark arithmetic|branch|memory] [--samples N] [--warmup N] [--quick] [--output benchmarks/results]");
    eprintln!("       zk-jam bench report <run-id> [--output benchmarks/results]");
    eprintln!("       zk-jam bench verify-jambda --repo <checkout> [--manifest integration/jambda-m3.json]");
    eprintln!("       zk-jam bench validate-m3 <report.json> [--schema schema.json]");
    eprintln!("       zk-jam bench validate-m4 <report.json> [--schema schema.json]");
    eprintln!("       zk-jam bench validate-m4-preflight <report.json> [--schema schema.json]");
    eprintln!("       zk-jam bench validate-m4-proof <report.json> [--schema schema.json]");
    eprintln!("       zk-jam bench validate-m4-publication <report.json> [--schema schema.json]");
    eprintln!("       zk-jam bench m3 --jambda-repo <checkout> [--samples N] [--warmup N] [--output benchmarks/results]");
    eprintln!("       zk-jam bench m4 --execute-only --jambda-repo <checkout> [--samples 1] [--warmup 0] [--output benchmarks/results]");
    eprintln!("       zk-jam bench m4-proof --program arithmetic|branch|memory --jambda-repo <checkout> [--output benchmarks/results]");
    eprintln!("       zk-jam bench aggregate-m4 --preflight <report.json> --proof-arithmetic <report.json> --proof-branch <report.json> --proof-memory <report.json> [--output benchmarks/results]");
    eprintln!("       zk-jam bench m4-publication-workload --workload arithmetic|branch|memory --m4-report <report.json> [--output benchmarks/results]");
    eprintln!("       zk-jam bench pvm-openvm-workload --workload arithmetic|branch|memory --semantic-gate <report.json> [--only direct_openvm_guest|generated_guest|direct_pvm_lowering] [--output benchmarks/results]");
    eprintln!("       zk-jam bench aggregate-m4-publication --m4-report <report.json> --partial-arithmetic <report.json> --partial-branch <report.json> --partial-memory <report.json> [--output benchmarks/results]");
    eprintln!("       zk-jam bench m4-publication --m4-report <report.json> [--output benchmarks/results]");
    eprintln!("       zk-jam bench m4.1-preflight [--output benchmarks/results]");
    eprintln!("       zk-jam bench m4.1-publication-workload --workload arithmetic|branch|memory --preflight <report.json> [--output benchmarks/results]");
    eprintln!("       zk-jam bench aggregate-m4.1-publication --partial-arithmetic <report.json> --partial-branch <report.json> --partial-memory <report.json> [--output benchmarks/results]");
    eprintln!("       zk-jam bench pvm-openvm-preflight [--output benchmarks/results]");
    eprintln!("       zk-jam bench pvm-openvm-workload --workload arithmetic|branch|memory [--semantic-gate <report.json>] [--output benchmarks/results]");
    eprintln!("       zk-jam bench pvm-openvm-aggregate --semantic-gate <report.json> --partial-arithmetic <report.json> --partial-branch <report.json> --partial-memory <report.json> [--output benchmarks/results]");
    eprintln!("       zk-jam bench validate-pvm-openvm <report.json> [--schema schema.json]");
}

fn benchmark(name: &str) -> Result<(M2Benchmark, M2Input), Box<dyn std::error::Error>> {
    Ok(match name {
        "arithmetic" => (M2Benchmark::Arithmetic, M2Input::arithmetic(7, 9)),
        "branch" => (M2Benchmark::Branch, M2Input::branch(21, 8)),
        "memory" => (
            M2Benchmark::Memory { bytes: 1024 },
            M2Input::memory(0x1234_5678, 1024)?,
        ),
        other => return Err(format!("unknown OpenVM benchmark: {other}").into()),
    })
}

fn openvm_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let backend = OpenVmBackend;
    match args {
        [command, subcommand] if command == "openvm" && subcommand == "info" => {
            println!("{}", serde_json::to_string_pretty(&OpenVmBackend::info())?);
        }
        [command, action, name]
            if command == "openvm" && (action == "execute" || action == "prove") =>
        {
            let (benchmark, input) = benchmark(name)?;
            let program = backend.program(benchmark)?;
            if action == "execute" {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&backend.execute(&program, input)?)?
                );
            } else {
                let artifact = backend.prove(&program, input)?;
                let path = PathBuf::from(format!("{name}.proof.json"));
                fs::write(&path, artifact.to_bytes()?)?;
                println!("proof: {}", path.display());
                println!(
                    "verified: {}",
                    artifact
                        .verify(&input.context_hash(&artifact.benchmark))
                        .is_ok()
                );
            }
        }
        [command, action, path] if command == "openvm" && action == "verify" => {
            let bytes = fs::read(path)?;
            let artifact = zk_jam_openvm_backend::OpenVmProofArtifact::from_bytes(&bytes)?;
            let (_, input) = benchmark(artifact.benchmark.name())?;
            artifact.verify(&input.context_hash(&artifact.benchmark))?;
            println!("verified: true");
        }
        _ => return Err("invalid openvm command".into()),
    }
    Ok(())
}

fn bench_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.first().map(String::as_str) != Some("bench") {
        return Err("invalid bench command".into());
    }
    match args.get(1).map(String::as_str) {
        Some("m2") => {
            let mut backend = "cpu";
            let mut benchmark = None;
            let mut output = PathBuf::from("benchmarks/results");
            let mut samples = 10;
            let mut warmup = 1;
            let mut quick = false;
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--backend" => {
                        backend = args.get(index + 1).ok_or("missing --backend value")?;
                        index += 2;
                    }
                    "--benchmark" => {
                        benchmark = Some(
                            args.get(index + 1)
                                .ok_or("missing --benchmark value")?
                                .as_str(),
                        );
                        index += 2;
                    }
                    "--output" => {
                        output =
                            PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                        index += 2;
                    }
                    "--samples" => {
                        samples = args
                            .get(index + 1)
                            .ok_or("missing --samples value")?
                            .parse::<usize>()?;
                        index += 2;
                    }
                    "--warmup" => {
                        warmup = args
                            .get(index + 1)
                            .ok_or("missing --warmup value")?
                            .parse::<usize>()?;
                        index += 2;
                    }
                    "--quick" => {
                        quick = true;
                        index += 1;
                    }
                    other => return Err(format!("unknown bench option: {other}").into()),
                }
            }
            let run = run_m2(
                &output,
                benchmark,
                backend,
                BenchmarkOptions {
                    samples,
                    warmup,
                    quick,
                },
            )?;
            println!(
                "run_id: {}\nresult_dir: {}",
                run.run_id,
                run.result_dir.display()
            );
        }
        Some("report") => {
            let run_id = args.get(2).ok_or("missing run id")?;
            let output = args
                .iter()
                .position(|arg| arg == "--output")
                .and_then(|index| args.get(index + 1))
                .map_or_else(
                    || PathBuf::from("benchmarks/results"),
                    |value| PathBuf::from(value.as_str()),
                );
            print!("{}", report_run(&output, run_id)?);
        }
        Some("validate-m3") => {
            let report = PathBuf::from(args.get(2).ok_or("missing M3 report path")?);
            let schema = args
                .iter()
                .position(|arg| arg == "--schema")
                .and_then(|index| args.get(index + 1))
                .map_or_else(
                    || {
                        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                            .join("../../benchmarks/schema/m3-paired-v2.schema.json")
                    },
                    PathBuf::from,
                );
            validate_m3_report(&report, &schema)?;
            println!("M3 schema valid: {}", report.display());
        }
        Some("validate-m4") => {
            let report = PathBuf::from(args.get(2).ok_or("missing M4 report path")?);
            let schema = args
                .iter()
                .position(|arg| arg == "--schema")
                .and_then(|index| args.get(index + 1))
                .map_or_else(
                    || {
                        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                            .join("../../benchmarks/schema/m4-proven-translation-v1.schema.json")
                    },
                    PathBuf::from,
                );
            validate_m4_report(&report, &schema)?;
            println!("M4 schema valid: {}", report.display());
        }
        Some("validate-m4-preflight") => {
            let report = PathBuf::from(args.get(2).ok_or("missing M4 preflight report path")?);
            let schema = args
                .iter()
                .position(|arg| arg == "--schema")
                .and_then(|index| args.get(index + 1))
                .map_or_else(
                    || {
                        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                            .join("../../benchmarks/schema/m4-preflight-v1.schema.json")
                    },
                    PathBuf::from,
                );
            validate_m4_preflight_report(&report, &schema)?;
            println!("M4 preflight schema valid: {}", report.display());
        }
        Some("validate-m4-proof") => {
            let report = PathBuf::from(args.get(2).ok_or("missing M4 proof partial report path")?);
            let schema = args
                .iter()
                .position(|arg| arg == "--schema")
                .and_then(|index| args.get(index + 1))
                .map_or_else(
                    || {
                        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                            .join("../../benchmarks/schema/m4-proof-partial-v1.schema.json")
                    },
                    PathBuf::from,
                );
            validate_m4_proof_partial_report(&report, &schema)?;
            println!("M4 proof partial schema valid: {}", report.display());
        }
        Some("validate-m4-publication") => {
            let report = PathBuf::from(args.get(2).ok_or("missing M4 publication report path")?);
            let schema = args
                .iter()
                .position(|arg| arg == "--schema")
                .and_then(|index| args.get(index + 1))
                .map_or_else(
                    || {
                        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                            .join("../../benchmarks/schema/m4-publication-v1.schema.json")
                    },
                    PathBuf::from,
                );
            validate_m4_publication_report(&report, &schema)?;
            println!("M4 publication schema valid: {}", report.display());
        }
        Some("verify-jambda") => {
            let repo = args
                .iter()
                .position(|arg| arg == "--repo")
                .and_then(|index| args.get(index + 1))
                .map(PathBuf::from)
                .ok_or("missing --repo")?;
            let manifest = args
                .iter()
                .position(|arg| arg == "--manifest")
                .and_then(|index| args.get(index + 1))
                .map_or_else(
                    || {
                        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                            .join("../../integration/jambda-m3.json")
                    },
                    PathBuf::from,
                );
            let provenance = verify_jambda_provenance(&repo, &manifest)?;
            println!(
                "Jambda provenance verified: {}@{}",
                provenance.repository, provenance.revision
            );
        }
        Some("m3") => {
            let mut output = PathBuf::from("benchmarks/results");
            let mut samples = 5;
            let mut warmup = 1;
            let mut jambda_repo = None;
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--output" => {
                        output =
                            PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                        index += 2;
                    }
                    "--samples" => {
                        samples = args
                            .get(index + 1)
                            .ok_or("missing --samples value")?
                            .parse()?;
                        index += 2;
                    }
                    "--warmup" => {
                        warmup = args
                            .get(index + 1)
                            .ok_or("missing --warmup value")?
                            .parse()?;
                        index += 2;
                    }
                    "--jambda-repo" => {
                        jambda_repo = Some(PathBuf::from(
                            args.get(index + 1).ok_or("missing --jambda-repo value")?,
                        ));
                        index += 2;
                    }
                    other => return Err(format!("unknown bench option: {other}").into()),
                }
            }
            let jambda_repo = jambda_repo.ok_or("M3 requires --jambda-repo")?;
            let report = run_m3(&output, samples, warmup, &jambda_repo)?;
            println!(
                "M3 complete: {}\nM3 publication ready: {}",
                report.complete, report.publication_ready
            );
        }
        Some("m4-proof") => {
            let mut output = PathBuf::from("benchmarks/results");
            let mut jambda_repo = None;
            let mut program = None;
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--output" => {
                        output = PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                        index += 2;
                    }
                    "--jambda-repo" => {
                        jambda_repo = Some(PathBuf::from(args.get(index + 1).ok_or("missing --jambda-repo value")?));
                        index += 2;
                    }
                    "--program" => {
                        program = Some(match args.get(index + 1).ok_or("missing --program value")?.as_str() {
                            "arithmetic" => M4ProgramId::Arithmetic,
                            "branch" => M4ProgramId::Branch,
                            "memory" | "memory-16384" => M4ProgramId::Memory16K,
                            other => return Err(format!("unknown M4 program: {other}").into()),
                        });
                        index += 2;
                    }
                    other => return Err(format!("unknown bench option: {other}").into()),
                }
            }
            let report = run_m4_proof_program(
                &output,
                &jambda_repo.ok_or("M4 proof requires --jambda-repo")?,
                program.ok_or("M4 proof requires --program")?,
            )?;
            println!("M4 proof program {} complete: {}", report.program, report.complete);
            if !report.complete {
                return Err("M4 proof program failed".into());
            }
        }
        Some("aggregate-m4") => {
            let mut output = PathBuf::from("benchmarks/results");
            let mut preflight = None;
            let mut proofs = [None, None, None];
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--output" => {
                        output = PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                        index += 2;
                    }
                    "--preflight" => {
                        preflight = Some(PathBuf::from(args.get(index + 1).ok_or("missing --preflight value")?));
                        index += 2;
                    }
                    "--proof-arithmetic" => {
                        proofs[0] = Some(PathBuf::from(args.get(index + 1).ok_or("missing --proof-arithmetic value")?));
                        index += 2;
                    }
                    "--proof-branch" => {
                        proofs[1] = Some(PathBuf::from(args.get(index + 1).ok_or("missing --proof-branch value")?));
                        index += 2;
                    }
                    "--proof-memory" => {
                        proofs[2] = Some(PathBuf::from(args.get(index + 1).ok_or("missing --proof-memory value")?));
                        index += 2;
                    }
                    other => return Err(format!("unknown bench option: {other}").into()),
                }
            }
            let report = aggregate_m4_reports(
                &output,
                &preflight.ok_or("M4 aggregate requires --preflight")?,
                &proofs
                    .into_iter()
                    .map(|path| path.ok_or("M4 aggregate requires all three proof reports"))
                    .collect::<Result<Vec<_>, _>>()?,
            )?;
            println!("M4 aggregate complete: {}\nM4 publication ready: {}", report.complete, report.publication_ready);
            if !report.complete {
                return Err("M4 aggregate failed".into());
            }
        }
        Some("m4-publication-workload") => {
            let mut output = PathBuf::from("benchmarks/results");
            let mut m4_report = None;
            let mut workload = None;
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--output" => {
                        output = PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                        index += 2;
                    }
                    "--m4-report" => {
                        m4_report = Some(PathBuf::from(args.get(index + 1).ok_or("missing --m4-report value")?));
                        index += 2;
                    }
                    "--workload" => {
                        workload = Some(match args.get(index + 1).ok_or("missing --workload value")?.as_str() {
                            "arithmetic" => M4ProgramId::Arithmetic,
                            "branch" => M4ProgramId::Branch,
                            "memory" | "memory-16384" => M4ProgramId::Memory16K,
                            other => return Err(format!("unknown M4 publication workload: {other}").into()),
                        });
                        index += 2;
                    }
                    other => return Err(format!("unknown bench option: {other}").into()),
                }
            }
            let report = run_m4_publication_workload(
                &output,
                &m4_report.ok_or("M4 publication workload requires --m4-report")?,
                workload.ok_or("M4 publication workload requires --workload")?,
            )?;
            println!("M4 publication workload {} complete: {}", report.name, report.semantics_match);
        }
        Some("aggregate-m4-publication") => {
            let mut output = PathBuf::from("benchmarks/results");
            let mut m4_report = None;
            let mut partials = [None, None, None];
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--output" => {
                        output = PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                        index += 2;
                    }
                    "--m4-report" => {
                        m4_report = Some(PathBuf::from(args.get(index + 1).ok_or("missing --m4-report value")?));
                        index += 2;
                    }
                    "--partial-arithmetic" => {
                        partials[0] = Some(PathBuf::from(args.get(index + 1).ok_or("missing --partial-arithmetic value")?));
                        index += 2;
                    }
                    "--partial-branch" => {
                        partials[1] = Some(PathBuf::from(args.get(index + 1).ok_or("missing --partial-branch value")?));
                        index += 2;
                    }
                    "--partial-memory" => {
                        partials[2] = Some(PathBuf::from(args.get(index + 1).ok_or("missing --partial-memory value")?));
                        index += 2;
                    }
                    other => return Err(format!("unknown bench option: {other}").into()),
                }
            }
            let partial_arithmetic = partials[0].as_deref().ok_or("missing --partial-arithmetic")?;
            let partial_branch = partials[1].as_deref().ok_or("missing --partial-branch")?;
            let partial_memory = partials[2].as_deref().ok_or("missing --partial-memory")?;
            let report = aggregate_m4_publication(
                &output,
                &m4_report.ok_or("M4 publication aggregate requires --m4-report")?,
                [partial_arithmetic, partial_branch, partial_memory],
            )?;
            println!("M4 publication status: {}\nM4 comparison complete: {}", report.comparison_status, report.comparison_complete);
        }
        Some("m4-publication") => {
            let mut output = PathBuf::from("benchmarks/results");
            let mut m4_report = None;
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--output" => {
                        output = PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                        index += 2;
                    }
                    "--m4-report" => {
                        m4_report = Some(PathBuf::from(args.get(index + 1).ok_or("missing --m4-report value")?));
                        index += 2;
                    }
                    other => return Err(format!("unknown bench option: {other}").into()),
                }
            }
            let report = run_m4_publication(&output, &m4_report.ok_or("M4 publication requires --m4-report")?)?;
            println!("M4 publication status: {}\nM4 comparison complete: {}", report.comparison_status, report.comparison_complete);
        }
        Some("pvm-openvm-preflight") => {
            let mut output = PathBuf::from("benchmarks/results");
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--output" => {
                        output = PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                        index += 2;
                    }
                    other => return Err(format!("unknown PVM -> OpenVM preflight option: {other}").into()),
                }
            }
            let report = run_pvm_openvm_preflight(&output)?;
            println!("PVM -> OpenVM semantic gate complete: {}", report.complete);
            if !report.complete {
                return Err("PVM -> OpenVM semantic gate failed".into());
            }
        }
        Some("pvm-openvm-workload") => {
            let mut output = PathBuf::from("benchmarks/results");
            let mut semantic_gate = None;
            let mut workload = None;
            let mut only = None;
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--output" => {
                        output = PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                        index += 2;
                    }
                    "--semantic-gate" => {
                        semantic_gate = Some(PathBuf::from(args.get(index + 1).ok_or("missing --semantic-gate value")?));
                        index += 2;
                    }
                    "--workload" => {
                        workload = Some(args.get(index + 1).ok_or("missing --workload value")?.clone());
                        index += 2;
                    }
                    "--only" => {
                        only = Some(args.get(index + 1).ok_or("missing --only value")?.clone());
                        index += 2;
                    }
                    other => return Err(format!("unknown PVM -> OpenVM workload option: {other}").into()),
                }
            }
            let workload = workload.ok_or("PVM -> OpenVM workload requires --workload")?;
            let workload = match workload.as_str() {
                "arithmetic" => M4ProgramId::Arithmetic,
                "branch" => M4ProgramId::Branch,
                "memory" | "memory-16k" => M4ProgramId::Memory16K,
                other => return Err(format!("unknown PVM -> OpenVM workload: {other}").into()),
            };
            let gate = semantic_gate.ok_or("PVM -> OpenVM workload requires --semantic-gate")?;
            let report = run_pvm_openvm_workload_filtered(
                &output,
                &gate,
                workload,
                only.as_deref(),
            )?;
            println!("PVM -> OpenVM workload {} complete: {}", report.workload, report.semantics_match);
            if !report.semantics_match {
                return Err("PVM -> OpenVM workload failed".into());
            }
        }
        Some("pvm-openvm-aggregate") => {
            let mut output = PathBuf::from("benchmarks/results");
            let mut semantic_gate = None;
            let mut partials = [None, None, None];
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--output" => {
                        output = PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                        index += 2;
                    }
                    "--semantic-gate" => {
                        semantic_gate = Some(PathBuf::from(args.get(index + 1).ok_or("missing --semantic-gate value")?));
                        index += 2;
                    }
                    "--partial-arithmetic" => {
                        partials[0] = Some(PathBuf::from(args.get(index + 1).ok_or("missing --partial-arithmetic value")?));
                        index += 2;
                    }
                    "--partial-branch" => {
                        partials[1] = Some(PathBuf::from(args.get(index + 1).ok_or("missing --partial-branch value")?));
                        index += 2;
                    }
                    "--partial-memory" => {
                        partials[2] = Some(PathBuf::from(args.get(index + 1).ok_or("missing --partial-memory value")?));
                        index += 2;
                    }
                    other => return Err(format!("unknown PVM -> OpenVM aggregate option: {other}").into()),
                }
            }
            let report = aggregate_pvm_openvm(
                &output,
                &semantic_gate.ok_or("PVM -> OpenVM aggregate requires --semantic-gate")?,
                [
                    partials[0].as_deref().ok_or("missing --partial-arithmetic")?,
                    partials[1].as_deref().ok_or("missing --partial-branch")?,
                    partials[2].as_deref().ok_or("missing --partial-memory")?,
                ],
            )?;
            println!("PVM -> OpenVM status: {}\ncomparison complete: {}", report.comparison_status, report.comparison_complete);
        }
        Some("validate-pvm-openvm") => {
            let report = PathBuf::from(args.get(2).ok_or("missing PVM -> OpenVM report path")?);
            let schema = args.iter().position(|arg| arg == "--schema")
                .and_then(|index| args.get(index + 1))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("benchmarks/schema/pvm-openvm-benchmark-v1.schema.json"));
            validate_pvm_openvm_report(&report, &schema)?;
            println!("PVM -> OpenVM report valid: {}", report.display());
        }
        Some("m4.1-preflight") => {
            let mut output = PathBuf::from("benchmarks/results");
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--output" => {
                        output = PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                        index += 2;
                    }
                    other => return Err(format!("unknown M4.1 preflight option: {other}").into()),
                }
            }
            let report = run_m4_1_preflight(&output)?;
            println!("M4.1 execute-only preflight complete: {}", report.complete);
            if !report.complete {
                return Err("M4.1 execute-only preflight failed".into());
            }
        }
        Some("m4.1-publication-workload") => {
            let mut output = PathBuf::from("benchmarks/results");
            let mut preflight = None;
            let mut workload = None;
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--output" => {
                        output = PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                        index += 2;
                    }
                    "--preflight" => {
                        preflight = Some(PathBuf::from(args.get(index + 1).ok_or("missing --preflight value")?));
                        index += 2;
                    }
                    "--workload" => {
                        workload = Some(match args.get(index + 1).ok_or("missing --workload value")?.as_str() {
                            "arithmetic" => M4ProgramId::Arithmetic,
                            "branch" => M4ProgramId::Branch,
                            "memory" => M4ProgramId::Memory16K,
                            other => return Err(format!("unknown M4.1 workload: {other}").into()),
                        });
                        index += 2;
                    }
                    other => return Err(format!("unknown M4.1 publication option: {other}").into()),
                }
            }
            let report = run_m4_1_publication_workload(
                &output,
                &preflight.ok_or("M4.1 publication requires --preflight")?,
                workload.ok_or("M4.1 publication requires --workload")?,
            )?;
            println!("M4.1 workload {} complete: {}", report.workload, report.semantics_match);
        }
        Some("aggregate-m4.1-publication") => {
            let mut output = PathBuf::from("benchmarks/results");
            let mut partials = [None, None, None];
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--output" => {
                        output = PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                        index += 2;
                    }
                    "--partial-arithmetic" => {
                        partials[0] = Some(PathBuf::from(args.get(index + 1).ok_or("missing --partial-arithmetic value")?));
                        index += 2;
                    }
                    "--partial-branch" => {
                        partials[1] = Some(PathBuf::from(args.get(index + 1).ok_or("missing --partial-branch value")?));
                        index += 2;
                    }
                    "--partial-memory" => {
                        partials[2] = Some(PathBuf::from(args.get(index + 1).ok_or("missing --partial-memory value")?));
                        index += 2;
                    }
                    other => return Err(format!("unknown M4.1 aggregate option: {other}").into()),
                }
            }
            let report = aggregate_m4_1_publication(
                &output,
                [
                    partials[0].as_deref().unwrap_or(Path::new("")),
                    partials[1].as_deref().unwrap_or(Path::new("")),
                    partials[2].as_deref().unwrap_or(Path::new("")),
                ],
            )?;
            println!("M4.1 publication status: {}", report.comparison_status);
        }
        Some("m4") => {
            let mut output = PathBuf::from("benchmarks/results");
            let mut samples = 1;
            let mut warmup = 0;
            let mut jambda_repo = None;
            let mut execute_only = false;
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--output" => {
                        output =
                            PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                        index += 2;
                    }
                    "--samples" => {
                        samples = args
                            .get(index + 1)
                            .ok_or("missing --samples value")?
                            .parse()?;
                        index += 2;
                    }
                    "--warmup" => {
                        warmup = args
                            .get(index + 1)
                            .ok_or("missing --warmup value")?
                            .parse()?;
                        index += 2;
                    }
                    "--jambda-repo" => {
                        jambda_repo = Some(PathBuf::from(
                            args.get(index + 1).ok_or("missing --jambda-repo value")?,
                        ));
                        index += 2;
                    }
                    "--execute-only" => {
                        execute_only = true;
                        index += 1;
                    }
                    other => return Err(format!("unknown bench option: {other}").into()),
                }
            }
            if samples != 1 || warmup != 0 {
                return Err(
                    "M4 functional proving currently requires --samples 1 --warmup 0".into(),
                );
            }
            if !execute_only {
                return Err("local M4 requires --execute-only; full proving runs through bench m4-proof in GitHub Actions".into());
            }
            let report =
                run_m4_preflight(&output, &jambda_repo.ok_or("M4 requires --jambda-repo")?)?;
            println!("M4 preflight complete: {}", report.complete);
            if !report.complete {
                return Err("M4 execute-only preflight failed".into());
            }
        }
        _ => {
            return Err("expected bench m2, report, validate-m3, validate-m4, validate-m4-preflight, validate-m4-proof, validate-m4-publication, m4, m4-proof, aggregate-m4, m4-publication-workload, aggregate-m4-publication, or m4-publication".into())
        }
    }
    Ok(())
}

fn bench_worker_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut benchmark = None;
    let mut case = None;
    let mut samples = 10;
    let mut warmup = 1;
    let mut output = None;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "m2" => index += 1,
            "--benchmark" => {
                benchmark = args.get(index + 1).cloned();
                index += 2;
            }
            "--case" => {
                case = args.get(index + 1).cloned();
                index += 2;
            }
            "--samples" => {
                samples = args
                    .get(index + 1)
                    .ok_or("missing --samples value")?
                    .parse()?;
                index += 2;
            }
            "--warmup" => {
                warmup = args
                    .get(index + 1)
                    .ok_or("missing --warmup value")?
                    .parse()?;
                index += 2;
            }
            "--output" => {
                output = args.get(index + 1).map(PathBuf::from);
                index += 2;
            }
            other => return Err(format!("unknown worker option: {other}").into()),
        }
    }
    run_worker(
        benchmark.as_deref().ok_or("missing worker benchmark")?,
        case.as_deref().ok_or("missing worker case")?,
        samples,
        warmup,
        output.as_deref().ok_or("missing worker output")?,
    )?;
    Ok(())
}

fn inspect(path: &Path, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read(path)?;
    let case = RefineCaseV1::decode_canonical(&bytes)?;
    case.validate()?;
    if json {
        println!("{}", case.debug_json()?);
        return Ok(());
    }

    println!("format_version: {}", case.format_version);
    println!("profile: {:?}", case.profile);
    println!("core_index: {}", case.core_index);
    println!("item_index: {}", case.item_index);
    println!("program_hash: {}", hex(&case.program.code_hash));
    println!("blocks: {}", case.program.blocks.len());
    println!("instructions: {}", case.program.instruction_count());
    println!(
        "host_calls: {}",
        case.program
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter_map(|i| i.host_call_id())
            .count()
    );
    println!("o_bytes: {}", case.program.o_blob.len());
    println!("w_bytes: {}", case.program.w_blob.len());
    println!("z_pages: {}", case.program.z_pages);
    println!("s_bytes: {}", case.program.s_bytes);
    println!("external_groups: {}", case.external_data.len());
    println!("import_groups: {}", case.import_segments.len());
    println!(
        "historical_lookups: {}",
        case.state_witness.historical_lookups.len()
    );
    println!("state_witness_binding: {:?}", case.state_witness.binding);
    Ok(())
}

fn make_minimal(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let case = RefineCaseV1 {
        format_version: REFINE_CASE_FORMAT_V1,
        profile: SmokeProfile::default(),
        core_index: 0,
        item_index: 0,
        work_package: Vec::new(),
        authorization_trace: Vec::new(),
        external_data: vec![Vec::new()],
        import_segments: vec![Vec::new()],
        export_offset: 0,
        program: PvmProgramV1 {
            format_version: PVM_PROGRAM_FORMAT_V1,
            code_hash: [0; 32],
            o_blob: Vec::new(),
            w_blob: Vec::new(),
            z_pages: 1,
            s_bytes: 0,
            blocks: vec![PvmBlockV1 {
                entry_pc: 0,
                instructions: vec![PvmInstructionV1 {
                    pc: 0,
                    opcode: 1,
                    registers: RegisterOperandsV1::default(),
                    immediate: Vec::new(),
                    pc_delta: 0,
                }],
                terminator: PvmTerminatorV1::Halt,
            }],
            jump_table: Vec::new(),
            c_blob: Vec::new(),
        },
        state_witness: RefineStateWitnessV1 {
            binding: StateWitnessBindingV1::Fixture,
            historical_lookups: Vec::new(),
        },
    };
    case.validate()?;
    fs::write(path, case.encode_canonical())?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() -> ExitCode {
    let argv = env::args().skip(1).collect::<Vec<_>>();
    let mut args = argv.iter().cloned();
    match (args.next().as_deref(), args.next()) {
        (Some("inspect"), Some(path)) => {
            let json = args.next().as_deref() == Some("--json");
            match inspect(Path::new(&path), json) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("zk-jam: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        (Some("make-minimal"), Some(path)) if args.next().is_none() => {
            match make_minimal(Path::new(&path)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("zk-jam: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        (Some("openvm"), _) => match openvm_command(&argv) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("zk-jam: {error}");
                ExitCode::FAILURE
            }
        },
        (Some("bench"), _) => match bench_command(&argv) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("zk-jam: {error}");
                ExitCode::FAILURE
            }
        },
        (Some("__bench-worker"), _) => match bench_worker_command(&argv) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("zk-jam worker: {error}");
                ExitCode::FAILURE
            }
        },
        (Some("__m3-worker"), _) => match m3_worker_command(&argv) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("zk-jam M3 worker: {error}");
                ExitCode::FAILURE
            }
        },
        (Some("__m4-publication-worker"), _) => match m4_publication_worker_command(&argv) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("zk-jam M4 publication worker: {error}");
                ExitCode::FAILURE
            }
        },
        (Some("__pvm-openvm-worker"), _) => match pvm_openvm_worker_command(&argv) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("zk-jam PVM -> OpenVM worker: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            usage();
            ExitCode::FAILURE
        }
    }
}

fn pvm_openvm_worker_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut implementation = None;
    let mut workload = None;
    let mut a = None;
    let mut b = None;
    let mut output = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--implementation" => {
                implementation = Some(
                    args.get(index + 1)
                        .ok_or("missing --implementation value")?
                        .clone(),
                );
                index += 2;
            }
            "--workload" => {
                workload = Some(
                    args.get(index + 1)
                        .ok_or("missing --workload value")?
                        .clone(),
                );
                index += 2;
            }
            "--a" => {
                a = Some(
                    args.get(index + 1)
                        .ok_or("missing --a value")?
                        .parse::<u32>()?,
                );
                index += 2;
            }
            "--b" => {
                b = Some(
                    args.get(index + 1)
                        .ok_or("missing --b value")?
                        .parse::<u32>()?,
                );
                index += 2;
            }
            "--output" => {
                output = Some(PathBuf::from(
                    args.get(index + 1).ok_or("missing --output value")?,
                ));
                index += 2;
            }
            other => return Err(format!("unknown PVM -> OpenVM worker option: {other}").into()),
        }
    }
    run_pvm_openvm_worker(
        &implementation.ok_or("worker requires --implementation")?,
        &workload.ok_or("worker requires --workload")?,
        [
            a.ok_or("worker requires --a")?,
            b.ok_or("worker requires --b")?,
        ],
        &output.ok_or("worker requires --output")?,
    )?;
    Ok(())
}

fn m3_worker_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut benchmark = None;
    let mut a = None;
    let mut b = None;
    let mut samples = 1;
    let mut warmup = 0;
    let mut output = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "__m3-worker" => index += 1,
            "--benchmark" => {
                benchmark = args.get(index + 1).cloned();
                index += 2;
            }
            "--a" => {
                a = Some(args.get(index + 1).ok_or("missing --a value")?.parse()?);
                index += 2;
            }
            "--b" => {
                b = Some(args.get(index + 1).ok_or("missing --b value")?.parse()?);
                index += 2;
            }
            "--samples" => {
                samples = args
                    .get(index + 1)
                    .ok_or("missing --samples value")?
                    .parse()?;
                index += 2;
            }
            "--warmup" => {
                warmup = args
                    .get(index + 1)
                    .ok_or("missing --warmup value")?
                    .parse()?;
                index += 2;
            }
            "--output" => {
                output = args.get(index + 1).map(PathBuf::from);
                index += 2;
            }
            other => return Err(format!("unknown M3 worker option: {other}").into()),
        }
    }
    run_m3_worker(
        benchmark.as_deref().ok_or("missing M3 worker benchmark")?,
        a.ok_or("missing M3 worker a")?,
        b.ok_or("missing M3 worker b")?,
        samples,
        warmup,
        output.as_deref().ok_or("missing M3 worker output")?,
    )?;
    Ok(())
}

fn m4_publication_worker_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut implementation = None;
    let mut workload = None;
    let mut a = None;
    let mut b = None;
    let mut output = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "__m4-publication-worker" => index += 1,
            "--implementation" => {
                implementation = Some(args.get(index + 1).ok_or("missing implementation")?.clone());
                index += 2;
            }
            "--workload" => {
                workload = Some(
                    match args.get(index + 1).ok_or("missing workload")?.as_str() {
                        "arithmetic" => M4ProgramId::Arithmetic,
                        "branch" => M4ProgramId::Branch,
                        "memory" => M4ProgramId::Memory16K,
                        other => return Err(format!("unknown M4 workload: {other}").into()),
                    },
                );
                index += 2;
            }
            "--a" => {
                a = Some(args.get(index + 1).ok_or("missing --a value")?.parse()?);
                index += 2;
            }
            "--b" => {
                b = Some(args.get(index + 1).ok_or("missing --b value")?.parse()?);
                index += 2;
            }
            "--output" => {
                output = Some(PathBuf::from(
                    args.get(index + 1).ok_or("missing --output value")?,
                ));
                index += 2;
            }
            other => return Err(format!("unknown M4 publication worker option: {other}").into()),
        }
    }
    run_m4_publication_worker(
        implementation.as_deref().ok_or("missing implementation")?,
        workload.ok_or("missing workload")?,
        [a.ok_or("missing --a value")?, b.ok_or("missing --b value")?],
        output.as_deref().ok_or("missing --output value")?,
    )?;
    Ok(())
}
