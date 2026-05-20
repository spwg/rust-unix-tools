//! GNU-style `grep`.
//!
//! This module implements the core logic of the `grep` command, supporting
//! option parsing, recursive directory traversal, and pattern matching using
//! the `regex` crate.

use regex::{Regex, RegexBuilder};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use crate::getopt::{self, HasArg, OptSpec, ParsedArg};

/// Options for the `grep` command, parsed from command-line arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GrepOptions {
    /// A list of pattern strings to match against the inputs.
    patterns: Vec<String>,
    /// Whether to perform case-insensitive matching (`-i`, `-y`, `--ignore-case`).
    ignore_case: bool,
    /// Whether to invert the match results, selecting non-matching lines (`-v`, `--invert-match`).
    invert_match: bool,
    /// Whether to match only whole words (`-w`, `--word-regexp`).
    word_regexp: bool,
    /// Whether to match only whole lines (`-x`, `--line-regexp`).
    line_regexp: bool,
    /// Whether to print only a count of matching lines per file (`-c`, `--count`).
    count: bool,
    /// Whether to print only names of files with no matching lines (`-L`, `--files-without-match`).
    files_without_match: bool,
    /// Whether to print only names of files with matching lines (`-l`, `--files-with-matches`).
    files_with_matches: bool,
    /// Limit the maximum number of matching lines to output or count per file (`-m`, `--max-count`).
    max_count: Option<usize>,
    /// Print only the matching parts of a line (`-o`, `--only-matching`).
    only_matching: bool,
    /// Suppress all normal output; exit immediately with status 0 if any match is found (`-q`, `--quiet`, `--silent`).
    quiet: bool,
    /// Suppress error messages about nonexistent or unreadable files (`-s`, `--no-messages`).
    no_messages: bool,
    /// Print the 0-based byte offset of each matching line / match (`-b`, `--byte-offset`).
    byte_offset: bool,
    /// Print the 1-based line number of each matching line (`-n`, `--line-number`).
    line_number: bool,
    /// Print the file name prefix for each match, even if only one file is searched (`-H`, `--with-filename`).
    with_filename: bool,
    /// Suppress file name prefixes on output (`-h`, `--no-filename`).
    no_filename: bool,
    /// Search directories recursively (`-r`, `--recursive`).
    recursive: bool,
    /// Search directories recursively, following all symbolic links (`-R`, `--dereference-recursive`).
    dereference_recursive: bool,
    /// Treat patterns as fixed strings instead of regular expressions (`-F`, `--fixed-strings`).
    fixed_strings: bool,
    /// Print a null byte (character 0) after file name prefixes instead of a colon (`-Z`, `--null`).
    null_separator: bool,
}

impl GrepOptions {
    /// Constructs a new `GrepOptions` instance with default values.
    fn new() -> Self {
        Self {
            patterns: Vec::new(),
            ignore_case: false,
            invert_match: false,
            word_regexp: false,
            line_regexp: false,
            count: false,
            files_without_match: false,
            files_with_matches: false,
            max_count: None,
            only_matching: false,
            quiet: false,
            no_messages: false,
            byte_offset: false,
            line_number: false,
            with_filename: false,
            no_filename: false,
            recursive: false,
            dereference_recursive: false,
            fixed_strings: false,
            null_separator: false,
        }
    }
}

/// Result of parsing grep options.
#[derive(Debug, PartialEq, Eq)]
enum ParseResult {
    /// Option parsing succeeded, and we should proceed with the returned options and file list.
    Success {
        options: GrepOptions,
        files: Vec<OsString>,
    },
    /// An early exit is requested (e.g. `--help` or `--version`). The associated exit code is provided.
    EarlyExit(i32),
}

