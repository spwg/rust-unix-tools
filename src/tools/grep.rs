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

struct GrepOptions {
    patterns: Vec<String>,
    ignore_case: bool,
    invert_match: bool,
    word_regexp: bool,
    line_regexp: bool,
    count: bool,
    files_without_match: bool,
    files_with_matches: bool,
    max_count: Option<usize>,
    only_matching: bool,
    quiet: bool,
    no_messages: bool,
    byte_offset: bool,
    line_number: bool,
    with_filename: bool,
    no_filename: bool,
    recursive: bool,
    dereference_recursive: bool,
    fixed_strings: bool,
    null_separator: bool,
}

impl GrepOptions {
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

pub fn run<I>(
    args: I,
    cwd: &Path,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let mut options = GrepOptions::new();
    let mut files = Vec::new();
    let mut args_iter = args.into_iter().peekable();

    while let Some(arg) = args_iter.next() {
        let arg_str = arg.to_string_lossy();
        if arg_str == "--" {
            for remaining in args_iter {
                files.push(remaining);
            }
            break;
        } else if arg_str.starts_with("--") {
            let (opt_name, opt_val) = if let Some(idx) = arg_str.find('=') {
                (&arg_str[..idx], Some(&arg_str[idx + 1..]))
            } else {
                (arg_str.as_ref(), None)
            };

            match opt_name {
                "--regexp" => {
                    let val = opt_val
                        .map(|s| s.to_string())
                        .or_else(|| args_iter.next().map(|s| s.to_string_lossy().into_owned()));
                    if let Some(p) = val {
                        options.patterns.push(p);
                    } else {
                        let _ = writeln!(stderr, "grep: option '--regexp' requires an argument");
                        return 2;
                    }
                }
                "--file" => {
                    let val = opt_val
                        .map(|s| s.to_string())
                        .or_else(|| args_iter.next().map(|s| s.to_string_lossy().into_owned()));
                    if let Some(f) = val {
                        if let Err(e) = read_patterns_from_file(&f, cwd, &mut options.patterns) {
                            if !options.no_messages {
                                let _ = writeln!(stderr, "grep: {}: {}", f, e);
                            }
                            return 2;
                        }
                    } else {
                        let _ = writeln!(stderr, "grep: option '--file' requires an argument");
                        return 2;
                    }
                }
                "--ignore-case" => options.ignore_case = true,
                "--invert-match" => options.invert_match = true,
                "--word-regexp" => options.word_regexp = true,
                "--line-regexp" => options.line_regexp = true,
                "--count" => options.count = true,
                "--files-without-match" => options.files_without_match = true,
                "--files-with-matches" => options.files_with_matches = true,
                "--max-count" => {
                    let val = opt_val
                        .map(|s| s.to_string())
                        .or_else(|| args_iter.next().map(|s| s.to_string_lossy().into_owned()));
                    if let Some(s) = val {
                        if let Ok(n) = s.parse::<usize>() {
                            options.max_count = Some(n);
                        } else {
                            let _ = writeln!(stderr, "grep: invalid max-count");
                            return 2;
                        }
                    } else {
                        let _ = writeln!(stderr, "grep: option '--max-count' requires an argument");
                        return 2;
                    }
                }
                "--only-matching" => options.only_matching = true,
                "--quiet" | "--silent" => options.quiet = true,
                "--no-messages" => options.no_messages = true,
                "--byte-offset" => options.byte_offset = true,
                "--line-number" => options.line_number = true,
                "--with-filename" => options.with_filename = true,
                "--no-filename" => options.no_filename = true,
                "--recursive" => options.recursive = true,
                "--dereference-recursive" => options.dereference_recursive = true,
                "--fixed-strings" => options.fixed_strings = true,
                "--extended-regexp" | "--basic-regexp" => {} // Treat both as standard regex crate
                "--null" => options.null_separator = true,
                "--help" => {
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
                    return 0;
                }
                "--version" => {
                    let _ = writeln!(stdout, "grep (rust-unix-tools) 0.1.0");
                    return 0;
                }
                _ => {
                    let _ = writeln!(stderr, "grep: unrecognized option '{}'", arg_str);
                    return 2;
                }
            }
        } else if arg_str.starts_with('-') && arg_str != "-" {
            let mut chars = arg_str.chars().skip(1).peekable();
            while let Some(c) = chars.next() {
                match c {
                    'e' => {
                        let val = if chars.peek().is_some() {
                            let rest: String = chars.collect();
                            Some(rest)
                        } else {
                            args_iter.next().map(|s| s.to_string_lossy().into_owned())
                        };
                        if let Some(p) = val {
                            options.patterns.push(p);
                        } else {
                            let _ = writeln!(stderr, "grep: option requires an argument -- 'e'");
                            return 2;
                        }
                        break;
                    }
                    'f' => {
                        let val = if chars.peek().is_some() {
                            let rest: String = chars.collect();
                            Some(rest)
                        } else {
                            args_iter.next().map(|s| s.to_string_lossy().into_owned())
                        };
                        if let Some(f) = val {
                            if let Err(e) = read_patterns_from_file(&f, cwd, &mut options.patterns) {
                                if !options.no_messages {
                                    let _ = writeln!(stderr, "grep: {}: {}", f, e);
                                }
                                return 2;
                            }
                        } else {
                            let _ = writeln!(stderr, "grep: option requires an argument -- 'f'");
                            return 2;
                        }
                        break;
                    }
                    'm' => {
                        let val = if chars.peek().is_some() {
                            let rest: String = chars.collect();
                            Some(rest)
                        } else {
                            args_iter.next().map(|s| s.to_string_lossy().into_owned())
                        };
                        if let Some(s) = val {
                            if let Ok(n) = s.parse::<usize>() {
                                options.max_count = Some(n);
                            } else {
                                let _ = writeln!(stderr, "grep: invalid max-count");
                                return 2;
                            }
                        } else {
                            let _ = writeln!(stderr, "grep: option requires an argument -- 'm'");
                            return 2;
                        }
                        break;
                    }
                    'i' | 'y' => options.ignore_case = true,
                    'v' => options.invert_match = true,
                    'w' => options.word_regexp = true,
                    'x' => options.line_regexp = true,
                    'c' => options.count = true,
                    'L' => options.files_without_match = true,
                    'l' => options.files_with_matches = true,
                    'o' => options.only_matching = true,
                    'q' => options.quiet = true,
                    's' => options.no_messages = true,
                    'b' => options.byte_offset = true,
                    'n' => options.line_number = true,
                    'H' => options.with_filename = true,
                    'h' => options.no_filename = true,
                    'r' => options.recursive = true,
                    'R' => options.dereference_recursive = true,
                    'E' | 'G' => {} // Standard regex crate support
                    'F' => options.fixed_strings = true,
                    'Z' => options.null_separator = true,
                    'V' => {
                        let _ = writeln!(stdout, "grep (rust-unix-tools) 0.1.0");
                        return 0;
                    }
                    _ => {
                        let _ = writeln!(stderr, "grep: invalid option -- '{}'", c);
                        return 2;
                    }
                }
            }
        } else {
            files.push(arg);
        }
    }

    // If no patterns specified via -e or -f, the first positional argument is the pattern
    if options.patterns.is_empty() {
        if files.is_empty() {
            let _ = writeln!(stderr, "Usage: grep [OPTION]... PATTERNS [FILE]...");
            return 2;
        }
        let pattern_arg = files.remove(0);
        options.patterns.push(pattern_arg.to_string_lossy().into_owned());
    }

    // Split patterns by newline
    let mut final_patterns = Vec::new();
    for p in &options.patterns {
        for sub in p.split('\n') {
            final_patterns.push(sub.to_string());
        }
    }

    // Compile regexes
    let mut compiled_regexes = Vec::new();
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
            Ok(re) => compiled_regexes.push(re),
            Err(e) => {
                if !options.no_messages {
                    let _ = writeln!(stderr, "grep: invalid regex '{}': {}", p, e);
                }
                return 2;
            }
        }
    }

    if files.is_empty() {
        files.push(OsString::from("-"));
    }

    let mut match_found = false;
    let mut error_encountered = false;
    let num_files = files.len();

    let mut visited = HashSet::new();

    for file_arg in files {
        if file_arg == "-" {
            let stdin = io::stdin();
            let mut handle = stdin.lock();
            let res = search_reader(
                &mut handle,
                "<stdin>",
                &options,
                &compiled_regexes,
                stdout,
                &final_patterns,
            );
            match res {
                Ok(matched) => {
                    if matched {
                        match_found = true;
                        if options.quiet {
                            return 0;
                        }
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
            let path = cwd.join(&file_arg);
            let metadata = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    if !options.no_messages {
                        let _ = writeln!(stderr, "grep: {}: {}", file_arg.to_string_lossy(), e);
                    }
                    error_encountered = true;
                    continue;
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
                        &file_arg,
                        follow,
                        &options,
                        &compiled_regexes,
                        &final_patterns,
                        stdout,
                        stderr,
                        &mut matched,
                        &mut err,
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
                } else {
                    if !options.no_messages {
                        let _ = writeln!(stderr, "grep: {}: Is a directory", file_arg.to_string_lossy());
                    }
                    error_encountered = true;
                }
            } else {
                match File::open(&path) {
                    Ok(file) => {
                        let mut reader = BufReader::new(file);
                        let print_filename = (num_files > 1 || options.with_filename) && !options.no_filename;
                        let file_arg_str = file_arg.to_string_lossy();
                        let filename_to_print = if print_filename {
                            &*file_arg_str
                        } else {
                            ""
                        };

                        let res = search_reader(
                            &mut reader,
                            filename_to_print,
                            &options,
                            &compiled_regexes,
                            stdout,
                            &final_patterns,
                        );
                        match res {
                            Ok(matched) => {
                                if matched {
                                    match_found = true;
                                    if options.quiet {
                                        return 0;
                                    }
                                }
                            }
                            Err(e) => {
                                if !options.no_messages {
                                    let _ = writeln!(stderr, "grep: {}: {}", file_arg.to_string_lossy(), e);
                                }
                                error_encountered = true;
                            }
                        }
                    }
                    Err(e) => {
                        if !options.no_messages {
                            let _ = writeln!(stderr, "grep: {}: {}", file_arg.to_string_lossy(), e);
                        }
                        error_encountered = true;
                    }
                }
            }
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

    let entries = match fs::read_dir(dir_path) {
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
            // Sort by file name for determinism in testing
            list.sort_by_key(|e| e.file_name());
            list
        }
        Err(e) => {
            if !options.no_messages {
                let _ = writeln!(stderr, "grep: {}: {}", display_path.to_string_lossy(), e);
            }
            *error_encountered = true;
            return;
        }
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
            match File::open(&entry_path) {
                Ok(file) => {
                    let mut reader = BufReader::new(file);
                    let filename = entry_display.to_string_lossy();
                    let res = search_reader(
                        &mut reader,
                        &filename,
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
                                let _ = writeln!(stderr, "grep: {}: {}", filename, e);
                            }
                            *error_encountered = true;
                        }
                    }
                }
                Err(e) => {
                    if !options.no_messages {
                        let _ = writeln!(stderr, "grep: {}: {}", entry_display.to_string_lossy(), e);
                    }
                    *error_encountered = true;
                }
            }
        }
    }
}

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

        let mut matched = false;
        let mut word_matches = Vec::new();

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
                            word_matches.push((start, end, m.as_str().to_string()));
                        }
                    }
                }
            } else {
                if re.is_match(trimmed_line) {
                    matched = true;
                    if options.only_matching {
                        for m in re.find_iter(trimmed_line) {
                            word_matches.push((m.start(), m.end(), m.as_str().to_string()));
                        }
                    }
                }
            }
        }

        if options.invert_match {
            matched = !matched;
        }

        if matched {
            file_matched = true;
            match_count += 1;

            if !options.quiet && !options.count && !options.files_with_matches && !options.files_without_match {
                if options.only_matching && !options.invert_match {
                    // Sort word matches by start index to ensure output order
                    word_matches.sort_by_key(|m| m.0);
                    for (start, _end, match_str) in word_matches {
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
