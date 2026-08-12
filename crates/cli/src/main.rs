use std::{env, fs, path::Path, process::ExitCode};
use zk_jam_refine_interface::{
    CanonicalCodec, PvmBlockV1, PvmInstructionV1, PvmProgramV1, PvmTerminatorV1, RefineCaseV1,
    RefineStateWitnessV1, RegisterOperandsV1, SmokeProfile, StateWitnessBindingV1,
    PVM_PROGRAM_FORMAT_V1, REFINE_CASE_FORMAT_V1,
};

fn usage() {
    eprintln!("usage: zk-jam inspect <case.bin> [--json]");
    eprintln!("       zk-jam make-minimal <case.bin>");
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
    let mut args = env::args().skip(1);
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
        _ => {
            usage();
            ExitCode::FAILURE
        }
    }
}
