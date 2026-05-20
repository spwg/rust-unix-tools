//! Reusable GNU-compatible option parsing library.
//!
//! This module parses command-line arguments (represented as `OsString`)
//! according to standard GNU getopt semantics. It supports:
//! - Short options (e.g. `-a`, `-l`) and short option grouping (e.g. `-al`).
//! - Short options taking required or optional arguments, either attached
//!   (e.g. `-bval`) or as the next argument (e.g. `-b val`).
//! - Long options (e.g. `--all`, `--sort=size`) with required or optional arguments.
//! - Option terminator (`--`).
//! - GNU-style argument permutation (intermixed options and operands)
//!   which can be disabled via a `posixly_correct` flag.
//! 
//! [getopt.rs](file:///Users/spencergreene/github/rust-unix-tools/src/getopt.rs)

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;

/// Describes whether an option expects an argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HasArg {
    /// The option does not take an argument.
    No,
    /// The option requires an argument.
    Yes,
    /// The option takes an optional argument (long options only with `=`, short options only if attached).
    Optional,
}

/// Description of a supported option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptSpec {
    /// The single-character short name (e.g. `Some('a')`).
    pub short: Option<char>,
    /// The long name without dashes (e.g. `Some("all")`).
    pub long: Option<&'static str>,
    /// Whether the option takes an argument.
    pub has_arg: HasArg,
}

/// A successfully parsed argument.
#[derive(Debug, PartialEq, Eq)]
pub enum ParsedArg<'a> {
    /// An option matching one of the specifications.
    Option {
        /// The short char of the matched option, if defined.
        short: Option<char>,
        /// The long name of the matched option, if defined.
        long: Option<&'static str>,
        /// The associated option value, if any.
        value: Option<&'a OsStr>,
    },
    /// A positional operand (non-option).
    Operand(&'a OsStr),
}

