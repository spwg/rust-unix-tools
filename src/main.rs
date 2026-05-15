//! A Rust implementation of the POSIX `echo` utility.
//!
//! Writes its arguments to standard output, separated by spaces, followed
//! by a newline character.
//!
//! # Supported options
//!
//! - `-n` — Do not print the trailing newline character.
//! - `\c` — When `\c` appears at the end of the last operand, the trailing
//!   newline is suppressed and the `\c` itself is removed from output.
//!   This is the iBCS2-compatible way to suppress the newline.
//!
//! # Exit status
//!
//! Exits `0` on success, `> 0` if an error occurs.

use std::env;
use std::io::{self, Write};
use std::os::unix::ffi::OsStringExt;
use std::process;

/// Parses up to 3 octal digits from the given byte slice, starting at `index`.
/// Returns the parsed byte value (which may wrap if over 255) and the number of characters consumed.
fn parse_octal(bytes: &[u8], mut index: usize) -> (u8, usize) {
    let start = index;
    let mut val: u16 = 0;

    // Consume up to 3 valid octal digits ('0'..='7')
    while index - start < 3 && index < bytes.len() && bytes[index] >= b'0' && bytes[index] <= b'7' {
        // Shift left by 3 bits (multiply by 8) and add the parsed digit
        val = (val << 3) | ((bytes[index] - b'0') as u16);
        index += 1;
    }

    // If the value exceeds 255 (e.g. \0777), it wraps around to a u8
    ((val % 256) as u8, index - start)
}

/// Parses up to 2 hexadecimal digits from the given byte slice, starting at `index`.
/// Returns the parsed byte value and the number of characters consumed.
fn parse_hex(bytes: &[u8], mut index: usize) -> (u8, usize) {
    let start = index;
    let mut val: u8 = 0;

    // Consume up to 2 valid hexadecimal digits
    while index - start < 2 && index < bytes.len() {
        let b = bytes[index];
        if b.is_ascii_hexdigit() {
            let digit = match b {
                b'0'..=b'9' => b - b'0',
                b'A'..=b'F' => b - b'A' + 10,
                b'a'..=b'f' => b - b'a' + 10,
                _ => unreachable!(),
            };
            // Shift left by 4 bits (multiply by 16) and add the parsed digit
            val = (val << 4) | digit;
            index += 1;
        } else {
            break;
        }
    }

    (val, index - start)
}

/// Helper to parse escape sequences if `enable_escapes` is true.
/// Returns `Ok(false)` if `\c` is encountered, meaning we must stop processing entirely.
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
                    let (val, consumed) = parse_octal(bytes, i + 2); // Skip '\' and '0'
                    writer.write_all(&[val])?;
                    i += 2 + consumed;
                }
                b'x' => {
                    let (val, consumed) = parse_hex(bytes, i + 2); // Skip '\' and 'x'
                    if consumed == 0 {
                        writer.write_all(b"\\x")?; // Literal \x if no valid hex digits
                    } else {
                        writer.write_all(&[val])?;
                    }
                    i += 2 + consumed;
                }
                b'1'..=b'7' => {
                    let (val, consumed) = parse_octal(bytes, i + 1); // Skip '\', point at first digit
                    writer.write_all(&[val])?;
                    i += 1 + consumed;
                }
                _ => {
                    // Unrecognized escape: print literal \ and the following char
                    writer.write_all(&[b'\\', bytes[i + 1]])?;
                    i += 2;
                }
            }
        } else if bytes[i] == b'\\' {
            // Trailing backslash at the end of the argument
            writer.write_all(b"\\")?;
            i += 1;
        } else {
            // Normal character
            writer.write_all(&[bytes[i]])?;
            i += 1;
        }
    }
    Ok(true)
}

