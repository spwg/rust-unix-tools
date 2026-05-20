use rust_unix_tools::tools::cat;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rust-unix-tools-cat-{name}-{nanos}"));
        fs::create_dir(&root).unwrap();
        Self { root }
    }

    fn file(&self, name: &str, contents: &[u8]) {
        fs::write(self.root.join(name), contents).unwrap();
    }

    fn run(&self, args: &[&str]) -> (i32, Vec<u8>, Vec<u8>) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = cat::run(
            args.iter().map(OsString::from),
            &self.root,
            &mut stdout,
            &mut stderr,
        );
        (code, stdout, stderr)
    }

    fn run_sys(&self, args: &[&str]) -> (i32, Vec<u8>, Vec<u8>) {
        let out = Command::new("gcat")
            .current_dir(&self.root)
            .args(args)
            .output()
            .expect("Failed to run gcat");
        (
            out.status.code().unwrap_or(-1),
            out.stdout,
            out.stderr,
        )
    }

    fn assert_matches(&self, args: &[&str]) {
        let (rust_code, rust_out, _rust_err) = self.run(args);
        let (sys_code, sys_out, _sys_err) = self.run_sys(args);

        assert_eq!(rust_code, sys_code, "Exit codes do not match for args {:?}", args);
        assert_eq!(rust_out, sys_out, "Stdout does not match for args {:?}", args);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_cat_simple() {
    let f = Fixture::new("simple");
    f.file("a.txt", b"hello world\n");
    f.file("b.txt", b"second file\n");

    f.assert_matches(&["a.txt"]);
    f.assert_matches(&["a.txt", "b.txt"]);
}

#[test]
fn test_cat_options() {
    let f = Fixture::new("options");
    f.file("a.txt", b"line1\n\n\nline4\n\twith tab\nnon-printing \x00\x01\x1b\x7f\x80\xff\n");

    let options_to_test = vec![
        vec!["-n"],
        vec!["-b"],
        vec!["-s"],
        vec!["-v"],
        vec!["-T"],
        vec!["-E"],
        vec!["-A"],
        vec!["-e"],
        vec!["-t"],
        vec!["-ns"],
        vec!["-bs"],
        vec!["-vET"],
        vec!["--number"],
        vec!["--number-nonblank"],
        vec!["--squeeze-blank"],
        vec!["--show-ends"],
        vec!["--show-tabs"],
        vec!["--show-nonprinting"],
        vec!["--show-all"],
    ];

    for opts in options_to_test {
        let mut args = opts.clone();
        args.push("a.txt");
        f.assert_matches(&args);
    }
}

#[test]
fn test_cat_missing_file() {
    let f = Fixture::new("missing");
    let (rust_code, _rust_out, rust_err) = f.run(&["does_not_exist.txt"]);
    let (sys_code, _sys_out, _sys_err) = f.run_sys(&["does_not_exist.txt"]);

    assert_eq!(rust_code, sys_code);
    assert!(!rust_err.is_empty());
}

#[test]
fn test_cat_multi_file_squeeze_and_number() {
    let f = Fixture::new("multi-squeeze");
    f.file("f1.txt", b"a\n\n");
    f.file("f2.txt", b"\nb\n");

    f.assert_matches(&["-s", "f1.txt", "f2.txt"]);
    f.assert_matches(&["-n", "f1.txt", "f2.txt"]);
    f.assert_matches(&["-b", "f1.txt", "f2.txt"]);
    f.assert_matches(&["-sb", "f1.txt", "f2.txt"]);
}
