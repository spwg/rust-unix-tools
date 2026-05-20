//! GNU-style `cat`.
//!
//! This module implements the core logic of the `cat` command, supporting
//! option parsing and line-by-line formatting matching GNU coreutils.


use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

struct CatOptions {
    number: bool,
    number_nonblank: bool,
    squeeze_blank: bool,
    show_ends: bool,
    show_tabs: bool,
    show_nonprinting: bool,
}

impl CatOptions {
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
    let args_iter = args.into_iter();

    // The first argument is typically the binary name, but since we receive args directly,
    // we should process all of them. Let's parse options.
    let mut double_dash = false;
    for arg in args_iter {
        let arg_str = arg.to_string_lossy();
        if double_dash {
            files.push(arg);
        } else if arg_str == "--" {
            double_dash = true;
        } else if arg_str.starts_with("--") {
            match arg_str.as_ref() {
                "--show-all" => {
                    options.show_nonprinting = true;
                    options.show_ends = true;
                    options.show_tabs = true;
                }
                "--number-nonblank" => {
                    options.number_nonblank = true;
                }
                "--show-ends" => {
                    options.show_ends = true;
                }
                "--number" => {
                    options.number = true;
                }
                "--squeeze-blank" => {
                    options.squeeze_blank = true;
                }
                "--show-tabs" => {
                    options.show_tabs = true;
                }
                "--show-nonprinting" => {
                    options.show_nonprinting = true;
                }
                "--help" => {
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
                "--version" => {
                    let _ = writeln!(stdout, "cat (rust-unix-tools) 0.1.0");
                    return 0;
                }
                _ => {
                    let _ = writeln!(stderr, "cat: unrecognized option '{}'", arg_str);
                    let _ = writeln!(stderr, "Try 'cat --help' for more information.");
                    return 1;
                }
            }
        } else if arg_str.starts_with('-') && arg_str != "-" {
            // Short options
            for c in arg_str.chars().skip(1) {
                match c {
                    'A' => {
                        options.show_nonprinting = true;
                        options.show_ends = true;
                        options.show_tabs = true;
                    }
                    'b' => {
                        options.number_nonblank = true;
                    }
                    'e' => {
                        options.show_nonprinting = true;
                        options.show_ends = true;
                    }
                    'E' => {
                        options.show_ends = true;
                    }
                    'n' => {
                        options.number = true;
                    }
                    's' => {
                        options.squeeze_blank = true;
                    }
                    't' => {
                        options.show_nonprinting = true;
                        options.show_tabs = true;
                    }
                    'T' => {
                        options.show_tabs = true;
                    }
                    'u' => {
                        // Ignored for POSIX compliance
                    }
                    'v' => {
                        options.show_nonprinting = true;
                    }
                    _ => {
                        let _ = writeln!(stderr, "cat: invalid option -- '{}'", c);
                        let _ = writeln!(stderr, "Try 'cat --help' for more information.");
                        return 1;
                    }
                }
            }
        } else {
            files.push(arg);
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
                        let _ = writeln!(stderr, "cat: {}: {}", file_arg.to_string_lossy(), e);
                        exit_code = 1;
                    }
                }
                Err(e) => {
                    let _ = writeln!(stderr, "cat: {}: {}", file_arg.to_string_lossy(), e);
                    exit_code = 1;
                }
            }
        }
    }

    exit_code
}

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
            if *at_line_start {
                if byte == b'\n' {
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

                    if options.show_ends {
                        writer.write_all(b"$")?;
                    }
                    writer.write_all(b"\n")?;
                    *at_line_start = true;
                } else {
                    *consecutive_empty_lines = 0;
                    if options.number || options.number_nonblank {
                        *line_number += 1;
                        write!(writer, "{:6}\t", line_number)?;
                    }
                    *at_line_start = false;
                    write_byte(byte, writer, options)?;
                }
            } else {
                if byte == b'\n' {
                    if options.show_ends {
                        writer.write_all(b"$")?;
                    }
                    writer.write_all(b"\n")?;
                    *at_line_start = true;
                } else {
                    write_byte(byte, writer, options)?;
                }
            }
        }
    }
    Ok(())
}

#[inline]
fn write_byte(
    byte: u8,
    writer: &mut impl Write,
    options: &CatOptions,
) -> io::Result<()> {
    if byte == b'\t' {
        if options.show_tabs {
            writer.write_all(b"^I")?;
        } else {
            writer.write_all(b"\t")?;
        }
    } else if options.show_nonprinting {
        if byte >= 128 {
            writer.write_all(b"M-")?;
            let b = byte - 128;
            if b < 32 {
                writer.write_all(&[b'^', b + 64])?;
            } else if b == 127 {
                writer.write_all(b"^?")?;
            } else {
                writer.write_all(&[b])?;
            }
        } else if byte < 32 {
            writer.write_all(&[b'^', byte + 64])?;
        } else if byte == 127 {
            writer.write_all(b"^?")?;
        } else {
            writer.write_all(&[byte])?;
        }
    } else {
        writer.write_all(&[byte])?;
    }
    Ok(())
}