/// Parses command-line arguments into `GrepOptions` and a list of files to search.
fn parse_options(
    args: &[OsString],
    cwd: &Path,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<ParseResult, i32> {
    let specs = &[
        OptSpec { short: Some('e'), long: Some("regexp"), has_arg: HasArg::Yes },
        OptSpec { short: Some('f'), long: Some("file"), has_arg: HasArg::Yes },
        OptSpec { short: Some('i'), long: Some("ignore-case"), has_arg: HasArg::No },
        OptSpec { short: Some('y'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('v'), long: Some("invert-match"), has_arg: HasArg::No },
        OptSpec { short: Some('w'), long: Some("word-regexp"), has_arg: HasArg::No },
        OptSpec { short: Some('x'), long: Some("line-regexp"), has_arg: HasArg::No },
        OptSpec { short: Some('c'), long: Some("count"), has_arg: HasArg::No },
        OptSpec { short: Some('L'), long: Some("files-without-match"), has_arg: HasArg::No },
        OptSpec { short: Some('l'), long: Some("files-with-matches"), has_arg: HasArg::No },
        OptSpec { short: Some('m'), long: Some("max-count"), has_arg: HasArg::Yes },
        OptSpec { short: Some('o'), long: Some("only-matching"), has_arg: HasArg::No },
        OptSpec { short: Some('q'), long: Some("quiet"), has_arg: HasArg::No },
        OptSpec { short: None, long: Some("silent"), has_arg: HasArg::No },
        OptSpec { short: Some('s'), long: Some("no-messages"), has_arg: HasArg::No },
        OptSpec { short: Some('b'), long: Some("byte-offset"), has_arg: HasArg::No },
        OptSpec { short: Some('n'), long: Some("line-number"), has_arg: HasArg::No },
        OptSpec { short: Some('H'), long: Some("with-filename"), has_arg: HasArg::No },
        OptSpec { short: Some('h'), long: Some("no-filename"), has_arg: HasArg::No },
        OptSpec { short: Some('r'), long: Some("recursive"), has_arg: HasArg::No },
        OptSpec { short: Some('R'), long: Some("dereference-recursive"), has_arg: HasArg::No },
        OptSpec { short: Some('E'), long: Some("extended-regexp"), has_arg: HasArg::No },
        OptSpec { short: Some('F'), long: Some("fixed-strings"), has_arg: HasArg::No },
        OptSpec { short: Some('G'), long: Some("basic-regexp"), has_arg: HasArg::No },
        OptSpec { short: Some('Z'), long: Some("null"), has_arg: HasArg::No },
        OptSpec { short: None, long: Some("help"), has_arg: HasArg::No },
        OptSpec { short: Some('V'), long: Some("version"), has_arg: HasArg::No },
    ];

    let parsed_args = match getopt::parse(args, specs, false) {
        Ok(args) => args,
        Err(e) => {
            let _ = writeln!(stderr, "grep: {}", e);
            return Err(2);
        }
    };

    let mut options = GrepOptions::new();
    let mut files = Vec::new();
    let mut help_requested = false;
    let mut version_requested = false;

    for arg in parsed_args {
        match arg {
            ParsedArg::Option { short, long, value } => {
                match (short, long) {
                    (_, Some("help")) => {
                        help_requested = true;
                    }
                    (Some('V'), _) | (_, Some("version")) => {
                        version_requested = true;
                    }
                    (Some('e'), _) | (_, Some("regexp")) => {
                        if let Some(val) = value {
                            options.patterns.push(val.to_string_lossy().into_owned());
                        }
                    }
                    (Some('f'), _) | (_, Some("file")) => {
                        if let Some(val) = value {
                            let file_str = val.to_string_lossy();
                            if let Err(e) = read_patterns_from_file(&file_str, cwd, &mut options.patterns) {
                                if !options.no_messages {
                                    let _ = writeln!(stderr, "grep: {}: {}", file_str, e);
                                }
                                return Err(2);
                            }
                        }
                    }
                    (Some('i'), _) | (Some('y'), _) | (_, Some("ignore-case")) => {
                        options.ignore_case = true;
                    }
                    (Some('v'), _) | (_, Some("invert-match")) => {
                        options.invert_match = true;
                    }
                    (Some('w'), _) | (_, Some("word-regexp")) => {
                        options.word_regexp = true;
                    }
                    (Some('x'), _) | (_, Some("line-regexp")) => {
                        options.line_regexp = true;
                    }
                    (Some('c'), _) | (_, Some("count")) => {
                        options.count = true;
                    }
                    (Some('L'), _) | (_, Some("files-without-match")) => {
                        options.files_without_match = true;
                    }
                    (Some('l'), _) | (_, Some("files-with-matches")) => {
                        options.files_with_matches = true;
                    }
                    (Some('m'), _) | (_, Some("max-count")) => {
                        if let Some(val) = value {
                            let s = val.to_string_lossy();
                            if let Ok(n) = s.parse::<usize>() {
                                options.max_count = Some(n);
                            } else {
                                let _ = writeln!(stderr, "grep: invalid max-count");
                                return Err(2);
                            }
                        }
                    }
                    (Some('o'), _) | (_, Some("only-matching")) => {
                        options.only_matching = true;
                    }
                    (Some('q'), _) | (_, Some("quiet")) | (_, Some("silent")) => {
                        options.quiet = true;
                    }
                    (Some('s'), _) | (_, Some("no-messages")) => {
                        options.no_messages = true;
                    }
                    (Some('b'), _) | (_, Some("byte-offset")) => {
                        options.byte_offset = true;
                    }
                    (Some('n'), _) | (_, Some("line-number")) => {
                        options.line_number = true;
                    }
                    (Some('H'), _) | (_, Some("with-filename")) => {
                        options.with_filename = true;
                    }
                    (Some('h'), _) | (_, Some("no-filename")) => {
                        options.no_filename = true;
                    }
                    (Some('r'), _) | (_, Some("recursive")) => {
                        options.recursive = true;
                    }
                    (Some('R'), _) | (_, Some("dereference-recursive")) => {
                        options.dereference_recursive = true;
                    }
                    (Some('F'), _) | (_, Some("fixed-strings")) => {
                        options.fixed_strings = true;
                    }
                    (Some('Z'), _) | (_, Some("null")) => {
                        options.null_separator = true;
                    }
                    (Some('E'), _) | (_, Some("extended-regexp")) => {
                        // ignored
                    }
                    (Some('G'), _) | (_, Some("basic-regexp")) => {
                        // ignored
                    }
                    _ => unreachable!(),
                }
            }
            ParsedArg::Operand(op) => {
                files.push(op.to_os_string());
            }
        }
    }

    if help_requested {
        let _ = writeln!(
            stdout,
            "Usage: grep [OPTION]... PATTERNS [FILE]...\n\
             Search for PATTERNS in each FILE.\n\n\
             Regexp selection and interpretation:\n\
               -E, --extended-regexp     PATTERNS are extended regular expressions\n\
               -F, --fixed-strings       PATTERNS are strings\n\
               -G, --basic-regexp        PATTERNS are basic regular expressions\n\
               -e, --regexp=PATTERNS     use PATTERNS for matching\n\
               -f, --file=FILE           take PATTERNS from FILE\n\
               -i, --ignore-case         ignore case distinctions\n\
               -w, --word-regexp         match only whole words\n\
               -x, --line-regexp         match only whole lines\n\n\
             Miscellaneous:\n\
               -s, --no-messages         suppress error messages\n\
               -v, --invert-match        select non-matching lines\n\
               -V, --version             display version information\n\
                   --help                display this help text\n\n\
             Output control:\n\
               -m, --max-count=NUM       stop after NUM selected lines\n\
               -b, --byte-offset         print the byte offset with output lines\n\
               -n, --line-number         print line number with output lines\n\
               -H, --with-filename       print file name with output lines\n\
               -h, --no-filename         suppress file name prefixes\n\
               -q, --quiet, --silent     suppress all normal output\n\
               -l, --files-with-matches  print only names of FILEs with selected lines\n\
               -L, --files-without-match print only names of FILEs with no selected lines\n\
               -c, --count               print only a count of selected lines per FILE\n\
               -Z, --null                print 0 byte after FILE name\n\
               -r, --recursive           like --directories=recurse\n\
               -R, --dereference-recursive likewise, but follow all symlinks"
        );
        return Ok(ParseResult::EarlyExit(0));
    }

    if version_requested {
        let _ = writeln!(stdout, "grep (rust-unix-tools) 0.1.0");
        return Ok(ParseResult::EarlyExit(0));
    }

    // If no patterns specified via -e or -f, the first positional argument is the pattern
    if options.patterns.is_empty() {
        if files.is_empty() {
            let _ = writeln!(stderr, "Usage: grep [OPTION]... PATTERNS [FILE]...");
            return Err(2);
        }
        let pattern_arg = files.remove(0);
        options.patterns.push(pattern_arg.to_string_lossy().into_owned());
    }

    Ok(ParseResult::Success { options, files })
}

/// Helper to read pattern strings from a file, line-by-line.
fn read_patterns_from_file(
    file_path: &str,
    cwd: &Path,
    patterns: &mut Vec<String>,
) -> io::Result<()> {
    let path = cwd.join(file_path);
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line?;
        patterns.push(line);
    }
    Ok(())
}

/// Compiles a list of pattern strings (which may contain newlines) into regexes.
/// Returns a tuple of compiled regexes and the raw sub-patterns list.
fn compile_regexes(
    patterns: &[String],
    options: &GrepOptions,
) -> Result<(Vec<Regex>, Vec<String>), String> {
    let mut compiled = Vec::new();
    let mut final_patterns = Vec::new();

    for p in patterns {
        for sub in p.split('\n') {
            final_patterns.push(sub.to_string());
        }
    }

    for p in &final_patterns {
        let pattern_str = if options.fixed_strings {
            regex::escape(p)
        } else {
            p.clone()
        };

        let final_pattern = if options.line_regexp {
            format!("^(?:{})$", pattern_str)
        } else {
            pattern_str
        };

        let mut builder = RegexBuilder::new(&final_pattern);
        builder.case_insensitive(options.ignore_case);
        match builder.build() {
            Ok(re) => compiled.push(re),
            Err(e) => {
                return Err(format!("invalid regex '{}': {}", p, e));
            }
        }
    }

    Ok((compiled, final_patterns))
}

/// Determines the filename prefix to display based on formatting options.
fn determine_display_filename(
    display_path: &OsStr,
    options: &GrepOptions,
    force_filename: bool,
) -> String {
    if options.no_filename {
        String::new()
    } else if options.with_filename || force_filename {
        display_path.to_string_lossy().into_owned()
    } else {
        String::new()
    }
}

/// Reads entries from a directory, sorts them by name for determinism, and handles errors.
fn read_dir_entries(
    dir_path: &Path,
    display_path: &OsStr,
    options: &GrepOptions,
    stderr: &mut impl Write,
    error_encountered: &mut bool,
) -> Option<Vec<fs::DirEntry>> {
    match fs::read_dir(dir_path) {
        Ok(read) => {
            let mut list = Vec::new();
            for entry in read {
                match entry {
                    Ok(e) => list.push(e),
                    Err(err) => {
                        if !options.no_messages {
                            let _ = writeln!(stderr, "grep: {}: {}", display_path.to_string_lossy(), err);
                        }
                        *error_encountered = true;
                    }
                }
            }
            list.sort_by_key(|e| e.file_name());
            Some(list)
        }
        Err(e) => {
            if !options.no_messages {
                let _ = writeln!(stderr, "grep: {}: {}", display_path.to_string_lossy(), e);
            }
            *error_encountered = true;
            None
        }
    }
}

/// Recursively searches a directory for pattern matches.
fn search_recursive(
    dir_path: &Path,
    display_path: &OsStr,
    follow_symlinks: bool,
    options: &GrepOptions,
    compiled_regexes: &[Regex],
    raw_patterns: &[String],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
    match_found: &mut bool,
    error_encountered: &mut bool,
    visited: &mut HashSet<PathBuf>,
) {
    let canonical = match fs::canonicalize(dir_path) {
        Ok(c) => c,
        Err(e) => {
            if !options.no_messages {
                let _ = writeln!(stderr, "grep: {}: {}", display_path.to_string_lossy(), e);
            }
            *error_encountered = true;
            return;
        }
    };

    if !visited.insert(canonical) {
        // Loop detected
        return;
    }

    let entries = match read_dir_entries(dir_path, display_path, options, stderr, error_encountered) {
        Some(e) => e,
        None => return,
    };

    for entry in entries {
        let entry_path = entry.path();
        let entry_display = Path::new(display_path).join(entry.file_name());
        let metadata = match fs::symlink_metadata(&entry_path) {
            Ok(m) => m,
            Err(e) => {
                if !options.no_messages {
                    let _ = writeln!(stderr, "grep: {}: {}", entry_display.to_string_lossy(), e);
                }
                *error_encountered = true;
                continue;
            }
        };

        let is_dir = if metadata.is_symlink() && follow_symlinks {
            fs::metadata(&entry_path).map(|m| m.is_dir()).unwrap_or(false)
        } else {
            metadata.is_dir()
        };

        if is_dir {
            search_recursive(
                &entry_path,
                entry_display.as_os_str(),
                follow_symlinks,
                options,
                compiled_regexes,
                raw_patterns,
                stdout,
                stderr,
                match_found,
                error_encountered,
                visited,
            );
        } else if metadata.is_file() || (metadata.is_symlink() && follow_symlinks) {
            let filename_to_print = determine_display_filename(entry_display.as_os_str(), options, true);
            search_file(
                &entry_path,
                entry_display.as_os_str(),
                &filename_to_print,
                options,
                compiled_regexes,
                raw_patterns,
                stdout,
                stderr,
                match_found,
                error_encountered,
            );
        }
    }
}

/// Opens a file and initiates the line-by-line pattern search.
fn search_file(
    file_path: &Path,
    display_path: &OsStr,
    filename_to_print: &str,
    options: &GrepOptions,
    compiled_regexes: &[Regex],
    raw_patterns: &[String],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
    match_found: &mut bool,
    error_encountered: &mut bool,
) {
    match File::open(file_path) {
        Ok(file) => {
            let mut reader = BufReader::new(file);
            let res = search_reader(
                &mut reader,
                filename_to_print,
                options,
                compiled_regexes,
                stdout,
                raw_patterns,
            );
            match res {
                Ok(matched) => {
                    if matched {
                        *match_found = true;
                    }
                }
                Err(e) => {
                    if !options.no_messages {
                        let _ = writeln!(stderr, "grep: {}: {}", display_path.to_string_lossy(), e);
                    }
                    *error_encountered = true;
                }
            }
        }
        Err(e) => {
            if !options.no_messages {
                let _ = writeln!(stderr, "grep: {}: {}", display_path.to_string_lossy(), e);
            }
            *error_encountered = true;
        }
    }
}

/// Checks if a single trimmed line matches any of the compiled regexes.
/// Returns a tuple containing:
/// 1. A boolean indicating whether the line is considered a match (after taking `invert_match` into account).
/// 2. A vector of ranges (start index, end index, matched substring) for any matches found
///    (used if `only_matching` is enabled).
fn match_line(
    trimmed_line: &str,
    compiled_regexes: &[Regex],
    raw_patterns: &[String],
    options: &GrepOptions,
) -> (bool, Vec<(usize, usize, String)>) {
    let mut matched = false;
    let mut matches = Vec::new();

    for (idx, re) in compiled_regexes.iter().enumerate() {
        let raw_p = &raw_patterns[idx];
        if options.word_regexp {
            if raw_p.is_empty() {
                // Empty pattern never matches under word_regexp
                continue;
            }
            for m in re.find_iter(trimmed_line) {
                let start = m.start();
                let end = m.end();
                let char_before = if start > 0 { trimmed_line[..start].chars().next_back() } else { None };
                let char_after = if end < trimmed_line.len() { trimmed_line[end..].chars().next() } else { None };
                let is_word_before = char_before.map_or(false, |c| c.is_alphanumeric() || c == '_');
                let is_word_after = char_after.map_or(false, |c| c.is_alphanumeric() || c == '_');
                if !is_word_before && !is_word_after {
                    matched = true;
                    if options.only_matching {
                        matches.push((start, end, m.as_str().to_string()));
                    }
                }
            }
        } else {
            if re.is_match(trimmed_line) {
                matched = true;
                if options.only_matching {
                    for m in re.find_iter(trimmed_line) {
                        matches.push((m.start(), m.end(), m.as_str().to_string()));
                    }
                }
            }
        }
    }

    if options.invert_match {
        matched = !matched;
    }

    (matched, matches)
}

/// Prints standard grep line prefix (filename, line number, byte offset).
fn print_prefix(
    writer: &mut impl Write,
    filename: &str,
    line_num: usize,
    byte_offset: usize,
    options: &GrepOptions,
) -> io::Result<()> {
    if !filename.is_empty() {
        writer.write_all(filename.as_bytes())?;
        if options.null_separator {
            writer.write_all(b"\0")?;
        } else {
            writer.write_all(b":")?;
        }
    }
    if options.line_number {
        write!(writer, "{}:", line_num)?;
    }
    if options.byte_offset {
        write!(writer, "{}:", byte_offset)?;
    }
    Ok(())
}

/// Searches the content of a reader for matches line-by-line.
fn search_reader(
    reader: &mut impl Read,
    filename: &str,
    options: &GrepOptions,
    compiled_regexes: &[Regex],
    stdout: &mut impl Write,
    raw_patterns: &[String],
) -> io::Result<bool> {
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();
    let mut line_num = 0;
    let mut byte_offset = 0;
    let mut match_count = 0;
    let mut file_matched = false;

    let has_filename = !filename.is_empty();

    loop {
        line.clear();
        let n = buf_reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }

        line_num += 1;

        // strip trailing newline for matching
        let trimmed_line = if line.ends_with('\n') {
            if line.ends_with("\r\n") {
                &line[..line.len() - 2]
            } else {
                &line[..line.len() - 1]
            }
        } else {
            &line[..]
        };

        let (matched, mut matches) = match_line(trimmed_line, compiled_regexes, raw_patterns, options);

        if matched {
            file_matched = true;
            match_count += 1;

            if !options.quiet && !options.count && !options.files_with_matches && !options.files_without_match {
                if options.only_matching && !options.invert_match {
                    // Sort matches by start index to ensure output order
                    matches.sort_by_key(|m| m.0);
                    for (start, _end, match_str) in matches {
                        print_prefix(stdout, filename, line_num, byte_offset + start, options)?;
                        stdout.write_all(match_str.as_bytes())?;
                        stdout.write_all(b"\n")?;
                    }
                } else {
                    print_prefix(stdout, filename, line_num, byte_offset, options)?;
                    stdout.write_all(line.as_bytes())?;
                    // Ensure newline if line didn't end with one
                    if !line.ends_with('\n') {
                        stdout.write_all(b"\n")?;
                    }
                }
            }

            if let Some(max) = options.max_count {
                if match_count >= max {
                    break;
                }
            }
        }

        byte_offset += n;
    }

    if options.files_with_matches {
        if file_matched && !options.quiet {
            stdout.write_all(filename.as_bytes())?;
            stdout.write_all(b"\n")?;
        }
    } else if options.files_without_match {
        if !file_matched && !options.quiet {
            stdout.write_all(filename.as_bytes())?;
            stdout.write_all(b"\n")?;
        }
    } else if options.count && !options.quiet {
        if has_filename {
            stdout.write_all(filename.as_bytes())?;
            if options.null_separator {
                stdout.write_all(b"\0")?;
            } else {
                stdout.write_all(b":")?;
            }
        }
        writeln!(stdout, "{}", match_count)?;
    }

    Ok(file_matched)
}

