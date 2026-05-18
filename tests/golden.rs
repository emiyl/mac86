/// Golden / smoke tests for sample binaries.
///
/// Each test loads a pre-built i386 binary through the full emulation stack
/// (binary_loader → Process → CpuEmulator) and asserts that execution
/// completes without returning an error.
///
/// Full stdout comparison requires VFS output-buffer support (see the
/// `VirtualFileSystem::capture_output` TODO). For now we verify the happy
/// path: no panic, no `Err` result.
///
/// Missing binaries are skipped rather than failed so that CI without the
/// i386 SDK still passes.
use mac86::binary_loader;
use mac86::emulator::{EmulationContext, TraceConfig};
use mac86::process::Process;
use std::path::PathBuf;

fn samples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples")
}

fn run_binary(name: &str) -> Option<Result<(), String>> {
    let path = samples_dir().join(name);
    if !path.exists() {
        eprintln!("[skip] {} not found", name);
        return None;
    }

    let bi = match binary_loader::load_binary(&path) {
        Ok(b) => b,
        Err(e) => return Some(Err(format!("load_binary: {}", e))),
    };

    let mut ctx = EmulationContext::new(None, TraceConfig::default()).unwrap();
    let mut process = match Process::new(bi, &mut ctx) {
        Ok(p) => p,
        Err(e) => return Some(Err(format!("Process::new: {}", e))),
    };

    let argv = vec![name.to_string()];
    Some(process.execute(&argv).map_err(|e| format!("execute: {}", e)))
}

#[test]
fn phase1_hello_static_runs_without_error() {
    let Some(result) = run_binary("phase1_hello_static") else {
        return;
    };
    result.unwrap_or_else(|e| panic!("phase1_hello_static: {}", e));
}

#[test]
fn phase1_hello_runs_without_error() {
    let Some(result) = run_binary("phase1_hello") else {
        return;
    };
    result.unwrap_or_else(|e| panic!("phase1_hello: {}", e));
}
