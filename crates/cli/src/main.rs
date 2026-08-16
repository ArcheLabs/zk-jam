use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use zk_jam_benchmark::run_zkrefine;
use zk_jam_refine_interface::{
    CanonicalCodec, PvmBlockV1, PvmInstructionV1, PvmProgramV1, PvmTerminatorV1, RefineCaseV1,
    RefineStateWitnessV1, RegisterOperandsV1, StateWitnessBindingV1, ZkRefineProfile,
    PVM_PROGRAM_FORMAT_V1, REFINE_CASE_FORMAT_V1,
};

fn usage() {
    eprintln!("usage: zk-jam inspect <case.bin> [--json]");
    eprintln!("       zk-jam make-minimal <case.bin>");
    eprintln!("       zk-jam zkrefine --fixture fixtures/refine-import-export-v1 [--output artifacts/zkrefine]");
}

fn inspect(path: &Path, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let case = RefineCaseV1::decode_canonical(&fs::read(path)?)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&case)?);
        return Ok(());
    }
    println!("format_version: {}", case.format_version);
    println!("profile: {:?}", case.profile);
    println!("core_index: {}", case.core_index);
    println!("item_index: {}", case.item_index);
    println!("program_instructions: {}", case.program.instruction_count());
    println!("external_groups: {}", case.external_data.len());
    println!("import_groups: {}", case.import_segments.len());
    println!(
        "historical_lookups: {}",
        case.state_witness.historical_lookups.len()
    );
    Ok(())
}

fn make_minimal(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let case = RefineCaseV1 {
        format_version: REFINE_CASE_FORMAT_V1,
        profile: ZkRefineProfile::default(),
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
    fs::write(path, case.encode_canonical())?;
    Ok(())
}

fn zkrefine_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = PathBuf::from("fixtures/refine-import-export-v1");
    let mut output = PathBuf::from("artifacts/zkrefine");
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--fixture" => {
                fixture = PathBuf::from(args.get(index + 1).ok_or("missing --fixture value")?);
                index += 2;
            }
            "--output" => {
                output = PathBuf::from(args.get(index + 1).ok_or("missing --output value")?);
                index += 2;
            }
            other => return Err(format!("unknown ZkRefine option: {other}").into()),
        }
    }
    let report = run_zkrefine(&fixture, &output)?;
    println!("ZkRefine verified: {}", report.verified);
    if !report.verified {
        return Err("ZkRefine acceptance failed".into());
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = env::args().collect::<Vec<_>>();
    let result = match args.get(1).map(String::as_str) {
        Some("inspect") => inspect(
            Path::new(args.get(2).map(String::as_str).unwrap_or("")),
            args.iter().any(|arg| arg == "--json"),
        ),
        Some("make-minimal") => {
            make_minimal(Path::new(args.get(2).map(String::as_str).unwrap_or("")))
        }
        Some("zkrefine") => zkrefine_command(&args),
        _ => {
            usage();
            Err("unknown command".into())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("zk-jam: {error}");
            ExitCode::FAILURE
        }
    }
}
