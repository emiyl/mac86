use anyhow::Result;
use clap::Parser;
use log::info;
use mac86::{binary_loader, emulator, process};
use std::path::PathBuf;

/// i386 macOS emulator for arm64 Macs
#[derive(Parser, Debug)]
#[command(name = "mac86")]
#[command(about = "An i386 macOS application emulator for arm64 Macs", long_about = None)]
struct Args {
    /// Path to the i386 macOS executable to run
    #[arg(value_name = "BINARY")]
    binary: PathBuf,

    /// Arguments to pass to the binary
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Path to the emulation environment (optional)
    #[arg(short, long)]
    env_path: Option<PathBuf>,

    /// Print each syscall number and arguments as they execute
    #[arg(long)]
    trace_syscalls: bool,

    /// Print each instruction address as it executes
    #[arg(long)]
    trace_instr: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    if args.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Info)
            .init();
    }

    info!("mac86 i386 emulator starting");
    info!("Target binary: {}", args.binary.display());

    // Create the emulation environment
    let trace = emulator::TraceConfig {
        syscalls: args.trace_syscalls,
        instructions: args.trace_instr,
    };
    let mut emulation_context = emulator::EmulationContext::new(args.env_path, trace)?;

    // Load the binary
    let binary_info = binary_loader::load_binary(&args.binary)?;
    info!(
        "Binary loaded: {} (entry: 0x{:x})",
        binary_info.name, binary_info.entry_point
    );

    // Create and run the process
    let mut process = process::Process::new(binary_info, &mut emulation_context)?;
    process.execute(&args.args)?;

    info!("Execution completed");
    Ok(())
}
