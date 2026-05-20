//! GNU-style `echo`.
//!
//! This module writes operands to a byte-oriented writer, preserving raw Unix
//! argument bytes and matching GNU `echo` option and escape handling for the
//! supported behavior documented in this repository.
//! 
//! [echo.rs](file:///Users/spencergreene/github/rust-unix-tools/src/tools/echo.rs)

use std::env;
use std::io::{self, Write};

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_POSIXLY_CORRECT: std::cell::Cell<Option<bool>> = std::cell::Cell::new(None);
}

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
                _ => b - b'a' + 10,
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

    let posixly_correct = {
        #[cfg(test)]
        {
            TEST_POSIXLY_CORRECT.with(|cell| cell.get()).unwrap_or_else(|| {
                env::var_os("POSIXLY_CORRECT").is_some()
            })
        }
        #[cfg(not(test))]
        {
            env::var_os("POSIXLY_CORRECT").is_some()
        }
    };

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
                            _ => enable_escapes = false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Write};
    use std::env;

    // Helper writer that can fail on demand
    struct MockWriter {
        pub data: Vec<u8>,
        pub fail_after: Option<usize>,
    }

    impl MockWriter {
        fn new() -> Self {
            Self { data: Vec::new(), fail_after: None }
        }

        fn new_failing(after_bytes: usize) -> Self {
            Self { data: Vec::new(), fail_after: Some(after_bytes) }
        }
    }

    impl Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if let Some(limit) = self.fail_after {
                if self.data.len() + buf.len() > limit {
                    let allowed = limit.saturating_sub(self.data.len());
                    if allowed > 0 {
                        self.data.extend_from_slice(&buf[..allowed]);
                    }
                    return Err(io::Error::new(io::ErrorKind::Other, "mock write error"));
                }
            }
            self.data.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_echo_basic() {
        let mut w = MockWriter::new();
        echo(vec!["hello", "world"], &mut w).unwrap();
        assert_eq!(w.data, b"hello world\n");
    }

    #[test]
    fn test_echo_empty() {
        let mut w = MockWriter::new();
        echo(Vec::<String>::new(), &mut w).unwrap();
        assert_eq!(w.data, b"\n");
    }

    #[test]
    fn test_echo_options() {
        // -n (no newline)
        let mut w = MockWriter::new();
        echo(vec!["-n", "hello", "world"], &mut w).unwrap();
        assert_eq!(w.data, b"hello world");

        // -e (enable escapes) and -E (disable escapes)
        let mut w = MockWriter::new();
        echo(vec!["-e", "a\\nb"], &mut w).unwrap();
        assert_eq!(w.data, b"a\nb\n");

        let mut w = MockWriter::new();
        echo(vec!["-E", "a\\nb"], &mut w).unwrap();
        assert_eq!(w.data, b"a\\nb\n");

        // combinations: -ne, -eE, etc.
        let mut w = MockWriter::new();
        echo(vec!["-ne", "a\\nb"], &mut w).unwrap();
        assert_eq!(w.data, b"a\nb");

        let mut w = MockWriter::new();
        echo(vec!["-eE", "a\\nb"], &mut w).unwrap();
        assert_eq!(w.data, b"a\\nb\n");

        // -n with -E
        let mut w = MockWriter::new();
        echo(vec!["-nE", "a\\nb"], &mut w).unwrap();
        assert_eq!(w.data, b"a\\nb");
    }

    #[test]
    fn test_echo_escape_sequences() {
        let test_cases = vec![
            ("hello\\aworld", b"hello\x07world\n".as_slice()),
            ("hello\\bworld", b"hello\x08world\n"),
            ("hello\\cworld", b"hello"), // early exit
            ("hello\\eworld", b"hello\x1Bworld\n"),
            ("hello\\fworld", b"hello\x0Cworld\n"),
            ("hello\\nworld", b"hello\nworld\n"),
            ("hello\\rworld", b"hello\rworld\n"),
            ("hello\\tworld", b"hello\tworld\n"),
            ("hello\\vworld", b"hello\x0Bworld\n"),
            ("hello\\\\world", b"hello\\world\n"),
            ("hello\\", b"hello\\\n"), // backslash at the end of argument
            ("hello\\zworld", b"hello\\zworld\n"), // invalid escape sequence
        ];

        for (input, expected) in test_cases {
            let mut w = MockWriter::new();
            echo(vec!["-e", input], &mut w).unwrap();
            assert_eq!(w.data, expected, "Failed for input: {}", input);
        }
    }

    #[test]
    fn test_echo_octal_escapes() {
        // \0123 -> octal 123 (83 = 'S')
        let mut w = MockWriter::new();
        echo(vec!["-e", "\\0123"], &mut w).unwrap();
        assert_eq!(w.data, b"S\n");

        // \123 -> octal 123 (83 = 'S')
        let mut w = MockWriter::new();
        echo(vec!["-e", "\\123"], &mut w).unwrap();
        assert_eq!(w.data, b"S\n");

        // short octal: \0 -> byte 0, \1 -> byte 1, \12 -> octal 12 (10 = '\n')
        let mut w = MockWriter::new();
        echo(vec!["-e", "\\0"], &mut w).unwrap();
        assert_eq!(w.data, b"\0\n");

        let mut w = MockWriter::new();
        echo(vec!["-e", "\\12"], &mut w).unwrap();
        assert_eq!(w.data, b"\n\n");

        // invalid octal: \08 (8 is not octal digit, so it writes \0 and then 8)
        let mut w = MockWriter::new();
        echo(vec!["-e", "\\08"], &mut w).unwrap();
        assert_eq!(w.data, b"\08\n");

        // large octal: \777 (511 % 256 = 255)
        let mut w = MockWriter::new();
        echo(vec!["-e", "\\777"], &mut w).unwrap();
        assert_eq!(w.data, &[255, b'\n']);
    }

    #[test]
    fn test_echo_hex_escapes() {
        // \x41 -> 'A'
        let mut w = MockWriter::new();
        echo(vec!["-e", "\\x41"], &mut w).unwrap();
        assert_eq!(w.data, b"A\n");

        // \x412 -> 'A' followed by '2'
        let mut w = MockWriter::new();
        echo(vec!["-e", "\\x412"], &mut w).unwrap();
        assert_eq!(w.data, b"A2\n");

        // \x -> empty hex, should write \x
        let mut w = MockWriter::new();
        echo(vec!["-e", "\\x"], &mut w).unwrap();
        assert_eq!(w.data, b"\\x\n");

        // \xZ -> invalid hex, should write \x followed by Z
        let mut w = MockWriter::new();
        echo(vec!["-e", "\\xZ"], &mut w).unwrap();
        assert_eq!(w.data, b"\\xZ\n");

        // lowercase hex digits
        let mut w = MockWriter::new();
        echo(vec!["-e", "\\x3a"], &mut w).unwrap();
        assert_eq!(w.data, b":\n");
    }

    #[test]
    fn test_echo_help_version() {
        // --help alone
        let mut w = MockWriter::new();
        echo(vec!["--help"], &mut w).unwrap();
        assert!(std::str::from_utf8(&w.data).unwrap().contains("Usage: echo"));

        // --version alone
        let mut w = MockWriter::new();
        echo(vec!["--version"], &mut w).unwrap();
        assert!(std::str::from_utf8(&w.data).unwrap().contains("echo (spencer's version)"));

        // --help with other args (not standalone)
        let mut w = MockWriter::new();
        echo(vec!["--help", "foo"], &mut w).unwrap();
        assert_eq!(w.data, b"--help foo\n");
    }

    #[test]
    fn test_echo_posixly_correct() {
        TEST_POSIXLY_CORRECT.with(|cell| cell.set(Some(true)));

        // In POSIX mode:
        // 1. --help and --version are treated as normal operands
        let mut w = MockWriter::new();
        echo(vec!["--help"], &mut w).unwrap();
        assert_eq!(w.data, b"--help\n");

        let mut w = MockWriter::new();
        echo(vec!["--version"], &mut w).unwrap();
        assert_eq!(w.data, b"--version\n");

        // 2. Default escape interpretation is active
        let mut w = MockWriter::new();
        echo(vec!["hello\\nworld"], &mut w).unwrap();
        assert_eq!(w.data, b"hello\nworld\n");

        // 3. Option parsing behavior: -n is allowed at the start
        let mut w = MockWriter::new();
        echo(vec!["-n", "hello"], &mut w).unwrap();
        assert_eq!(w.data, b"hello");

        // 4. -e is not parsed as an option, but as operand (so it gets printed)
        let mut w = MockWriter::new();
        echo(vec!["-e", "hello"], &mut w).unwrap();
        assert_eq!(w.data, b"-e hello\n");

        // 5. -n followed by -ne or similar: first -n sets parsed_posix_n = true,
        // then subsequent -ne is parsed as option
        let mut w = MockWriter::new();
        echo(vec!["-n", "-ne", "hello"], &mut w).unwrap();
        assert_eq!(w.data, b"hello");

        // 6. -n followed by an invalid option like -nx: treated as operand
        let mut w = MockWriter::new();
        echo(vec!["-n", "-nx", "hello"], &mut w).unwrap();
        assert_eq!(w.data, b"-nx hello");

        TEST_POSIXLY_CORRECT.with(|cell| cell.set(None));
    }

    #[test]
    fn test_echo_write_errors() {
        // Fail on first write
        let mut w = MockWriter::new_failing(0);
        let res = echo(vec!["hello"], &mut w);
        assert!(res.is_err());

        // Fail during first operand write
        let mut w = MockWriter::new_failing(2);
        let res = echo(vec!["hello"], &mut w);
        assert!(res.is_err());

        // Fail on space separator
        let mut w = MockWriter::new_failing(6);
        let res = echo(vec!["hello", "world"], &mut w);
        assert!(res.is_err());

        // Fail on newline
        let mut w = MockWriter::new_failing(11);
        let res = echo(vec!["hello", "world"], &mut w);
        assert!(res.is_err());
    }
}
