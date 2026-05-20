//! GNU-style `cat`.
//!
//! This module implements the core logic of the `cat` command, supporting
//! option parsing and line-by-line formatting matching GNU coreutils.

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
        Ok(args) => args,
        Err(err) => {
            let _ = writeln!(stderr, "cat: {}", err);
            let _ = writeln!(stderr, "Try 'cat --help' for more information.");
            return 1;
        }
    };

    for arg in parsed_args {
        match arg {
            crate::getopt::ParsedArg::Option { short, long, .. } => {
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

    let stdin = io::stdin();

    for file_arg in files {
        if file_arg == "-" {
            let mut handle = stdin.lock();
            if let Err(e) = process_reader(
                &mut handle,
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
}
