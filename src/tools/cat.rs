//! GNU-style `cat`.
//!
//! This module implements the core logic of the `cat` command, supporting
//! option parsing and line-by-line formatting matching GNU coreutils.
//! 
//! [cat.rs](file:///Users/spencergreene/github/rust-unix-tools/src/tools/cat.rs)

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

/// Options for the `cat` utility.
struct CatOptions {
    /// -n, --number: Number all output lines.
    number: bool,
    /// -b, --number-nonblank: Number nonempty output lines, overrides -n.
    number_nonblank: bool,
    /// -s, --squeeze-blank: Suppress repeated empty output lines.
    squeeze_blank: bool,
    /// -E, --show-ends: Display $ at end of each line.
    show_ends: bool,
    /// -T, --show-tabs: Display TAB characters as ^I.
    show_tabs: bool,
    /// -v, --show-nonprinting: Use ^ and M- notation, except for LFD and TAB.
    show_nonprinting: bool,
}

impl CatOptions {
    /// Create a new default set of `CatOptions`.
    fn new() -> Self {
        Self {
            number: false,
            number_nonblank: false,
            squeeze_blank: false,
            show_ends: false,
            show_tabs: false,
            show_nonprinting: false,
        }
    }
}

/// Run the `cat` utility with the given arguments, reading from and writing to the provided streams.
pub fn run<I>(
    args: I,
    cwd: &Path,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> i32
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let stdin = io::stdin();
    let mut handle = stdin.lock();
    run_impl(args, cwd, &mut handle, stdout, stderr)
}

