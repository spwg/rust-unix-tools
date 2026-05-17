use rust_unix_tools::tools::ls;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
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
        let root = std::env::temp_dir().join(format!("rust-unix-tools-{name}-{nanos}"));
        fs::create_dir(&root).unwrap();
        Self { root }
    }

    fn file(&self, name: &str, contents: &[u8]) {
        fs::write(self.root.join(name), contents).unwrap();
    }

    fn dir(&self, name: &str) {
        fs::create_dir(self.root.join(name)).unwrap();
    }

    fn executable(&self, name: &str) {
        self.file(name, b"#!/bin/sh\n");
        let path = self.root.join(name);
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }

    fn chmod(&self, name: &str, mode: u32) {
        let path = self.root.join(name);
        let mut perms = fs::symlink_metadata(&path).unwrap().permissions();
        perms.set_mode(mode);
        fs::set_permissions(path, perms).unwrap();
    }

    fn symlink(&self, target: &str, name: &str) {
        symlink(target, self.root.join(name)).unwrap();
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        run_ls(args, &self.root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run_ls(args: &[&str], cwd: &Path) -> (i32, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = ls::run(
        args.iter().map(OsString::from),
        cwd,
        &mut stdout,
        &mut stderr,
    );
    (
        code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[test]
fn downloaded_reference_is_gnu_coreutils_9_9() {
    let manpage = include_str!("fixtures/gnu-ls-9.9-manpage.html");
    assert!(manpage.contains("GNU coreutils 9.9"));
    assert!(manpage.contains("ls - list directory contents"));
    assert!(manpage.contains("Sort entries alphabetically"));
    assert!(manpage.contains("-a</b>, <b>--all"));
    assert!(manpage.contains("--sort</b>=<i>WORD"));
}

#[test]
fn default_lists_current_directory_alphabetically_without_dot_entries() {
    let f = Fixture::new("default");
    f.file("zeta", b"z");
    f.file(".hidden", b"h");
    f.file("alpha", b"a");

    let (code, stdout, stderr) = f.run(&[]);

    assert_eq!(code, 0);
    assert_eq!(stdout, "alpha\nzeta\n");
    assert_eq!(stderr, "");
}

#[test]
fn all_and_almost_all_follow_dot_entry_rules_from_manpage() {
    let f = Fixture::new("all");
    f.file("plain", b"");
    f.file(".dot", b"");

    assert_eq!(f.run(&["--all"]).1, ".\n..\n.dot\nplain\n");
    assert_eq!(f.run(&["--almost-all"]).1, ".dot\nplain\n");
    assert_eq!(f.run(&["-a"]).1, ".\n..\n.dot\nplain\n");
    assert_eq!(f.run(&["-A"]).1, ".dot\nplain\n");
}

#[test]
fn explicit_hidden_operands_are_listed_even_without_all() {
    let f = Fixture::new("explicit-hidden");
    f.file(".dot", b"");

    let (code, stdout, stderr) = f.run(&[".dot"]);

    assert_eq!(code, 0);
    assert_eq!(stdout, ".dot\n");
    assert_eq!(stderr, "");
}

#[test]
fn directory_option_lists_directory_itself_not_contents() {
    let f = Fixture::new("directory");
    f.dir("sub");
    fs::write(f.root.join("sub").join("child"), b"").unwrap();

    assert_eq!(f.run(&["--directory", "sub"]).1, "sub\n");
    assert_eq!(f.run(&["-d", "sub"]).1, "sub\n");
}

#[test]
fn non_directory_operands_are_printed_before_directory_operands() {
    let f = Fixture::new("operands");
    f.file("file", b"");
    f.dir("dir");
    fs::write(f.root.join("dir").join("inside"), b"").unwrap();

    let (code, stdout, stderr) = f.run(&["dir", "file"]);

    assert_eq!(code, 0);
    assert_eq!(stdout, "file\n\ndir:\ninside\n");
    assert_eq!(stderr, "");
}

#[test]
fn file_type_indicators_match_gnu_classify_and_slash_styles() {
    let f = Fixture::new("indicators");
    f.dir("dir");
    f.executable("run");
    f.file("plain", b"");
    f.symlink("plain", "link");

    assert_eq!(f.run(&["-F"]).1, "dir/\nlink@\nplain\nrun*\n");
    assert_eq!(f.run(&["--file-type"]).1, "dir/\nlink@\nplain\nrun\n");
    assert_eq!(f.run(&["-p"]).1, "dir/\nlink\nplain\nrun\n");
    assert_eq!(
        f.run(&["--indicator-style=none"]).1,
        "dir\nlink\nplain\nrun\n"
    );
    assert_eq!(
        f.run(&["--indicator-style", "classify"]).1,
        "dir/\nlink@\nplain\nrun*\n"
    );
    assert_eq!(f.run(&["--classify=never"]).1, "dir\nlink\nplain\nrun\n");
}

#[test]
fn backup_entries_can_be_ignored() {
    let f = Fixture::new("backups");
    f.file("keep", b"");
    f.file("drop~", b"");

    assert_eq!(f.run(&["--ignore-backups"]).1, "keep\n");
    assert_eq!(f.run(&["-B"]).1, "keep\n");
}

#[test]
fn format_options_override_each_other_and_support_commas() {
    let f = Fixture::new("format");
    f.file("a", b"");
    f.file("b", b"");

    assert_eq!(f.run(&["--format=commas"]).1, "a, b\n");
    assert_eq!(f.run(&["-m"]).1, "a, b\n");
    assert_eq!(f.run(&["--format=vertical"]).1, "a  b\n");
    assert_eq!(f.run(&["-C"]).1, "a  b\n");
    assert_eq!(f.run(&["-C1"]).1, "a\nb\n");
    assert!(f.run(&["--format=long"]).1.contains(" a\n"));
}

#[test]
fn sorting_options_cover_name_none_size_extension_reverse_and_directories_first() {
    let f = Fixture::new("sort");
    f.file("b.rs", &[b'x'; 10_000]);
    f.file("a.txt", b"1");
    f.dir("dir");

    assert_eq!(f.run(&[]).1, "a.txt\nb.rs\ndir\n");
    assert_eq!(f.run(&["--sort=size"]).1, "b.rs\ndir\na.txt\n");
    assert_eq!(f.run(&["-S"]).1, "b.rs\ndir\na.txt\n");
    assert_eq!(f.run(&["--sort=extension"]).1, "dir\nb.rs\na.txt\n");
    assert_eq!(f.run(&["-X"]).1, "dir\nb.rs\na.txt\n");
    assert_eq!(f.run(&["--sort=name", "-r"]).1, "dir\nb.rs\na.txt\n");
    assert_eq!(
        f.run(&["--group-directories-first", "-r"]).1,
        "dir\nb.rs\na.txt\n"
    );
    assert_eq!(f.run(&["-f"]).1.lines().count(), 5);
    assert_eq!(f.run(&["--sort=none"]).1.lines().count(), 3);
}

#[test]
fn long_inode_size_human_and_numeric_options_are_rendered() {
    let f = Fixture::new("long");
    f.file("plain", b"hello");

    let long = f.run(&["-lisnh", "plain"]).1;

    assert!(long.starts_with("total "));
    assert!(long.contains(" -rw"));
    assert!(long.contains(" plain\n"));
    assert!(long.split_whitespace().any(|part| part == "plain"));
    assert_eq!(f.run(&["-g", "plain"]).1.lines().count(), 2);
    assert_eq!(f.run(&["-o", "plain"]).1.lines().count(), 2);
    assert_eq!(f.run(&["--block-size=KB", "-s", "plain"]).0, 0);
    assert_eq!(f.run(&["--si", "-s", "plain"]).0, 0);
}

#[test]
fn time_related_gnu_options_parse_and_sort() {
    let f = Fixture::new("time");
    f.file("old", b"");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    f.file("new", b"");

    assert_eq!(f.run(&["-t"]).1, "new\nold\n");
    assert_eq!(f.run(&["--sort=time", "--time=mtime"]).1, "new\nold\n");
    assert_eq!(f.run(&["--time=ctime", "-l"]).0, 0);
    assert_eq!(f.run(&["--time=access", "-l"]).0, 0);
}

#[test]
fn recursive_lists_subdirectories_with_headers() {
    let f = Fixture::new("recursive");
    f.dir("a");
    fs::write(f.root.join("a").join("child"), b"").unwrap();

    let (code, stdout, stderr) = f.run(&["-R"]);

    assert_eq!(code, 0);
    assert!(stdout.contains(".:\na\n"));
    assert!(stdout.contains("a:\nchild\n"));
    assert_eq!(stderr, "");
}

#[test]
fn dereference_changes_symlink_directory_handling() {
    let f = Fixture::new("dereference");
    f.dir("real");
    fs::write(f.root.join("real").join("child"), b"").unwrap();
    f.symlink("real", "link");

    assert_eq!(f.run(&["link"]).1, "link\n");
    assert_eq!(f.run(&["-L", "link"]).1, "child\n");
}

#[test]
fn long_options_that_are_noops_or_value_consumers_are_accepted() {
    let f = Fixture::new("gnu-options");
    f.file("a", b"");

    let args = [
        "--color=never",
        "--hyperlink=never",
        "--literal",
        "--quote-name",
        "--quoting-style=literal",
        "--show-control-chars",
        "--tabsize=4",
        "--width=80",
        "--context",
        "--dereference-command-line",
        "--dereference-command-line-symlink-to-dir",
    ];
    assert_eq!(f.run(&args).1, "a\n");
    assert_eq!(f.run(&["--hide=*", "--ignore=*"]).1, "a\n");
    assert_eq!(f.run(&["--author", "--dired", "--full-time"]).0, 0);
}

#[test]
fn remaining_gnu_short_and_long_aliases_are_covered() {
    let f = Fixture::new("aliases");
    f.file("a", b"");
    f.file("b", &[0; 2048]);
    f.file("c", &[0; 20_000]);

    assert_eq!(f.run(&["--recursive"]).1, ".:\na\nb\nc\n");
    assert_eq!(f.run(&["--classify"]).1, "a\nb\nc\n");
    assert_eq!(f.run(&["--human-readable", "-s", "b"]).0, 0);
    assert_eq!(f.run(&["--zero"]).1, "a\nb\nc\n");
    assert_eq!(f.run(&["-c", "-l", "a"]).0, 0);
    assert_eq!(f.run(&["-G", "-l", "a"]).0, 0);
    assert_eq!(f.run(&["-H", "a"]).1, "a\n");
    assert_eq!(f.run(&["-NqQwZ", "a"]).1, "a\n");
    assert_eq!(f.run(&["-u", "-l", "a"]).0, 0);
    assert_eq!(f.run(&["--sort", "width"]).1, "a\nb\nc\n");
    assert_eq!(f.run(&["--sort=version"]).1, "a\nb\nc\n");
    assert_eq!(f.run(&["--time=use", "-l", "a"]).0, 0);
    assert_eq!(f.run(&["--time=modification", "-l", "a"]).0, 0);
    assert_eq!(f.run(&["--time=birth", "-l", "a"]).0, 0);
    assert_eq!(f.run(&["--numeric-uid-gid", "a"]).0, 0);
    assert_eq!(f.run(&["--kibibytes", "-s", "a"]).0, 0);
    assert_eq!(f.run(&["--block-size=M", "-s", "b"]).0, 0);
    assert_eq!(f.run(&["--block-size=MB", "-s", "b"]).0, 0);
    assert_eq!(f.run(&["--block-size=1", "-s", "b"]).0, 0);
    assert_eq!(f.run(&["-k", "-s", "a"]).0, 0);
    assert_eq!(f.run(&["-v", "a"]).1, "a\n");

    let long_human = f.run(&["-lh", "b"]).1;
    assert!(long_human.contains("2.0K") || long_human.contains("2K"));
    let long_human = f.run(&["-lh", "c"]).1;
    assert!(long_human.contains("20K"));

    let (code, _, stderr) = f.run(&["-a-"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("use '--'"));
}

#[test]
fn mode_strings_cover_special_permission_combinations() {
    let f = Fixture::new("modes");
    f.file("setuid_exec", b"");
    f.file("setuid_noexec", b"");
    f.file("sticky_exec", b"");
    f.file("sticky_noexec", b"");
    f.dir("directory");
    f.symlink("setuid_exec", "symlink");
    f.chmod("setuid_exec", 0o4755);
    f.chmod("setuid_noexec", 0o4644);
    f.chmod("sticky_exec", 0o1755);
    f.chmod("sticky_noexec", 0o1644);

    let output = f
        .run(&[
            "-ld",
            "setuid_exec",
            "setuid_noexec",
            "sticky_exec",
            "sticky_noexec",
            "directory",
            "symlink",
        ])
        .1;

    if fs::metadata(f.root.join("setuid_exec"))
        .unwrap()
        .permissions()
        .mode()
        & 0o4000
        != 0
    {
        assert!(output.contains("-rwsr-xr-x"));
        assert!(output.contains("-rwSr--r--"));
    }
    assert!(output.contains("drwx"));
    assert!(output.contains("lrwx"));
    assert!(output.contains("-rwxr-xr-t"));
    assert!(output.contains("-rw-r--r-T"));
}

#[test]
fn fifo_and_socket_indicators_are_rendered_when_present() {
    let f = Fixture::new("special-files");
    let fifo = f.root.join("pipe");
    let status = Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(status.success());
    let listener = UnixListener::bind(f.root.join("sock")).ok();

    let output = f.run(&["--indicator-style=file-type"]).1;
    let long_output = f.run(&["-l", "pipe"]).1;

    drop(listener);
    assert!(output.contains("pipe|"));
    assert!(long_output.contains("prw"));
    if output.contains("sock") {
        assert!(output.contains("sock="));
    }
}

#[test]
fn platform_special_files_cover_long_type_letters_when_available() {
    let f = Fixture::new("platform-special");

    if Path::new("/dev/null").exists() {
        assert!(f.run(&["-l", "/dev/null"]).1.contains("crw"));
    }
    if Path::new("/dev/disk0").exists() {
        assert!(f.run(&["-l", "/dev/disk0"]).1.contains("brw"));
    }
    for candidate in ["/var/run/syslog", "/var/run/mDNSResponder"] {
        if Path::new(candidate).exists() {
            let output = f.run(&["--indicator-style=file-type", candidate]).1;
            assert!(output.contains('='));
            let long = f.run(&["-l", candidate]).1;
            assert!(long.contains("srw"));
            break;
        }
    }
}

#[test]
fn dereferencing_broken_symlink_inside_directory_reports_minor_trouble() {
    let f = Fixture::new("broken-link");
    f.symlink("missing", "broken");

    let (code, _, stderr) = f.run(&["-L"]);

    assert_eq!(code, 1);
    assert!(stderr.contains("cannot access 'broken'"));
}

#[test]
fn long_format_handles_pre_epoch_timestamps() {
    let f = Fixture::new("old-time");
    f.file("old", b"");
    let status = Command::new("touch")
        .arg("-t")
        .arg("196001010000")
        .arg(f.root.join("old"))
        .status()
        .unwrap();
    assert!(status.success());

    let output = f.run(&["-l", "old"]).1;

    assert!(output.contains(" -"));
}

#[test]
fn unreadable_directory_reports_minor_trouble_when_platform_denies_reading() {
    let f = Fixture::new("unreadable");
    f.dir("closed");
    f.chmod("closed", 0o0);

    let (code, _, stderr) = f.run(&["closed"]);

    f.chmod("closed", 0o755);
    if code == 1 {
        assert!(stderr.contains("cannot open directory"));
    }
}

#[test]
fn recursive_unreadable_child_reports_minor_trouble_when_platform_denies_reading() {
    let f = Fixture::new("recursive-unreadable");
    f.dir("parent");
    fs::create_dir(f.root.join("parent").join("closed")).unwrap();
    let closed = f.root.join("parent").join("closed");
    let mut perms = fs::metadata(&closed).unwrap().permissions();
    perms.set_mode(0o0);
    fs::set_permissions(&closed, perms).unwrap();

    let (code, _, stderr) = f.run(&["-R", "parent"]);

    let mut perms = fs::metadata(&closed).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&closed, perms).unwrap();
    if code == 1 {
        assert!(stderr.contains("cannot open directory"));
    }
}

#[test]
fn multiple_directories_are_separated_and_absolute_operands_are_supported() {
    let f = Fixture::new("multi-dir");
    f.dir("a");
    f.dir("b");
    fs::write(f.root.join("a").join("one"), b"").unwrap();
    fs::write(f.root.join("b").join("two"), b"").unwrap();

    let absolute = f.root.join("a");
    let (code, stdout, stderr) = f.run(&[absolute.to_str().unwrap(), "b"]);

    assert_eq!(code, 0);
    assert!(stdout.contains("a:\none\n\nb:\ntwo\n"));
    assert_eq!(stderr, "");
}

#[test]
fn help_version_errors_and_option_terminator_match_gnu_shapes() {
    let f = Fixture::new("meta");
    f.file("-dash", b"");

    let (_, help, _) = f.run(&["--help"]);
    assert!(help.starts_with("Usage: ls [OPTION]... [FILE]..."));
    assert!(help.contains("--sort=WORD"));
    assert_eq!(f.run(&["--version"]).1, "ls (rust-unix-tools) 0.1.0\n");
    assert_eq!(f.run(&["--", "-dash"]).1, "-dash\n");

    let (code, _, stderr) = f.run(&["--definitely-not-real"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("unrecognized option"));

    let (code, _, stderr) = f.run(&["missing"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("cannot access 'missing'"));
}

#[test]
fn invalid_option_arguments_report_serious_trouble() {
    let f = Fixture::new("bad-options");
    assert_eq!(f.run(&["--sort=bad"]).0, 2);
    assert_eq!(f.run(&["--time=bad"]).0, 2);
    assert_eq!(f.run(&["--block-size=bad"]).0, 2);
    assert_eq!(f.run(&["--format=bad"]).0, 2);
    assert_eq!(f.run(&["--indicator-style=bad"]).0, 2);
    assert_eq!(f.run(&["--width"]).0, 2);
    assert_eq!(f.run(&["-?"]).0, 2);
}

#[test]
fn test_dotfile_extension_sort() {
    let f = Fixture::new("dotfile-extension");
    f.file(".z_no_extension", b"");
    f.file("a.txt", b"");
    f.file(".bashrc", b"");

    let output = f.run(&["-A", "--sort=extension"]).1;

    assert_eq!(output, ".bashrc\n.z_no_extension\na.txt\n");
}

#[test]
fn test_recursive_directory_loop_detected() {
    let f = Fixture::new("recursive-loop");
    f.dir("parent");
    f.symlink("../parent", "parent/loop_link");

    let (code, _, stderr) = f.run(&["-R", "-L", "parent"]);

    assert_eq!(code, 1);
    assert!(stderr.contains("is part of a loop"));
}

#[test]
fn test_recursive_relative_headers() {
    let f = Fixture::new("relative-headers");
    f.dir("dir");
    f.dir("dir/sub");
    f.file("dir/sub/file", b"");

    let stdout = f.run(&["-R", "dir"]).1;

    assert!(stdout.contains("dir/sub:\nfile\n"));
    assert!(!stdout.contains(&f.root.display().to_string()));
}

