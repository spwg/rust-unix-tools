use std::process::Command;

#[test]
fn test_bin_cat() {
    let bin_path = env!("CARGO_BIN_EXE_cat");
    let output = Command::new(bin_path).arg("--help").output().unwrap();
    assert!(output.status.success());
    assert!(!output.stdout.is_empty());
}

#[test]
fn test_bin_find() {
    let bin_path = env!("CARGO_BIN_EXE_find");
    let output = Command::new(bin_path).arg(".").output().unwrap();
    assert!(output.status.success());
    assert!(!output.stdout.is_empty());
}

#[test]
fn test_bin_grep() {
    let bin_path = env!("CARGO_BIN_EXE_grep");
    let output = Command::new(bin_path).arg("--help").output().unwrap();
    assert!(output.status.success());
    assert!(!output.stdout.is_empty());
}

#[test]
fn test_bin_ls() {
    let bin_path = env!("CARGO_BIN_EXE_ls");
    let output = Command::new(bin_path).arg("--help").output().unwrap();
    assert!(output.status.success());
    assert!(!output.stdout.is_empty());
}
