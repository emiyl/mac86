use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

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
fn run_core_io_fs() {
    let tmp = TempDir::new().expect("tempdir");
    let output = run_program(
        "core_io_fs/core_io_fs_i386",
        &[tmp.path().to_str().unwrap()],
    );
    assert_clean_output(&output, "core_io_fs");

    let rename_new = tmp.path().join("rename_new.txt");
    let copy_dst = tmp.path().join("copy_dst.txt");
    let fcopy_dst = tmp.path().join("fcopy_dst.txt");
    let chmod_path = tmp.path().join("chmod_me.txt");
    let fchmod_path = tmp.path().join("fchmod_me.txt");
    let rename_old = tmp.path().join("rename_old.txt");
    let unlink_path = tmp.path().join("unlink_me.txt");
    let rmdir_path = tmp.path().join("rmdir_dir");
    let mkdir_path = tmp.path().join("mkdir_dir");

    assert!(rename_new.exists());
    assert!(copy_dst.exists());
    assert!(fcopy_dst.exists());
    assert!(chmod_path.exists());
    assert!(fchmod_path.exists());
    assert!(!rename_old.exists());
    assert!(!unlink_path.exists());
    assert!(!rmdir_path.exists());
    assert!(mkdir_path.exists());

    assert_eq!(std::fs::read_to_string(copy_dst).unwrap(), "copyfile-data");
    assert_eq!(
        std::fs::read_to_string(fcopy_dst).unwrap(),
        "fcopyfile-data"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(chmod_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(fchmod_path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}
