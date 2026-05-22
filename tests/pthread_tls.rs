use std::path::PathBuf;
use std::process::Command;

fn bin(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(name)
}

fn emulator_bin() -> String {
    std::env::var("CARGO_BIN_EXE_mac86").unwrap_or_else(|_| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/debug/mac86")
            .to_string_lossy()
            .into_owned()
    })
}

fn run_program(program: &str, args: &[&str]) -> std::process::Output {
    Command::new(emulator_bin())
        .arg(bin(program))
        .args(args)
        .output()
        .expect("failed to spawn emulator process")
}

fn assert_clean_output(output: &std::process::Output, label: &str) {
    println!(
        "stdout for {}:\n{}",
        label,
        String::from_utf8_lossy(&output.stdout)
    );
    eprintln!(
        "stderr for {}:\n{}",
        label,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "{} exited with non-zero status",
        label
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("FAIL")
            && !String::from_utf8_lossy(&output.stderr).contains("FAIL"),
        "{} printed FAIL",
        label
    );
}

#[test]
fn run_pthread_tls() {
    let output = run_program("pthread_tls/pthread_tls_i386", &[]);
    assert_clean_output(&output, "pthread_tls");
}