/// Parses the command-line arguments using the provided specifications.
///
/// Returns a vector of parsed arguments in the order they were processed,
/// or an error message if option parsing failed.
pub fn parse<'a>(
    args: &'a [OsString],
    specs: &[OptSpec],
    posixly_correct: bool,
) -> Result<Vec<ParsedArg<'a>>, String> {
    let mut parsed = Vec::new();
    let mut operands = Vec::new();
    let mut i = 0;
    let mut options_allowed = true;

    while i < args.len() {
        let arg = &args[i];
        let bytes = arg.as_bytes();

        if !options_allowed || bytes == b"-" || !bytes.starts_with(b"-") {
            if posixly_correct {
                options_allowed = false;
            }
            operands.push(arg.as_os_str());
            i += 1;
            continue;
        }

        if bytes == b"--" {
            options_allowed = false;
            i += 1;
            continue;
        }

        if bytes.starts_with(b"--") {
            // Parse long option
            let name_with_dashes = &bytes[2..];
            let (name_bytes, inline_value) = if let Some(idx) = name_with_dashes.iter().position(|&b| b == b'=') {
                (&name_with_dashes[..idx], Some(&name_with_dashes[idx + 1..]))
            } else {
                (name_with_dashes, None)
            };

            let name_str = std::str::from_utf8(name_bytes)
                .map_err(|_| format!("unrecognized option '--{}'", String::from_utf8_lossy(name_bytes)))?;

            let spec = specs.iter().find(|s| s.long == Some(name_str));
            if let Some(spec) = spec {
                match spec.has_arg {
                    HasArg::No => {
                        if inline_value.is_some() {
                            return Err(format!("option '--{}' doesn't allow an argument", name_str));
                        }
                        parsed.push(ParsedArg::Option {
                            short: spec.short,
                            long: spec.long,
                            value: None,
                        });
                    }
                    HasArg::Yes => {
                        let val = if let Some(val_bytes) = inline_value {
                            Some(OsStr::from_bytes(val_bytes))
                        } else {
                            i += 1;
                            if i < args.len() {
                                Some(args[i].as_os_str())
                            } else {
                                return Err(format!("option '--{}' requires an argument", name_str));
                            }
                        };
                        parsed.push(ParsedArg::Option {
                            short: spec.short,
                            long: spec.long,
                            value: val,
                        });
                    }
                    HasArg::Optional => {
                        let val = inline_value.map(OsStr::from_bytes);
                        parsed.push(ParsedArg::Option {
                            short: spec.short,
                            long: spec.long,
                            value: val,
                        });
                    }
                }
            } else {
                return Err(format!("unrecognized option '--{}'", name_str));
            }
            i += 1;
        } else {
            // Parse short option group (e.g. -abc)
            let mut char_idx = 1;
            while char_idx < bytes.len() {
                let b = bytes[char_idx];
                let spec = specs.iter().find(|s| s.short.map(|s| s as u8) == Some(b));
                if let Some(spec) = spec {
                    let short_char = b as char;
                    match spec.has_arg {
                        HasArg::No => {
                            parsed.push(ParsedArg::Option {
                                short: Some(short_char),
                                long: spec.long,
                                value: None,
                            });
                            char_idx += 1;
                        }
                        HasArg::Yes => {
                            char_idx += 1;
                            let rest = &bytes[char_idx..];
                            let val = if !rest.is_empty() {
                                Some(OsStr::from_bytes(rest))
                            } else {
                                i += 1;
                                if i < args.len() {
                                    Some(args[i].as_os_str())
                                } else {
                                    return Err(format!("option requires an argument -- '{}'", short_char));
                                }
                            };
                            parsed.push(ParsedArg::Option {
                                short: Some(short_char),
                                long: spec.long,
                                value: val,
                            });
                            break;
                        }
                        HasArg::Optional => {
                            char_idx += 1;
                            let rest = &bytes[char_idx..];
                            let val = if !rest.is_empty() {
                                Some(OsStr::from_bytes(rest))
                            } else {
                                None
                            };
                            parsed.push(ParsedArg::Option {
                                short: Some(short_char),
                                long: spec.long,
                                value: val,
                            });
                            break;
                        }
                    }
                } else {
                    return Err(format!("invalid option -- '{}'", b as char));
                }
            }
            i += 1;
        }
    }

    // Append operands
    for op in operands {
        parsed.push(ParsedArg::Operand(op));
    }

    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_getopt_basic() {
        let specs = vec![
            OptSpec { short: Some('a'), long: Some("all"), has_arg: HasArg::No },
            OptSpec { short: Some('b'), long: Some("block-size"), has_arg: HasArg::Yes },
            OptSpec { short: Some('c'), long: Some("classify"), has_arg: HasArg::Optional },
        ];

        // Mixed options and operands with permutation
        let args: Vec<OsString> = vec![
            "-a".into(),
            "operand1".into(),
            "-b".into(),
            "1024".into(),
            "--classify=always".into(),
            "operand2".into(),
        ];

        let result = parse(&args, &specs, false).unwrap();
        assert_eq!(
            result,
            vec![
                ParsedArg::Option { short: Some('a'), long: Some("all"), value: None },
                ParsedArg::Option { short: Some('b'), long: Some("block-size"), value: Some(OsStr::new("1024")) },
                ParsedArg::Option { short: Some('c'), long: Some("classify"), value: Some(OsStr::new("always")) },
                ParsedArg::Operand(OsStr::new("operand1")),
                ParsedArg::Operand(OsStr::new("operand2")),
            ]
        );
    }

    #[test]
    fn test_getopt_posix() {
        let specs = vec![
            OptSpec { short: Some('a'), long: Some("all"), has_arg: HasArg::No },
            OptSpec { short: Some('b'), long: Some("block-size"), has_arg: HasArg::Yes },
        ];

        let args: Vec<OsString> = vec![
            "-a".into(),
            "operand1".into(),
            "-b".into(),
            "1024".into(),
        ];

        // POSIX stops at the first operand
        let result = parse(&args, &specs, true).unwrap();
        assert_eq!(
            result,
            vec![
                ParsedArg::Option { short: Some('a'), long: Some("all"), value: None },
                ParsedArg::Operand(OsStr::new("operand1")),
                ParsedArg::Operand(OsStr::new("-b")),
                ParsedArg::Operand(OsStr::new("1024")),
            ]
        );
    }

    #[test]
    fn test_getopt_attached_and_extra() {
        let specs = vec![
            OptSpec { short: Some('a'), long: Some("all"), has_arg: HasArg::No },
            OptSpec { short: Some('b'), long: Some("block-size"), has_arg: HasArg::Yes },
            OptSpec { short: Some('c'), long: Some("classify"), has_arg: HasArg::Optional },
        ];

        // 1. Attached required option arguments (-b1024)
        let args1 = vec!["-b1024".into()];
        let res1 = parse(&args1, &specs, false).unwrap();
        assert_eq!(
            res1,
            vec![ParsedArg::Option { short: Some('b'), long: Some("block-size"), value: Some(OsStr::new("1024")) }]
        );

        // 2. Attached optional option arguments (-cval)
        let args2 = vec!["-cval".into()];
        let res2 = parse(&args2, &specs, false).unwrap();
        assert_eq!(
            res2,
            vec![ParsedArg::Option { short: Some('c'), long: Some("classify"), value: Some(OsStr::new("val")) }]
        );

        // 3. Option terminator --
        let args3 = vec!["-a".into(), "--".into(), "-b".into(), "1024".into()];
        let res3 = parse(&args3, &specs, false).unwrap();
        assert_eq!(
            res3,
            vec![
                ParsedArg::Option { short: Some('a'), long: Some("all"), value: None },
                ParsedArg::Operand(OsStr::new("-b")),
                ParsedArg::Operand(OsStr::new("1024")),
            ]
        );

        // 4. Operand "-"
        let args4 = vec!["-".into(), "-a".into()];
        let res4 = parse(&args4, &specs, false).unwrap();
        assert_eq!(
            res4,
            vec![
                ParsedArg::Option { short: Some('a'), long: Some("all"), value: None },
                ParsedArg::Operand(OsStr::new("-")),
            ]
        );

        // 5. Long option optional argument (no value provided)
        let args5 = vec!["--classify".into()];
        let res5 = parse(&args5, &specs, false).unwrap();
        assert_eq!(
            res5,
            vec![ParsedArg::Option { short: Some('c'), long: Some("classify"), value: None }]
        );

        // 6. Long option required argument (inline value provided via =)
        let args6 = vec!["--block-size=512".into()];
        let res6 = parse(&args6, &specs, false).unwrap();
        assert_eq!(
            res6,
            vec![ParsedArg::Option { short: Some('b'), long: Some("block-size"), value: Some(OsStr::new("512")) }]
        );

        // 7. Grouping of options
        let args7 = vec!["-acval".into()];
        let res7 = parse(&args7, &specs, false).unwrap();
        assert_eq!(
            res7,
            vec![
                ParsedArg::Option { short: Some('a'), long: Some("all"), value: None },
                ParsedArg::Option { short: Some('c'), long: Some("classify"), value: Some(OsStr::new("val")) },
            ]
        );

        let args8 = vec!["-ab512".into()];
        let res8 = parse(&args8, &specs, false).unwrap();
        assert_eq!(
            res8,
            vec![
                ParsedArg::Option { short: Some('a'), long: Some("all"), value: None },
                ParsedArg::Option { short: Some('b'), long: Some("block-size"), value: Some(OsStr::new("512")) },
            ]
        );
    }

    #[test]
    fn test_invalid_utf8_long_option() {
        use std::os::unix::ffi::OsStringExt;
        let specs = vec![
            OptSpec { short: Some('a'), long: Some("all"), has_arg: HasArg::No },
        ];
        // Create an invalid UTF-8 long option `--\xff`
        let invalid_arg = OsString::from_vec(vec![b'-', b'-', 0xff]);
        let args = vec![invalid_arg];
        let res = parse(&args, &specs, false);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("unrecognized option"));
    }

    #[test]
    fn test_getopt_errors() {
        let specs = vec![
            OptSpec { short: Some('a'), long: Some("all"), has_arg: HasArg::No },
            OptSpec { short: Some('b'), long: Some("block-size"), has_arg: HasArg::Yes },
        ];

        // Missing argument for short option
        let args1: Vec<OsString> = vec!["-b".into()];
        assert_eq!(
            parse(&args1, &specs, false).unwrap_err(),
            "option requires an argument -- 'b'"
        );

        // Missing argument for long option
        let args2: Vec<OsString> = vec!["--block-size".into()];
        assert_eq!(
            parse(&args2, &specs, false).unwrap_err(),
            "option '--block-size' requires an argument"
        );

        // Unrecognized option
        let args3: Vec<OsString> = vec!["--foo".into()];
        assert_eq!(
            parse(&args3, &specs, false).unwrap_err(),
            "unrecognized option '--foo'"
        );

        // Invalid short option
        let args4: Vec<OsString> = vec!["-x".into()];
        assert_eq!(
            parse(&args4, &specs, false).unwrap_err(),
            "invalid option -- 'x'"
        );

        // Argument not allowed
        let args5: Vec<OsString> = vec!["--all=yes".into()];
        assert_eq!(
            parse(&args5, &specs, false).unwrap_err(),
            "option '--all' doesn't allow an argument"
        );
    }
}
