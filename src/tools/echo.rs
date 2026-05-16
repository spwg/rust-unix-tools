//! GNU-style `echo`.
//!
//! This module writes operands to a byte-oriented writer, preserving raw Unix
//! argument bytes and matching GNU `echo` option and escape handling for the
//! supported behavior documented in this repository.

use std::env;
use std::io::{self, Write};

fn parse_octal(bytes: &[u8], mut index: usize) -> (u8, usize) {
    let start = index;
    let mut val: u16 = 0;

    while index - start < 3 && index < bytes.len() && bytes[index] >= b'0' && bytes[index] <= b'7' {
        val = (val << 3) | ((bytes[index] - b'0') as u16);
        index += 1;
    }

    ((val % 256) as u8, index - start)
}

fn parse_hex(bytes: &[u8], mut index: usize) -> (u8, usize) {
    let start = index;
    let mut val: u8 = 0;

    while index - start < 2 && index < bytes.len() {
        let b = bytes[index];
        if b.is_ascii_hexdigit() {
            let digit = match b {
                b'0'..=b'9' => b - b'0',
                b'A'..=b'F' => b - b'A' + 10,
                b'a'..=b'f' => b - b'a' + 10,
                _ => unreachable!(),
            };
            val = (val << 4) | digit;
            index += 1;
        } else {
            break;
        }
    }

    (val, index - start)
}

fn write_arg(bytes: &[u8], writer: &mut impl Write, enable_escapes: bool) -> io::Result<bool> {
    if !enable_escapes {
        writer.write_all(bytes)?;
        return Ok(true);
    }

    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'a' => {
                    writer.write_all(b"\x07")?;
                    i += 2;
                }
                b'b' => {
                    writer.write_all(b"\x08")?;
                    i += 2;
                }
                b'c' => return Ok(false),
                b'e' => {
                    writer.write_all(b"\x1B")?;
                    i += 2;
                }
                b'f' => {
                    writer.write_all(b"\x0C")?;
                    i += 2;
                }
                b'n' => {
                    writer.write_all(b"\n")?;
                    i += 2;
                }
                b'r' => {
                    writer.write_all(b"\r")?;
                    i += 2;
                }
                b't' => {
                    writer.write_all(b"\t")?;
                    i += 2;
                }
                b'v' => {
                    writer.write_all(b"\x0B")?;
                    i += 2;
                }
                b'\\' => {
                    writer.write_all(b"\\")?;
                    i += 2;
                }
                b'0' => {
                    let (val, consumed) = parse_octal(bytes, i + 2);
                    writer.write_all(&[val])?;
                    i += 2 + consumed;
                }
                b'x' => {
                    let (val, consumed) = parse_hex(bytes, i + 2);
                    if consumed == 0 {
                        writer.write_all(b"\\x")?;
                    } else {
                        writer.write_all(&[val])?;
                    }
                    i += 2 + consumed;
                }
                b'1'..=b'7' => {
                    let (val, consumed) = parse_octal(bytes, i + 1);
                    writer.write_all(&[val])?;
                    i += 1 + consumed;
                }
                _ => {
                    writer.write_all(&[b'\\', bytes[i + 1]])?;
                    i += 2;
                }
            }
        } else if bytes[i] == b'\\' {
            writer.write_all(b"\\")?;
            i += 1;
        } else {
            writer.write_all(&[bytes[i]])?;
            i += 1;
        }
    }
    Ok(true)
}

/// Writes `args` to `writer` using GNU-style `echo` semantics.
///
/// Arguments are consumed as byte slices instead of UTF-8 strings so invalid
/// Unix argument bytes are preserved. The implementation supports `-n`, `-e`,
/// `-E`, standalone `--help`, standalone `--version`, and the
/// `POSIXLY_CORRECT` parsing mode tested by the repository.
pub fn echo<I, T>(args: I, writer: &mut impl Write) -> io::Result<()>
where
    I: IntoIterator<Item = T>,
    T: AsRef<[u8]>,
{
    let mut args = args.into_iter();
    let first = args.next();
    let second = args.next();

    let posixly_correct = env::var_os("POSIXLY_CORRECT").is_some();

    if let (Some(ref f), None) = (&first, &second) {
        if !posixly_correct && f.as_ref() == b"--help" {
            let help_text = "\
Usage: echo [SHORT-OPTION]... [STRING]...
  or:  echo LONG-OPTION

Echo the STRING(s) to standard output.

  -n             do not output the trailing newline
  -e             enable interpretation of backslash escapes
  -E             disable interpretation of backslash escapes (default)
      --help     display this help and exit
      --version  output version information and exit

If -e is in effect, the following sequences are recognized:

  \\\\      backslash
  \\a      alert (BEL)
  \\b      backspace
  \\c      produce no further output
  \\e      escape
  \\f      form feed
  \\n      new line
  \\r      carriage return
  \\t      horizontal tab
  \\v      vertical tab
  \\0NNN   byte with octal value NNN (1 to 3 digits)
  \\xHH    byte with hexadecimal value HH (1 to 2 digits)\n";
            return write!(writer, "{}", help_text);
        }
        if !posixly_correct && f.as_ref() == b"--version" {
            return writeln!(writer, "echo (spencer's version) 0.0.1");
        }
    }

    let mut append_newline = true;
    let mut enable_escapes = posixly_correct;
    let allow_options = !posixly_correct;
    let mut parsed_posix_n = false;

    let mut all_args = first.into_iter().chain(second).chain(args).peekable();

    while let Some(f) = all_args.peek() {
        let bytes = f.as_ref();

        if allow_options {
            if bytes.starts_with(b"-") && bytes.len() > 1 {
                let mut valid_flags = true;
                for &b in &bytes[1..] {
                    if b != b'n' && b != b'e' && b != b'E' {
                        valid_flags = false;
                        break;
                    }
                }

                if valid_flags {
                    for &b in &bytes[1..] {
                        match b {
                            b'n' => append_newline = false,
                            b'e' => enable_escapes = true,
                            b'E' => enable_escapes = false,
                            _ => unreachable!(),
                        }
                    }
                    all_args.next();
                    continue;
                }
            }
        } else if bytes == b"-n"
            || (parsed_posix_n
                && bytes.starts_with(b"-")
                && bytes.len() > 1
                && bytes[1..]
                    .iter()
                    .all(|&b| b == b'n' || b == b'e' || b == b'E'))
        {
            for &b in &bytes[1..] {
                if b == b'n' {
                    append_newline = false;
                }
            }
            parsed_posix_n = true;
            all_args.next();
            continue;
        }

        break;
    }

    for (i, arg) in all_args.enumerate() {
        if i > 0 {
            writer.write_all(b" ")?;
        }
        if !write_arg(arg.as_ref(), writer, enable_escapes)? {
            return Ok(());
        }
    }

    if append_newline {
        writer.write_all(b"\n")?;
    }

    Ok(())
}