/// Processes a single file path or stdin operand, determining if matches are found.
fn process_file_arg(
    file_arg: &OsStr,
    cwd: &Path,
    num_files: usize,
    options: &GrepOptions,
    compiled_regexes: &[Regex],
    final_patterns: &[String],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
    visited: &mut HashSet<PathBuf>,
) -> (bool, bool) {
    let mut match_found = false;
    let mut error_encountered = false;

    if file_arg == "-" {
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        let res = search_reader(
            &mut handle,
            "<stdin>",
            options,
            compiled_regexes,
            stdout,
            final_patterns,
        );
        match res {
            Ok(matched) => {
                if matched {
                    match_found = true;
                }
            }
            Err(e) => {
                if !options.no_messages {
                    let _ = writeln!(stderr, "grep: <stdin>: {}", e);
                }
                error_encountered = true;
            }
        }
    } else {
        let path = cwd.join(file_arg);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                if !options.no_messages {
                    let _ = writeln!(stderr, "grep: {}: {}", file_arg.to_string_lossy(), e);
                }
                return (false, true);
            }
        };

        let is_dir = if metadata.is_symlink() && (options.dereference_recursive || !options.recursive) {
            fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false)
        } else {
            metadata.is_dir()
        };

        if is_dir {
            if options.recursive || options.dereference_recursive {
                let follow = options.dereference_recursive;
                let mut matched = false;
                let mut err = false;
                search_recursive(
                    &path,
                    file_arg,
                    follow,
                    options,
                    compiled_regexes,
                    final_patterns,
                    stdout,
                    stderr,
                    &mut matched,
                    &mut err,
                    visited,
                );
                if matched {
                    match_found = true;
                }
                if err {
                    error_encountered = true;
                }
            } else {
                if !options.no_messages {
                    let _ = writeln!(stderr, "grep: {}: Is a directory", file_arg.to_string_lossy());
                }
                error_encountered = true;
            }
        } else {
            let force_filename = num_files > 1;
            let filename_to_print = determine_display_filename(file_arg, options, force_filename);
            search_file(
                &path,
                file_arg,
                &filename_to_print,
                options,
                compiled_regexes,
                final_patterns,
                stdout,
                stderr,
                &mut match_found,
                &mut error_encountered,
            );
        }
    }

    (match_found, error_encountered)
}