/// Writes `args` to `writer`, mimicking POSIX `echo` behavior.
///
/// **Zero-allocation byte-streaming**: processes arguments directly as bytes
/// without intermediate `String` or `Vec` allocations.
pub fn echo<I, T>(args: I, writer: &mut impl Write) -> io::Result<()>
where
    I: IntoIterator<Item = T>,
    T: AsRef<[u8]>,
{
    let mut args = args.into_iter();
    let first = args.next();
    let second = args.next();

    let posixly_correct = env::var_os("POSIXLY_CORRECT").is_some();

    // Mimic GNU echo behavior: print help if `--help` is the *only* argument.
    // POSIXLY_CORRECT disables this long-option special case.
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

    // GNU Flag Parsing
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
                && bytes[1..].iter().all(|&b| b == b'n' || b == b'e' || b == b'E'))
        {
            // Under POSIXLY_CORRECT, GNU only starts option parsing if the
            // first argument is exactly "-n"; valid option groups may follow.
            for &b in &bytes[1..] {
                if b == b'n' {
                    append_newline = false;
                }
            }
            parsed_posix_n = true;
            all_args.next();
            continue;
        }

        break; // Not a valid flag block, stop parsing flags
    }

    for (i, arg) in all_args.enumerate() {
        if i > 0 {
            writer.write_all(b" ")?;
        }
        if !write_arg(arg.as_ref(), writer, enable_escapes)? {
            return Ok(()); // \c was encountered, halt entirely
        }
    }

    if append_newline {
        writer.write_all(b"\n")?;
    }

    Ok(())
}