/// Helper that implements the core `cat` logic but allows passing a mock stdin reader.
pub fn run_impl<I>(
    args: I,
    cwd: &Path,
    stdin: &mut impl Read,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> i32
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let mut options = CatOptions::new();
    let mut files = Vec::new();
    let args_vec: Vec<std::ffi::OsString> = args.into_iter().collect();

    let specs = &[
        crate::getopt::OptSpec {
            short: Some('A'),
            long: Some("show-all"),
            has_arg: crate::getopt::HasArg::No,
        },
        crate::getopt::OptSpec {
            short: Some('b'),
            long: Some("number-nonblank"),
            has_arg: crate::getopt::HasArg::No,
        },
        crate::getopt::OptSpec {
            short: Some('e'),
            long: None,
            has_arg: crate::getopt::HasArg::No,
        },
        crate::getopt::OptSpec {
            short: Some('E'),
            long: Some("show-ends"),
            has_arg: crate::getopt::HasArg::No,
        },
        crate::getopt::OptSpec {
            short: Some('n'),
            long: Some("number"),
            has_arg: crate::getopt::HasArg::No,
        },
        crate::getopt::OptSpec {
            short: Some('s'),
            long: Some("squeeze-blank"),
            has_arg: crate::getopt::HasArg::No,
        },
        crate::getopt::OptSpec {
            short: Some('t'),
            long: None,
            has_arg: crate::getopt::HasArg::No,
        },
        crate::getopt::OptSpec {
            short: Some('T'),
            long: Some("show-tabs"),
            has_arg: crate::getopt::HasArg::No,
        },
        crate::getopt::OptSpec {
            short: Some('u'),
            long: None,
            has_arg: crate::getopt::HasArg::No,
        },
        crate::getopt::OptSpec {
            short: Some('v'),
            long: Some("show-nonprinting"),
            has_arg: crate::getopt::HasArg::No,
        },
        crate::getopt::OptSpec {
            short: None,
            long: Some("help"),
            has_arg: crate::getopt::HasArg::No,
        },
        crate::getopt::OptSpec {
            short: None,
            long: Some("version"),
            has_arg: crate::getopt::HasArg::No,
        },
    ];

    let parsed_args = match crate::getopt::parse(&args_vec, specs, false) {
        Ok(res) => res,
        Err(e) => {
            let _ = writeln!(stderr, "cat: {}", e);
            return 1;
        }
    };

    for arg in parsed_args {
        match arg {
            crate::getopt::ParsedArg::Option { short, long, value: _ } => {
                match (short, long) {
                    (_, Some("help")) => {
                        let _ = writeln!(
                            stdout,
                            "Usage: cat [OPTION]... [FILE]...\n\
                             Concatenate FILE(s) to standard output.\n\n\
                                -A, --show-all           equivalent to -vET\n\
                                -b, --number-nonblank    number nonempty output lines, overrides -n\n\
                                -e                       equivalent to -vE\n\
                                -E, --show-ends          display $ at end of each line\n\
                                -n, --number             number all output lines\n\
                                -s, --squeeze-blank      suppress repeated empty output lines\n\
                                -t                       equivalent to -vT\n\
                                -T, --show-tabs          display TAB characters as ^I\n\
                                -u                       (ignored)\n\
                                -v, --show-nonprinting   use ^ and M- notation, except for LFD and TAB\n\
                                    --help        display this help and exit\n\
                                    --version     output version information and exit"
                        );
                        return 0;
                    }
                    (_, Some("version")) => {
                        let _ = writeln!(stdout, "cat (rust-unix-tools) 0.1.0");
                        return 0;
                    }
                    (Some('A'), _) | (_, Some("show-all")) => {
                        options.show_nonprinting = true;
                        options.show_ends = true;
                        options.show_tabs = true;
                    }
                    (Some('b'), _) | (_, Some("number-nonblank")) => {
                        options.number = true;
                        options.number_nonblank = true;
                    }
                    (Some('e'), _) => {
                        options.show_nonprinting = true;
                        options.show_ends = true;
                    }
                    (Some('E'), _) | (_, Some("show-ends")) => {
                        options.show_ends = true;
                    }
                    (Some('n'), _) | (_, Some("number")) => {
                        options.number = true;
                    }
                    (Some('s'), _) | (_, Some("squeeze-blank")) => {
                        options.squeeze_blank = true;
                    }
                    (Some('t'), _) => {
                        options.show_nonprinting = true;
                        options.show_tabs = true;
                    }
                    (Some('T'), _) | (_, Some("show-tabs")) => {
                        options.show_tabs = true;
                    }
                    (Some('u'), _) => {
                        // Ignored for POSIX compliance
                    }
                    (Some('v'), _) | (_, Some("show-nonprinting")) => {
                        options.show_nonprinting = true;
                    }
                    _ => unreachable!(),
                }
            }
            crate::getopt::ParsedArg::Operand(operand) => {
                files.push(operand.to_owned());
            }
        }
    }

    if files.is_empty() {
        files.push(std::ffi::OsString::from("-"));
    }

    let mut exit_code = 0;
    let mut at_line_start = true;
    let mut consecutive_empty_lines = 0;
    let mut line_number = 0;

    for file_arg in files {
        if file_arg == "-" {
            if let Err(e) = process_reader(
                stdin,
                stdout,
                &options,
                &mut at_line_start,
                &mut consecutive_empty_lines,
                &mut line_number,
            ) {
                let _ = writeln!(stderr, "cat: <stdin>: {}", e);
                exit_code = 1;
            }
        } else {
            let path = cwd.join(&file_arg);
            match File::open(&path) {
                Ok(mut file) => {
                    if let Err(e) = process_reader(
                        &mut file,
                        stdout,
                        &options,
                        &mut at_line_start,
                        &mut consecutive_empty_lines,
                        &mut line_number,
                    ) {
                        // to_string_lossy() converts potentially non-UTF-8 UNIX arguments to a lossy String
                        // replacement character where needed for safe output/logging formatting.
                        let _ = writeln!(stderr, "cat: {}: {}", file_arg.to_string_lossy(), e);
                        exit_code = 1;
                    }
                }
                Err(e) => {
                    // to_string_lossy() converts potentially non-UTF-8 UNIX arguments to a lossy String
                    // replacement character where needed for safe output/logging formatting.
                    let _ = writeln!(stderr, "cat: {}: {}", file_arg.to_string_lossy(), e);
                    exit_code = 1;
                }
            }
        }
    }

    exit_code
}

/// Reads bytes from `reader`, formats them according to `options`, and writes them to `writer`.
///
/// Keeps track of state across multiple files:
/// - `at_line_start` tracks whether we are at the start of a line.
/// - `consecutive_empty_lines` counts consecutive empty lines to support option `-s` / `--squeeze-blank`.
/// - `line_number` tracks the current printed line number for options `-n` and `-b`.
///
/// Uses a 4096-byte buffer which is the standard page size for efficient disk I/O,
/// reducing the number of syscalls without consuming excessive stack space.
fn process_reader(
    reader: &mut impl Read,
    writer: &mut impl Write,
    options: &CatOptions,
    at_line_start: &mut bool,
    consecutive_empty_lines: &mut usize,
    line_number: &mut usize,
) -> io::Result<()> {
    let mut buf = [0; 4096];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        for &byte in chunk {
            if byte == b'\n' {
                if *at_line_start {
                    if options.squeeze_blank && *consecutive_empty_lines > 0 {
                        // Skip empty line
                        continue;
                    }
                    *consecutive_empty_lines += 1;

                    // If numbering is enabled, but number_nonblank is false
                    if options.number && !options.number_nonblank {
                        *line_number += 1;
                        write!(writer, "{:6}\t", line_number)?;
                    }
                }
                if options.show_ends {
                    writer.write_all(b"$")?;
                }
                writer.write_all(b"\n")?;
                *at_line_start = true;
            } else {
                if *at_line_start {
                    *consecutive_empty_lines = 0;
                    if options.number || options.number_nonblank {
                        *line_number += 1;
                        write!(writer, "{:6}\t", line_number)?;
                    }
                    *at_line_start = false;
                }
                write_byte(byte, writer, options)?;
            }
        }
    }
    Ok(())
}

