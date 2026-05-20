use rust_unix_tools::tools::grep;
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
        let root = std::env::temp_dir().join(format!("rust-unix-tools-grep-{name}-{nanos}"));
        fs::create_dir(&root).unwrap();
        Self { root }
    }

    fn file(&self, name: &str, contents: &[u8]) {
        fs::write(self.root.join(name), contents).unwrap();
    }

    fn dir(&self, name: &str) {
        fs::create_dir(self.root.join(name)).unwrap();
    }

    fn run(&self, args: &[&str]) -> (i32, Vec<u8>, Vec<u8>) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = grep::run(
            args.iter().map(OsString::from),
            &self.root,
            &mut stdout,
            &mut stderr,
        );
        (code, stdout, stderr)
    }

    fn run_sys(&self, args: &[&str]) -> (i32, Vec<u8>, Vec<u8>) {
        let out = Command::new("grep")
            .current_dir(&self.root)
            .args(args)
            .output()
            .expect("Failed to run grep");
        (
            out.status.code().unwrap_or(-1),
            out.stdout,
            out.stderr,
        )
    }

    fn assert_matches(&self, args: &[&str]) {
        let (rust_code, rust_out, _rust_err) = self.run(args);
        let (sys_code, sys_out, _sys_err) = self.run_sys(args);

        // Normalize stdout newline difference or file name prefix difference if any
        let rust_str = String::from_utf8_lossy(&rust_out);
        let sys_str = String::from_utf8_lossy(&sys_out);

        assert_eq!(rust_code, sys_code, "Exit codes do not match for args {:?}", args);
        assert_eq!(rust_str, sys_str, "Stdout does not match for args {:?}", args);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_grep_simple() {
    let f = Fixture::new("simple");
    f.file("a.txt", b"hello world\nrust language\nhello rust\n");

    f.assert_matches(&["hello", "a.txt"]);
    f.assert_matches(&["rust", "a.txt"]);
    f.assert_matches(&["missing_pattern", "a.txt"]);
}

#[test]
fn test_grep_options() {
    let f = Fixture::new("options");
    f.file("a.txt", b"apple\nBanana\ncherry\nBanana Split\nbanana\n");

    f.assert_matches(&["-i", "banana", "a.txt"]);
    f.assert_matches(&["-v", "Banana", "a.txt"]);
    f.assert_matches(&["-w", "Banana", "a.txt"]);
    f.assert_matches(&["-x", "Banana", "a.txt"]);
    f.assert_matches(&["-c", "Banana", "a.txt"]);
    f.assert_matches(&["-n", "banana", "a.txt"]);
    f.assert_matches(&["-b", "banana", "a.txt"]);
    f.assert_matches(&["-o", "banana", "a.txt"]);
    f.assert_matches(&["-m", "1", "banana", "a.txt"]);
    f.assert_matches(&["-F", "Banana Split", "a.txt"]);
}

#[test]
fn test_grep_multiple_patterns() {
    let f = Fixture::new("multi-patterns");
    f.file("a.txt", b"apple\nbanana\ncherry\n");

    f.assert_matches(&["-e", "apple", "-e", "cherry", "a.txt"]);
    
    // Newline-separated pattern string
    f.assert_matches(&["apple\ncherry", "a.txt"]);
}

#[test]
fn test_grep_recursive() {
    let f = Fixture::new("recursive");
    f.dir("subdir");
    f.file("subdir/a.txt", b"hello rust\n");
    f.file("subdir/b.txt", b"hello world\n");
    f.file("c.txt", b"hello rust\n");

    // Recursive search should order stdout deterministically because we sort entry listings!
    // But since system grep might not match our exact order, we compare the sorted lines.
    let (rust_code, rust_out, _) = f.run(&["-r", "hello", "."]);
    let (sys_code, sys_out, _) = f.run_sys(&["-r", "hello", "."]);

    assert_eq!(rust_code, sys_code);
    
    let rust_str = String::from_utf8_lossy(&rust_out);
    let sys_str = String::from_utf8_lossy(&sys_out);
    let mut rust_lines: Vec<_> = rust_str.lines().collect();
    let mut sys_lines: Vec<_> = sys_str.lines().collect();
    rust_lines.sort();
    sys_lines.sort();
    assert_eq!(rust_lines, sys_lines);
}