fn main() {
    // Grab bytes directly from the OS, avoiding String allocation
    let args = env::args_os().skip(1).map(|os_str| os_str.into_vec());

    let stdout = io::stdout();
    let mut handle = stdout.lock();

    if let Err(e) = echo(args, &mut handle) {
        eprintln!("echo: {e}");
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: call echo() with byte slices and return the output as a String.
    fn run(args: &[&[u8]]) -> String {
        String::from_utf8(run_bytes(args)).unwrap()
    }

    fn run_bytes(args: &[&[u8]]) -> Vec<u8> {
        let mut buf = Vec::new();
        echo(args.iter().copied(), &mut buf).unwrap();
        buf
    }

    // ─── Basic output: "followed by a newline ('\n')" ────────────────────

    #[test]
    fn no_args_prints_newline() {
        assert_eq!(run(&[]), "\n");
    }

    #[test]
    fn single_operand() {
        assert_eq!(run(&[b"hello"]), "hello\n");
    }

    // ─── Operand separation: "separated by single blank (' ') characters"

    #[test]
    fn multiple_operands_separated_by_space() {
        assert_eq!(run(&[b"hello", b"world"]), "hello world\n");
    }

    #[test]
    fn many_operands() {
        assert_eq!(run(&[b"a", b"b", b"c", b"d"]), "a b c d\n");
    }

    #[test]
    fn whitespace_within_operand_is_preserved() {
        assert_eq!(run(&[b"hello  world"]), "hello  world\n");
    }

    // ─── -n flag: "Do not print the trailing newline character." ─────────

    #[test]
    fn flag_n_suppresses_newline() {
        assert_eq!(run(&[b"-n", b"hello"]), "hello");
    }

    #[test]
    fn flag_n_with_multiple_operands() {
        assert_eq!(run(&[b"-n", b"hello", b"world"]), "hello world");
    }

    #[test]
    fn flag_n_with_no_operands() {
        assert_eq!(run(&[b"-n"]), "");
    }

    #[test]
    fn flag_n_only_recognized_as_first_arg() {
        assert_eq!(run(&[b"hello", b"-n"]), "hello -n\n");
    }

    // ─── \c escape: "truncates output immediately" ───────────

    #[test]
    fn backslash_c_suppresses_newline() {
        assert_eq!(run(&[b"-e", b"hello\\c"]), "hello");
    }

    #[test]
    fn backslash_c_with_multiple_operands() {
        assert_eq!(run(&[b"-e", b"hello", b"world\\c"]), "hello world");
    }

    #[test]
    fn backslash_c_in_middle_truncates_output() {
        assert_eq!(run(&[b"-e", b"hel\\clo"]), "hel");
    }

    #[test]
    fn backslash_c_combined_with_flag_n() {
        assert_eq!(run(&[b"-n", b"-e", b"hello\\c"]), "hello");
    }

    // ─── Escape Sequences ───────────────────────────────────────────────

    #[test]
    fn escape_alert() { assert_eq!(run(&[b"-e", b"\\a"]), "\x07\n"); }
    #[test]
    fn escape_backspace() { assert_eq!(run(&[b"-e", b"\\b"]), "\x08\n"); }
    #[test]
    fn escape_escape() { assert_eq!(run(&[b"-e", b"\\e"]), "\x1B\n"); }
    #[test]
    fn escape_form_feed() { assert_eq!(run(&[b"-e", b"\\f"]), "\x0C\n"); }
    #[test]
    fn escape_newline() { assert_eq!(run(&[b"-e", b"\\n"]), "\n\n"); }
    #[test]
    fn escape_carriage_return() { assert_eq!(run(&[b"-e", b"\\r"]), "\r\n"); }
    #[test]
    fn escape_horizontal_tab() { assert_eq!(run(&[b"-e", b"\\t"]), "\t\n"); }
    #[test]
    fn escape_vertical_tab() { assert_eq!(run(&[b"-e", b"\\v"]), "\x0B\n"); }
    #[test]
    fn escape_backslash() { assert_eq!(run(&[b"-e", b"\\\\"]), "\\\n"); }
    #[test]
    fn escape_unrecognized() { assert_eq!(run(&[b"-e", b"\\z"]), "\\z\n"); }
    #[test]
    fn escape_trailing_backslash() { assert_eq!(run(&[b"-e", b"hello\\"]), "hello\\\n"); }

    #[test]
    fn escape_octal() {
        assert_eq!(run(&[b"-e", b"\\0101"]), "A\n"); // 101 octal = 65 = A
        assert_eq!(run(&[b"-e", b"\\07"]), "\x07\n");
        assert_eq!(run_bytes(&[b"-e", b"\\0777"]), b"\xFF\n"); // wraps around
    }

    #[test]
    fn escape_hex() {
        assert_eq!(run(&[b"-e", b"\\x41"]), "A\n"); // 41 hex = 65 = A
        assert_eq!(run(&[b"-e", b"\\x4F"]), "O\n");
        assert_eq!(run(&[b"-e", b"\\x4f"]), "O\n");
        assert_eq!(run(&[b"-e", b"\\x"]), "\\x\n"); // no digits
    }

    #[test]
    fn escape_old_octal() {
        assert_eq!(run(&[b"-e", b"\\101"]), "A\n");
        assert_eq!(run(&[b"-e", b"\\7"]), "\x07\n");
    }

    // ─── Edge cases ─────────────────────────────────────────────────────

    #[test]
    fn empty_string_operand() {
        assert_eq!(run(&[b""]), "\n");
    }

    #[test]
    fn two_empty_string_operands() {
        assert_eq!(run(&[b"", b""]), " \n");
    }

    #[test]
    fn special_characters() {
        assert_eq!(run(&[b"hello!", b"@#$%", b"^&*()"]), "hello! @#$% ^&*()\n");
    }

    #[test]
    fn disable_escapes_flag() {
        assert_eq!(run(&[b"-E", b"hello\\n"]), "hello\\n\n");
        assert_eq!(run(&[b"-e", b"-E", b"hello\\n"]), "hello\\n\n");
    }

    // ─── Help & Version flag ────────────────────────────────────────────

    #[test]
    fn help_flag_when_alone_prints_help() {
        let output = run(&[b"--help"]);
        assert!(output.starts_with("Usage: echo"));
        assert!(output.contains("-n             do not output the trailing newline"));
    }

    #[test]
    fn version_flag_when_alone_prints_version() {
        assert_eq!(run(&[b"--version"]), "echo (spencer's version) 0.0.1\n");
    }

    #[test]
    fn help_flag_with_other_args_is_literal() {
        assert_eq!(run(&[b"--help", b"foo"]), "--help foo\n");
        assert_eq!(run(&[b"foo", b"--help"]), "foo --help\n");
    }
}