/// Runs the main grep task using parsed arguments, writers, and working directory.
pub fn run<I>(
    args: I,
    cwd: &Path,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let args_vec: Vec<OsString> = args.into_iter().collect();
    let parse_result = match parse_options(&args_vec, cwd, stdout, stderr) {
        Ok(res) => res,
        Err(code) => return code,
    };

    let (options, mut files) = match parse_result {
        ParseResult::EarlyExit(code) => return code,
        ParseResult::Success { options, files } => (options, files),
    };

    let (compiled_regexes, final_patterns) = match compile_regexes(&options.patterns, &options) {
        Ok(res) => res,
        Err(err) => {
            if !options.no_messages {
                let _ = writeln!(stderr, "grep: {}", err);
            }
            return 2;
        }
    };

    if files.is_empty() {
        files.push(OsString::from("-"));
    }

    let mut match_found = false;
    let mut error_encountered = false;
    let num_files = files.len();
    let mut visited = HashSet::new();

    for file_arg in files {
        let (matched, err) = process_file_arg(
            &file_arg,
            cwd,
            num_files,
            &options,
            &compiled_regexes,
            &final_patterns,
            stdout,
            stderr,
            &mut visited,
        );
        if matched {
            match_found = true;
            if options.quiet {
                return 0;
            }
        }
        if err {
            error_encountered = true;
        }
    }

    if options.quiet && match_found {
        0
    } else if error_encountered {
        2
    } else if match_found {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_options_basic() {
        let args = vec![
            OsString::from("-i"),
            OsString::from("pattern"),
            OsString::from("file.txt"),
        ];
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = parse_options(&args, Path::new("."), &mut stdout, &mut stderr);
        assert!(result.is_ok());
        if let Ok(ParseResult::Success { options, files }) = result {
            assert!(options.ignore_case);
            assert_eq!(options.patterns, vec!["pattern"]);
            assert_eq!(files, vec![OsString::from("file.txt")]);
        } else {
            panic!("Expected ParseResult::Success");
        }
    }

    #[test]
    fn test_parse_options_missing_pattern() {
        let args = vec![];
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = parse_options(&args, Path::new("."), &mut stdout, &mut stderr);
        assert_eq!(result, Err(2));
        assert!(String::from_utf8(stderr)
            .unwrap()
            .contains("Usage: grep"));
    }

    #[test]
    fn test_parse_options_invalid_flag() {
        let args = vec![OsString::from("--invalid-flag")];
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = parse_options(&args, Path::new("."), &mut stdout, &mut stderr);
        assert_eq!(result, Err(2));
        assert!(String::from_utf8(stderr)
            .unwrap()
            .contains("grep: unrecognized option"));
    }

    #[test]
    fn test_compile_regexes_simple() {
        let patterns = vec!["hello".to_string(), "world".to_string()];
        let mut options = GrepOptions::new();
        options.ignore_case = true;
        let (res, final_p) = compile_regexes(&patterns, &options).unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(final_p, vec!["hello".to_string(), "world".to_string()]);
        assert!(res[0].is_match("HeLlO"));
    }

    #[test]
    fn test_compile_regexes_fixed() {
        let patterns = vec!["h.llo".to_string()];
        let mut options = GrepOptions::new();
        options.fixed_strings = true;
        let (res, final_p) = compile_regexes(&patterns, &options).unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(final_p, vec!["h.llo".to_string()]);
        assert!(res[0].is_match("h.llo"));
        assert!(!res[0].is_match("hello"));
    }

    #[test]
    fn test_match_line_simple() {
        let options = GrepOptions::new();
        let patterns = vec!["abc".to_string()];
        let (res, raw) = compile_regexes(&patterns, &options).unwrap();

        let (matched, ranges) = match_line("123 abc 456", &res, &raw, &options);
        assert!(matched);
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_match_line_word_regexp() {
        let mut options = GrepOptions::new();
        options.word_regexp = true;
        let patterns = vec!["abc".to_string()];
        let (res, raw) = compile_regexes(&patterns, &options).unwrap();

        // Word boundary: matches
        let (matched, _) = match_line("123 abc 456", &res, &raw, &options);
        assert!(matched);

        // No word boundary: no match
        let (matched, _) = match_line("123abc456", &res, &raw, &options);
        assert!(!matched);
    }

    #[test]
    fn test_match_line_only_matching() {
        let mut options = GrepOptions::new();
        options.only_matching = true;
        let patterns = vec!["abc".to_string()];
        let (res, raw) = compile_regexes(&patterns, &options).unwrap();

        let (matched, ranges) = match_line("123 abc 456 abc", &res, &raw, &options);
        assert!(matched);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0], (4, 7, "abc".to_string()));
        assert_eq!(ranges[1], (12, 15, "abc".to_string()));
    }
}
