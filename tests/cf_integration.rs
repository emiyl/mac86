use std::env;
use std::process::Command;

#[test]
fn run_cf_test_binary() {
    // Path to the compiled emulator binary is provided by Cargo when built for tests
    let bin = env::var("CARGO_BIN_EXE_mac86").expect("CARGO_BIN_EXE_mac86 not set");

    // The test program to run (already included in the repository)
    let test_prog = "root/compile/cf_test";

    let output = Command::new(&bin)
        .arg(test_prog)
        .output()
        .expect("failed to spawn emulator process");

    println!("stdout:\n{}", String::from_utf8_lossy(&output.stdout));
    eprintln!("stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "emulator exited with non-zero status"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Fail the test if any FAIL markers appear in the program output.
    assert!(
        !stdout.contains("FAIL") && !stderr.contains("FAIL"),
        "cf_test printed FAIL in output"
    );
}
