use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn bin(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("root/bin")
        .join(name)
}

fn run_and_capture_cli(bin_name: &str, args: &[&str]) -> String {
    // Resolve emulator binary. Prefer CARGO_BIN_EXE_mac86 if provided by Cargo.
    let emulator_bin = std::env::var("CARGO_BIN_EXE_mac86").unwrap_or_else(|_| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/debug/mac86")
            .to_string_lossy()
            .into_owned()
    });

    let mut cmd_args = vec![bin(bin_name).to_string_lossy().into_owned()];
    cmd_args.extend(args.iter().map(|s| s.to_string()));

    let out = Command::new(emulator_bin)
        .args(&cmd_args)
        .output()
        .expect("run emulator binary");

    assert!(out.status.success(), "emulator exit code non-zero");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn test_echo_and_cat() {
    let out = run_and_capture_cli("echo", &["hello"]);
    assert_eq!(out, "hello\n");

    // cat: create temp file and cat it
    let td = TempDir::new().expect("tempdir");
    let file = td.path().join("f.txt");
    fs::write(&file, "line1\n").expect("write");
    let out2 = run_and_capture_cli("cat", &[file.to_str().unwrap()]);
    assert_eq!(out2, "line1\n");
}

#[test]
fn test_ls_mkdir_rmdir_rm_mv_chmod() {
    let td = TempDir::new().expect("tempdir");
    let dir = td.path().join("d1");

    // mkdir
    let _ = run_and_capture_cli("mkdir", &[dir.to_str().unwrap()]);
    assert!(dir.exists() && dir.is_dir());

    // ls should list the new directory when listing parent
    let out_ls = run_and_capture_cli("ls", &[td.path().to_str().unwrap()]);
    assert!(out_ls.contains("d1"));

    // create a file to test mv and rm
    let f1 = dir.join("a.txt");
    fs::write(&f1, "payload").expect("write file");

    // mv a.txt -> b.txt
    let b = dir.join("b.txt");
    let _ = run_and_capture_cli("mv", &[f1.to_str().unwrap(), b.to_str().unwrap()]);
    assert!(!f1.exists());
    assert!(b.exists());

    // chmod: make it read-only (0644 -> 0444)
    let _ = run_and_capture_cli("chmod", &["444", b.to_str().unwrap()]);
    let meta = fs::metadata(&b).expect("meta");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(meta.permissions().mode() & 0o777, 0o444);
    }

    // rm the file
    let _ = run_and_capture_cli("rm", &[b.to_str().unwrap()]);
    assert!(!b.exists());

    // rmdir the directory
    let _ = run_and_capture_cli("rmdir", &[dir.to_str().unwrap()]);
    assert!(!dir.exists());
}
