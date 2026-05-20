use rust_unix_tools::tools::find;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
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
        let root = std::env::temp_dir().join(format!("rust-unix-tools-find-{name}-{nanos}"));
        fs::create_dir(&root).unwrap();
        Self { root }
    }

    fn file(&self, name: &str, contents: &[u8]) {
        fs::write(self.root.join(name), contents).unwrap();
    }

    fn dir(&self, name: &str) {
        fs::create_dir(self.root.join(name)).unwrap();
    }

    fn symlink(&self, target: &str, link: &str) {
        std::os::unix::fs::symlink(target, self.root.join(link)).unwrap();
    }

    fn run(&self, args: &[&str]) -> (i32, Vec<u8>, Vec<u8>) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = find::run(
            args.iter().map(OsString::from),
            &self.root,
            &mut stdout,
            &mut stderr,
        );
        (code, stdout, stderr)
    }

    fn run_sys(&self, args: &[&str]) -> (i32, Vec<u8>, Vec<u8>) {
        let out = Command::new("find")
            .current_dir(&self.root)
            .args(args)
            .output()
            .expect("Failed to run find");
        (
            out.status.code().unwrap_or(-1),
            out.stdout,
            out.stderr,
        )
    }

    fn assert_matches(&self, args: &[&str]) {
        let (rust_code, rust_out, _) = self.run(args);
        let (sys_code, sys_out, _) = self.run_sys(args);

        assert_eq!(rust_code, sys_code, "Exit codes do not match for args {:?}", args);

        let rust_str = String::from_utf8_lossy(&rust_out);
        let sys_str = String::from_utf8_lossy(&sys_out);

        let mut rust_lines: Vec<_> = rust_str.lines().collect();
        let mut sys_lines: Vec<_> = sys_str.lines().collect();
        rust_lines.sort();
        sys_lines.sort();

        assert_eq!(rust_lines, sys_lines, "Outputs do not match for args {:?}", args);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_find_simple() {
    let f = Fixture::new("simple");
    f.file("a.txt", b"hello");
    f.dir("subdir");
    f.file("subdir/b.txt", b"world");

    f.assert_matches(&[".", "-name", "*.txt"]);
    f.assert_matches(&[".", "-type", "f"]);
    f.assert_matches(&[".", "-type", "d"]);
}

#[test]
fn test_find_operators() {
    let f = Fixture::new("operators");
    f.file("a.txt", b"hello");
    f.file("b.log", b"world");
    f.file("c.txt", b"test");

    f.assert_matches(&[".", "-name", "*.txt", "-o", "-name", "*.log"]);
    f.assert_matches(&[".", "!", "-name", "*.txt"]);
    f.assert_matches(&[".", "(", "-name", "*.txt", "-o", "-name", "*.log", ")"]);
}

#[test]
fn test_find_depth_limits() {
    let f = Fixture::new("depth");
    f.dir("d1");
    f.dir("d1/d2");
    f.dir("d1/d2/d3");
    f.file("d1/d2/d3/f.txt", b"hello");

    f.assert_matches(&[".", "-maxdepth", "2"]);
    f.assert_matches(&[".", "-mindepth", "2"]);
    f.assert_matches(&[".", "-mindepth", "2", "-maxdepth", "3"]);
}

#[test]
fn test_find_size() {
    let f = Fixture::new("size");
    f.file("small.txt", b"a"); // 1 byte
    f.file("medium.txt", &[0; 1000]); // 1000 bytes
    f.file("large.txt", &[0; 2000]); // 2000 bytes

    f.assert_matches(&[".", "-size", "+512c"]);
    f.assert_matches(&[".", "-size", "-2k"]);
    f.assert_matches(&[".", "-size", "+1k"]);
}

#[test]
fn test_find_depth_first() {
    let f = Fixture::new("depth_first");
    f.dir("d1");
    f.file("d1/f1.txt", b"");

    let (_, rust_out, _) = f.run(&[".", "-depth"]);
    let (_, sys_out, _) = f.run_sys(&[".", "-depth"]);

    let rust_str = String::from_utf8_lossy(&rust_out);
    let sys_str = String::from_utf8_lossy(&sys_out);

    // With -depth, children MUST be printed before parent
    assert_eq!(rust_str, sys_str);
}

#[test]
fn test_find_perm() {
    let f = Fixture::new("perm");
    f.file("f1.txt", b"");
    f.file("f2.txt", b"");

    fs::set_permissions(f.root.join("f1.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(f.root.join("f2.txt"), fs::Permissions::from_mode(0o644)).unwrap();

    f.assert_matches(&[".", "-perm", "755"]);
    f.assert_matches(&[".", "-perm", "644"]);
    f.assert_matches(&[".", "-perm", "-600"]);
}

#[test]
fn test_find_exec() {
    let f = Fixture::new("exec");
    f.file("a.txt", b"hello");
    f.file("b.txt", b"world");

    f.assert_matches(&[".", "-name", "*.txt", "-exec", "echo", "found", "{}", ";"]);
}

#[test]
fn test_find_symlinks() {
    let f = Fixture::new("symlinks");
    f.dir("d1");
    f.file("d1/f1.txt", b"");
    f.symlink("d1", "link1");

    // Test default (-P), follow-all (-L), follow-command-line (-H)
    f.assert_matches(&["-P", "."]);
    f.assert_matches(&["-H", "link1"]);
    f.assert_matches(&["-L", "."]);
}

#[test]
fn test_find_loop() {
    let f = Fixture::new("loop");
    f.dir("d1");
    // Create a loop: d1/link -> d1
    f.symlink("../d1", "d1/link");

    // Running with -L should detect loop and print error
    let (code, _, rust_err) = f.run(&["-L", "."]);
    assert_eq!(code, 1);
    let err_str = String::from_utf8_lossy(&rust_err);
    assert!(err_str.contains("loop"), "Expected loop warning in stderr: {}", err_str);
}