/// Write a single byte to the writer, formatting tabs and nonprinting characters if specified.
#[inline]
fn write_byte(
    byte: u8,
    writer: &mut impl Write,
    options: &CatOptions,
) -> io::Result<()> {
    match byte {
        b'\t' if options.show_tabs => writer.write_all(b"^I"),
        b'\t' => writer.write_all(b"\t"),
        _ if options.show_nonprinting => write_nonprinting_byte(byte, writer),
        _ => writer.write_all(&[byte]),
    }
}

/// Format a byte as standard UNIX nonprinting/meta character notation (e.g. `^A`, `^?`, `M-x`, `M-^A`).
fn write_nonprinting_byte(byte: u8, writer: &mut impl Write) -> io::Result<()> {
    let mut b = byte;
    if b >= 128 {
        writer.write_all(b"M-")?;
        b -= 128;
    }

    match b {
        0..=31 => writer.write_all(&[b'^', b + 64]),
        127 => writer.write_all(b"^?"),
        _ => writer.write_all(&[b]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_nonprinting_byte() {
        let cases = vec![
            (0, b"^@".to_vec()),
            (9, b"^I".to_vec()),
            (31, b"^_".to_vec()),
            (32, b" ".to_vec()),
            (126, b"~".to_vec()),
            (127, b"^?".to_vec()),
            (128, b"M-^@".to_vec()),
            (159, b"M-^_".to_vec()),
            (160, b"M- ".to_vec()),
            (254, b"M-~".to_vec()),
            (255, b"M-^?".to_vec()),
        ];
        for (input, expected) in cases {
            let mut buf = Vec::new();
            write_nonprinting_byte(input, &mut buf).unwrap();
            assert_eq!(buf, expected, "Failed for byte {}", input);
        }
    }

    #[test]
    fn test_write_byte() {
        // Tab display option
        let mut opts = CatOptions::new();
        opts.show_tabs = true;
        let mut buf = Vec::new();
        write_byte(b'\t', &mut buf, &opts).unwrap();
        assert_eq!(buf, b"^I");

        let mut opts = CatOptions::new();
        opts.show_tabs = false;
        let mut buf = Vec::new();
        write_byte(b'\t', &mut buf, &opts).unwrap();
        assert_eq!(buf, b"\t");

        // Nonprinting option
        let mut opts = CatOptions::new();
        opts.show_nonprinting = true;
        let mut buf = Vec::new();
        write_byte(128, &mut buf, &opts).unwrap();
        assert_eq!(buf, b"M-^@");

        let mut opts = CatOptions::new();
        opts.show_nonprinting = false;
        let mut buf = Vec::new();
        write_byte(128, &mut buf, &opts).unwrap();
        assert_eq!(buf, &[128]);
    }

    #[test]
    fn test_process_reader_line_numbering() {
        let mut options = CatOptions::new();
        options.number = true;

        let input = b"hello\nworld\n";
        let mut output = Vec::new();
        let mut at_line_start = true;
        let mut consecutive_empty_lines = 0;
        let mut line_number = 0;

        process_reader(
            &mut &input[..],
            &mut output,
            &options,
            &mut at_line_start,
            &mut consecutive_empty_lines,
            &mut line_number,
        )
        .unwrap();

        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(output_str, "     1\thello\n     2\tworld\n");
    }

    #[test]
    fn test_process_reader_squeeze_blank() {
        let mut options = CatOptions::new();
        options.squeeze_blank = true;

        let input = b"\n\n\nhello\n\n\n";
        let mut output = Vec::new();
        let mut at_line_start = true;
        let mut consecutive_empty_lines = 0;
        let mut line_number = 0;

        process_reader(
            &mut &input[..],
            &mut output,
            &options,
            &mut at_line_start,
            &mut consecutive_empty_lines,
            &mut line_number,
        )
        .unwrap();

        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(output_str, "\nhello\n\n");
    }

    #[test]
    fn test_process_reader_number_nonblank() {
        let mut options = CatOptions::new();
        options.number_nonblank = true;

        let input = b"hello\n\nworld\n";
        let mut output = Vec::new();
        let mut at_line_start = true;
        let mut consecutive_empty_lines = 0;
        let mut line_number = 0;

        process_reader(
            &mut &input[..],
            &mut output,
            &options,
            &mut at_line_start,
            &mut consecutive_empty_lines,
            &mut line_number,
        )
        .unwrap();

        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(output_str, "     1\thello\n\n     2\tworld\n");
    }

    #[test]
    fn test_cat_help() {
        let args = vec![std::ffi::OsString::from("--help")];
        let cwd = Path::new(".");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(args, cwd, &mut stdout, &mut stderr);
        assert_eq!(code, 0);
        let out_str = String::from_utf8_lossy(&stdout);
        assert!(out_str.contains("Usage: cat [OPTION]... [FILE]..."));
    }

    #[test]
    fn test_cat_version() {
        let args = vec![std::ffi::OsString::from("--version")];
        let cwd = Path::new(".");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(args, cwd, &mut stdout, &mut stderr);
        assert_eq!(code, 0);
        let out_str = String::from_utf8_lossy(&stdout);
        assert!(out_str.contains("cat (rust-unix-tools)"));
    }

    #[test]
    fn test_cat_ignored_u() {
        let temp_path = Path::new("temp_test_cat_u.txt");
        std::fs::write(temp_path, "hello").unwrap();

        let args = vec![
            std::ffi::OsString::from("-u"),
            std::ffi::OsString::from("temp_test_cat_u.txt"),
        ];
        let cwd = Path::new(".");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(args, cwd, &mut stdout, &mut stderr);
        
        let _ = std::fs::remove_file(temp_path);
        
        assert_eq!(code, 0);
        assert_eq!(stdout, b"hello");
    }

    #[test]
    fn test_cat_invalid_file() {
        let args = vec![std::ffi::OsString::from("nonexistent_file_xyz.txt")];
        let cwd = Path::new(".");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(args, cwd, &mut stdout, &mut stderr);
        assert_eq!(code, 1);
        let err_str = String::from_utf8_lossy(&stderr);
        assert!(err_str.contains("nonexistent_file_xyz.txt"));
    }

    #[test]
    fn test_cat_option_errors() {
        let args = vec![std::ffi::OsString::from("--invalid-option")];
        let cwd = Path::new(".");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(args, cwd, &mut stdout, &mut stderr);
        assert_ne!(code, 0);
        assert!(String::from_utf8_lossy(&stderr).contains("unrecognized option"));
    }

    #[test]
    fn test_cat_run_stdin() {
        let args = vec![std::ffi::OsString::from("-")];
        let cwd = Path::new(".");
        let mut stdin = io::Cursor::new("hello stdin");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_impl(args, cwd, &mut stdin, &mut stdout, &mut stderr);
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8_lossy(&stdout), "hello stdin");
    }

    #[test]
    fn test_cat_run_no_args() {
        let args: Vec<std::ffi::OsString> = vec![];
        let cwd = Path::new(".");
        let mut stdin = io::Cursor::new("hello standard input");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_impl(args, cwd, &mut stdin, &mut stdout, &mut stderr);
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8_lossy(&stdout), "hello standard input");
    }

    #[test]
    fn test_cat_directory_error() {
        let args = vec![std::ffi::OsString::from(".")];
        let cwd = Path::new(".");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(args, cwd, &mut stdout, &mut stderr);
        assert_ne!(code, 0);
        let err_str = String::from_utf8_lossy(&stderr);
        assert!(err_str.contains("Is a directory") || err_str.contains("is a directory") || err_str.contains("Permission denied"));
    }

    #[test]
    fn test_cat_various_options() {
        let temp_path = Path::new("temp_test_cat_opts.txt");
        std::fs::write(temp_path, "hello\tworld\n\nline3\n").unwrap();

        // Option -A
        let args = vec![
            std::ffi::OsString::from("-A"),
            std::ffi::OsString::from("temp_test_cat_opts.txt"),
        ];
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(run(args, Path::new("."), &mut stdout, &mut stderr), 0);
        assert_eq!(String::from_utf8_lossy(&stdout), "hello^Iworld$\n$\nline3$\n");

        // Option -b
        let args = vec![
            std::ffi::OsString::from("-b"),
            std::ffi::OsString::from("temp_test_cat_opts.txt"),
        ];
        stdout.clear();
        assert_eq!(run(args, Path::new("."), &mut stdout, &mut stderr), 0);
        assert_eq!(String::from_utf8_lossy(&stdout), "     1\thello\tworld\n\n     2\tline3\n");

        // Option -e
        let args = vec![
            std::ffi::OsString::from("-e"),
            std::ffi::OsString::from("temp_test_cat_opts.txt"),
        ];
        stdout.clear();
        assert_eq!(run(args, Path::new("."), &mut stdout, &mut stderr), 0);
        assert_eq!(String::from_utf8_lossy(&stdout), "hello\tworld$\n$\nline3$\n");

        // Option -t
        let args = vec![
            std::ffi::OsString::from("-t"),
            std::ffi::OsString::from("temp_test_cat_opts.txt"),
        ];
        stdout.clear();
        assert_eq!(run(args, Path::new("."), &mut stdout, &mut stderr), 0);
        assert_eq!(String::from_utf8_lossy(&stdout), "hello^Iworld\n\nline3\n");

        let _ = std::fs::remove_file(temp_path);
    }

    #[test]
    fn test_cat_long_options() {
        let temp_path = Path::new("temp_test_cat_long_opts.txt");
        std::fs::write(temp_path, "hello\tworld\n\nline3\n").unwrap();

        // Option --show-all
        let args = vec![
            std::ffi::OsString::from("--show-all"),
            std::ffi::OsString::from("temp_test_cat_long_opts.txt"),
        ];
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(run(args, Path::new("."), &mut stdout, &mut stderr), 0);
        assert_eq!(String::from_utf8_lossy(&stdout), "hello^Iworld$\n$\nline3$\n");

        // Option --number-nonblank
        let args = vec![
            std::ffi::OsString::from("--number-nonblank"),
            std::ffi::OsString::from("temp_test_cat_long_opts.txt"),
        ];
        stdout.clear();
        assert_eq!(run(args, Path::new("."), &mut stdout, &mut stderr), 0);
        assert_eq!(String::from_utf8_lossy(&stdout), "     1\thello\tworld\n\n     2\tline3\n");

        // Option --show-ends
        let args = vec![
            std::ffi::OsString::from("--show-ends"),
            std::ffi::OsString::from("temp_test_cat_long_opts.txt"),
        ];
        stdout.clear();
        assert_eq!(run(args, Path::new("."), &mut stdout, &mut stderr), 0);
        assert_eq!(String::from_utf8_lossy(&stdout), "hello\tworld$\n$\nline3$\n");

        // Option --number
        let args = vec![
            std::ffi::OsString::from("--number"),
            std::ffi::OsString::from("temp_test_cat_long_opts.txt"),
        ];
        stdout.clear();
        assert_eq!(run(args, Path::new("."), &mut stdout, &mut stderr), 0);
        assert_eq!(String::from_utf8_lossy(&stdout), "     1\thello\tworld\n     2\t\n     3\tline3\n");

        // Option --squeeze-blank
        let args = vec![
            std::ffi::OsString::from("--squeeze-blank"),
            std::ffi::OsString::from("temp_test_cat_long_opts.txt"),
        ];
        stdout.clear();
        assert_eq!(run(args, Path::new("."), &mut stdout, &mut stderr), 0);
        assert_eq!(String::from_utf8_lossy(&stdout), "hello\tworld\n\nline3\n");

        // Option --show-tabs
        let args = vec![
            std::ffi::OsString::from("--show-tabs"),
            std::ffi::OsString::from("temp_test_cat_long_opts.txt"),
        ];
        stdout.clear();
        assert_eq!(run(args, Path::new("."), &mut stdout, &mut stderr), 0);
        assert_eq!(String::from_utf8_lossy(&stdout), "hello^Iworld\n\nline3\n");

        // Option --show-nonprinting
        let args = vec![
            std::ffi::OsString::from("--show-nonprinting"),
            std::ffi::OsString::from("temp_test_cat_long_opts.txt"),
        ];
        stdout.clear();
        assert_eq!(run(args, Path::new("."), &mut stdout, &mut stderr), 0);
        assert_eq!(String::from_utf8_lossy(&stdout), "hello\tworld\n\nline3\n");

        let _ = std::fs::remove_file(temp_path);
    }

    struct FailingReader;
    impl std::io::Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::Other, "mock error"))
        }
    }

    #[test]
    fn test_cat_run_failing_stdin() {
        let args = vec![std::ffi::OsString::from("-")];
        let cwd = Path::new(".");
        let mut stdin = FailingReader;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_impl(args, cwd, &mut stdin, &mut stdout, &mut stderr);
        assert_eq!(code, 1);
        assert!(String::from_utf8_lossy(&stderr).contains("mock error"));
    }
}
