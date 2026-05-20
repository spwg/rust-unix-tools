//! GNU-style `ls`.
//!
//! The implementation targets the core behaviors described by the GNU
//! coreutils `ls(1)` man page shipped in this repository's test fixtures. It
//! favors deterministic non-terminal output: one entry per line unless an
//! explicit format option requests otherwise.

use crate::getopt::{HasArg, OptSpec, ParsedArg};
use std::cmp::Ordering;
use std::ffi::{OsStr, OsString};
use std::fs::{self, Metadata};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Format {
    One,
    Columns,
    Commas,
    Long,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Indicator {
    None,
    Slash,
    FileType,
    Classify,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Sort {
    Name,
    None,
    Size,
    Time,
    Extension,
}

#[derive(Clone, Copy)]
enum TimeField {
    Modified,
    Accessed,
    Changed,
}

#[derive(Clone)]
struct Options {
    /// Include `.`-prefixed entries, including implied `.` and `..`.
    all: bool,
    /// Include hidden entries but suppress implied `.` and `..`.
    almost_all: bool,
    /// Suppress entries ending with `~`.
    ignore_backups: bool,
    /// List directory operands as entries instead of listing their contents.
    directory: bool,
    /// Descend into listed directories.
    recursive: bool,
    /// Sort directories before non-directories inside each sort group.
    group_dirs_first: bool,
    /// Prefix each entry with its inode number.
    inode: bool,
    /// Prefix each entry with allocated block count.
    size: bool,
    /// Render sizes and block counts using unit suffixes.
    human_readable: bool,
    /// Use numeric user and group IDs in long output.
    numeric: bool,
    /// Suppress the owner column in long output.
    omit_owner: bool,
    /// Suppress the group column in long output.
    omit_group: bool,
    /// Follow symlinks when reading metadata.
    dereference: bool,
    /// Selected output layout.
    format: Format,
    /// Optional file-type suffix style.
    indicator: Indicator,
    /// Active sort key.
    sort: Sort,
    /// Reverse the selected sort order.
    reverse: bool,
    /// Timestamp field used for long output and time sorting.
    time: TimeField,
    /// Block size used for `-s` and directory totals.
    block_size: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            all: false,
            almost_all: false,
            ignore_backups: false,
            directory: false,
            recursive: false,
            group_dirs_first: false,
            inode: false,
            size: false,
            human_readable: false,
            numeric: false,
            omit_owner: false,
            omit_group: false,
            dereference: false,
            format: Format::One,
            indicator: Indicator::None,
            sort: Sort::Name,
            reverse: false,
            time: TimeField::Modified,
            block_size: 1024,
        }
    }
}

struct Entry {
    display_name: OsString,
    display_path: PathBuf,
    path: PathBuf,
    metadata: Metadata,
}

struct ParseResult {
    options: Options,
    operands: Vec<OsString>,
    help: bool,
    version: bool,
}

/// Runs `ls` with raw byte arguments relative to `cwd`.
///
/// Output and diagnostics are written to the supplied writers. The return
/// value follows GNU `ls` exit-status classes: `0` for success, `1` for minor
/// traversal problems, and `2` for serious command-line or operand errors.
pub fn run<I, T>(args: I, cwd: &Path, stdout: &mut impl Write, stderr: &mut impl Write) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    match parse_args(args) {
        Ok(parsed) if parsed.help => write_help(stdout),
        Ok(parsed) if parsed.version => write_version(stdout),
        Ok(parsed) => list(parsed.options, parsed.operands, cwd, stdout, stderr),
        Err(error) => {
            let _ = writeln!(stderr, "ls: {error}");
            2
        }
    }
}

fn write_help(stdout: &mut impl Write) -> i32 {
    let _ = stdout.write_all(
        b"Usage: ls [OPTION]... [FILE]...\n\
List information about the FILEs (the current directory by default).\n\n\
  -a, --all                  do not ignore entries starting with .\n\
  -A, --almost-all           do not list implied . and ..\n\
  -d, --directory            list directories themselves, not their contents\n\
  -F, --classify[=WHEN]      append indicator (one of */=>@|) to entries\n\
  -l                         use a long listing format\n\
  -R, --recursive            list subdirectories recursively\n\
      --sort=WORD            sort by none, size, time, extension, or name\n\
      --help                 display this help and exit\n\
      --version              output version information and exit\n",
    );
    0
}

fn write_version(stdout: &mut impl Write) -> i32 {
    let _ = stdout.write_all(b"ls (rust-unix-tools) 0.1.0\n");
    0
}

fn parse_args<I, T>(args: I) -> Result<ParseResult, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();

    let specs = [
        // Help & Version
        OptSpec { short: None, long: Some("help"), has_arg: HasArg::No },
        OptSpec { short: None, long: Some("version"), has_arg: HasArg::No },

        // Options with short & long mapping
        OptSpec { short: Some('a'), long: Some("all"), has_arg: HasArg::No },
        OptSpec { short: Some('A'), long: Some("almost-all"), has_arg: HasArg::No },
        OptSpec { short: Some('B'), long: Some("ignore-backups"), has_arg: HasArg::No },
        OptSpec { short: Some('d'), long: Some("directory"), has_arg: HasArg::No },
        OptSpec { short: Some('R'), long: Some("recursive"), has_arg: HasArg::No },
        OptSpec { short: Some('F'), long: Some("classify"), has_arg: HasArg::Optional },
        OptSpec { short: Some('i'), long: Some("inode"), has_arg: HasArg::No },
        OptSpec { short: Some('s'), long: Some("size"), has_arg: HasArg::No },
        OptSpec { short: Some('h'), long: Some("human-readable"), has_arg: HasArg::No },
        OptSpec { short: Some('k'), long: Some("kibibytes"), has_arg: HasArg::No },
        OptSpec { short: Some('n'), long: Some("numeric-uid-gid"), has_arg: HasArg::No },
        OptSpec { short: Some('L'), long: Some("dereference"), has_arg: HasArg::No },

        // Short options without long counterparts
        OptSpec { short: Some('c'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('C'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('x'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('f'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('U'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('g'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('G'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('H'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('l'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('m'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('N'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('q'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('Q'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('w'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('Z'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('o'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('p'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('r'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('S'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('t'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('u'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('v'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('X'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('1'), long: None, has_arg: HasArg::No },
        OptSpec { short: Some('-'), long: None, has_arg: HasArg::No },

        // Long options without short counterparts (that have logic)
        OptSpec { short: None, long: Some("group-directories-first"), has_arg: HasArg::No },
        OptSpec { short: None, long: Some("no-group"), has_arg: HasArg::No },
        OptSpec { short: None, long: Some("color"), has_arg: HasArg::Optional },
        OptSpec { short: None, long: Some("hyperlink"), has_arg: HasArg::Optional },
        OptSpec { short: None, long: Some("file-type"), has_arg: HasArg::No },
        OptSpec { short: None, long: Some("indicator-style"), has_arg: HasArg::Yes },
        OptSpec { short: None, long: Some("format"), has_arg: HasArg::Yes },
        OptSpec { short: None, long: Some("sort"), has_arg: HasArg::Yes },
        OptSpec { short: None, long: Some("time"), has_arg: HasArg::Yes },
        OptSpec { short: None, long: Some("block-size"), has_arg: HasArg::Yes },
        OptSpec { short: None, long: Some("si"), has_arg: HasArg::No },
        OptSpec { short: None, long: Some("zero"), has_arg: HasArg::No },

        // Long options without logic (no-op or consume and discard)
        OptSpec { short: None, long: Some("author"), has_arg: HasArg::Optional },
        OptSpec { short: None, long: Some("dired"), has_arg: HasArg::Optional },
        OptSpec { short: None, long: Some("full-time"), has_arg: HasArg::Optional },
        OptSpec { short: None, long: Some("hide"), has_arg: HasArg::Yes },
        OptSpec { short: None, long: Some("ignore"), has_arg: HasArg::Yes },
        OptSpec { short: None, long: Some("literal"), has_arg: HasArg::Optional },
        OptSpec { short: None, long: Some("quote-name"), has_arg: HasArg::Optional },
        OptSpec { short: None, long: Some("quoting-style"), has_arg: HasArg::Yes },
        OptSpec { short: None, long: Some("show-control-chars"), has_arg: HasArg::Optional },
        OptSpec { short: None, long: Some("tabsize"), has_arg: HasArg::Yes },
        OptSpec { short: None, long: Some("width"), has_arg: HasArg::Yes },
        OptSpec { short: None, long: Some("context"), has_arg: HasArg::Optional },
        OptSpec { short: None, long: Some("dereference-command-line"), has_arg: HasArg::Optional },
        OptSpec { short: None, long: Some("dereference-command-line-symlink-to-dir"), has_arg: HasArg::Optional },
    ];

    let posixly_correct = std::env::var_os("POSIXLY_CORRECT").is_some();
    let parsed_args = crate::getopt::parse(&args, &specs, posixly_correct)?;

    let mut options = Options::default();
    let mut operands = Vec::new();
    let mut help = false;
    let mut version = false;

    for parsed_arg in parsed_args {
        match parsed_arg {
            ParsedArg::Option { short, long, value } => {
                if let Some(l) = long {
                    match l {
                        "help" => help = true,
                        "version" => version = true,
                        "all" => options.all = true,
                        "almost-all" => options.almost_all = true,
                        "ignore-backups" => options.ignore_backups = true,
                        "directory" => options.directory = true,
                        "recursive" => options.recursive = true,
                        "group-directories-first" => options.group_dirs_first = true,
                        "inode" => options.inode = true,
                        "size" => options.size = true,
                        "human-readable" => options.human_readable = true,
                        "kibibytes" => options.block_size = 1024,
                        "numeric-uid-gid" => {
                            options.numeric = true;
                            options.format = Format::Long;
                        }
                        "no-group" => options.omit_group = true,
                        "dereference" => options.dereference = true,
                        "color" | "hyperlink" => {}
                        "classify" => {
                            if value.map(|v| v.as_bytes()) != Some(b"never") {
                                options.indicator = Indicator::Classify;
                            }
                        }
                        "file-type" => options.indicator = Indicator::FileType,
                        "indicator-style" => {
                            let val = value.ok_or_else(|| "option '--indicator-style' requires an argument".to_string())?;
                            match val.as_bytes() {
                                b"none" => options.indicator = Indicator::None,
                                b"slash" => options.indicator = Indicator::Slash,
                                b"file-type" => options.indicator = Indicator::FileType,
                                b"classify" => options.indicator = Indicator::Classify,
                                _ => return Err(format!("invalid indicator style: {}", os_lossy(val))),
                            }
                        }
                        "format" => {
                            let val = value.ok_or_else(|| "option '--format' requires an argument".to_string())?;
                            match val.as_bytes() {
                                b"single-column" => options.format = Format::One,
                                b"long" | b"verbose" => options.format = Format::Long,
                                b"commas" => options.format = Format::Commas,
                                b"vertical" | b"across" | b"horizontal" => options.format = Format::Columns,
                                other => return Err(format!("invalid format: {}", bytes_lossy(other))),
                            }
                        }
                        "sort" => {
                            let val = value.ok_or_else(|| "option '--sort' requires an argument".to_string())?;
                            set_sort(val, &mut options)?;
                        }
                        "time" => {
                            let val = value.ok_or_else(|| "option '--time' requires an argument".to_string())?;
                            set_time(val, &mut options)?;
                        }
                        "block-size" => {
                            let val = value.ok_or_else(|| "option '--block-size' requires an argument".to_string())?;
                            set_block_size(val, &mut options)?;
                        }
                        "si" => options.block_size = 1000,
                        "zero" => options.format = Format::One,
                        // No-op or consume-only options
                        "author" | "dired" | "full-time" | "hide" | "ignore" | "literal"
                        | "quote-name" | "quoting-style" | "show-control-chars" | "tabsize"
                        | "width" | "context" | "dereference-command-line"
                        | "dereference-command-line-symlink-to-dir" => {}
                        _ => return Err(format!("unrecognized option '--{}'", l)),
                    }
                } else if let Some(s) = short {
                    match s {
                        'a' => options.all = true,
                        'A' => options.almost_all = true,
                        'B' => options.ignore_backups = true,
                        'c' => options.time = TimeField::Changed,
                        'C' | 'x' => options.format = Format::Columns,
                        'd' => options.directory = true,
                        'f' | 'U' => {
                            options.all = true;
                            options.sort = Sort::None;
                        }
                        'F' => {
                            if value.map(|v| v.as_bytes()) != Some(b"never") {
                                options.indicator = Indicator::Classify;
                            }
                        }
                        'g' => {
                            options.format = Format::Long;
                            options.omit_owner = true;
                        }
                        'G' => options.omit_group = true,
                        'h' => options.human_readable = true,
                        'H' => {}
                        'i' => options.inode = true,
                        'k' => options.block_size = 1024,
                        'l' => options.format = Format::Long,
                        'L' => options.dereference = true,
                        'm' => options.format = Format::Commas,
                        'n' => {
                            options.numeric = true;
                            options.format = Format::Long;
                        }
                        'N' | 'q' | 'Q' | 'w' | 'Z' => {}
                        'o' => {
                            options.format = Format::Long;
                            options.omit_group = true;
                        }
                        'p' => options.indicator = Indicator::Slash,
                        'r' => options.reverse = true,
                        'R' => options.recursive = true,
                        's' => options.size = true,
                        'S' => options.sort = Sort::Size,
                        't' => options.sort = Sort::Time,
                        'u' => options.time = TimeField::Accessed,
                        'v' => options.sort = Sort::Name,
                        'X' => options.sort = Sort::Extension,
                        '1' => options.format = Format::One,
                        '-' => return Err("use '--' to end option parsing".to_string()),
                        _ => return Err(format!("invalid option -- '{}'", s)),
                    }
                }
            }
            ParsedArg::Operand(op) => {
                operands.push(OsString::from(op));
            }
        }
    }

    Ok(ParseResult {
        options,
        operands,
        help,
        version,
    })
}

fn list(
    options: Options,
    mut operands: Vec<OsString>,
    cwd: &Path,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> i32 {
    if operands.is_empty() {
        operands.push(OsString::from("."));
    }

    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut exit_code = 0;

    for operand in operands {
        let path = resolve(cwd, &operand);
        let display_path = PathBuf::from(&operand);
        match metadata_for(&path, options.dereference) {
            Ok(metadata) if metadata.is_dir() && !options.directory => dirs.push(Entry {
                display_name: operand,
                display_path,
                path,
                metadata,
            }),
            Ok(metadata) => files.push(Entry {
                display_name: operand,
                display_path,
                path,
                metadata,
            }),
            Err(error) => {
                let _ = writeln!(
                    stderr,
                    "ls: cannot access '{}': {error}",
                    os_lossy(&operand)
                );
                exit_code = 2;
            }
        }
    }

    sort_entries(&mut files, &options);
    sort_entries(&mut dirs, &options);

    if !files.is_empty() {
        write_entries(&files, &options, stdout);
        if !dirs.is_empty() {
            let _ = writeln!(stdout);
        }
    }

    for (index, dir) in dirs.iter().enumerate() {
        if dirs.len() > 1 || !files.is_empty() || options.recursive {
            let _ = writeln!(stdout, "{}:", os_lossy(&dir.display_name));
        }
        if list_dir(dir, &options, stdout, stderr) == 1 {
            exit_code = exit_code.max(1);
        }
        if index + 1 < dirs.len() {
            let _ = writeln!(stdout);
        }
    }

    exit_code
}

fn list_dir(
    dir: &Entry,
    options: &Options,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> i32 {
    let mut exit_code = 0;
    let Some(entries) = read_dir_entries(dir, options, stderr, &mut exit_code) else {
        return 1;
    };

    write_entries(&entries, options, stdout);

    if options.recursive {
        let mut visited = std::collections::HashSet::new();
        visited.insert((dir.metadata.dev(), dir.metadata.ino()));

        let mut pending = directory_entries(entries);
        pending.reverse();

        while let Some(child) = pending.pop() {
            let dev_ino = (child.metadata.dev(), child.metadata.ino());
            if !visited.insert(dev_ino) {
                let _ = writeln!(
                    stderr,
                    "ls: directory '{}' is part of a loop",
                    child.display_path.display()
                );
                exit_code = exit_code.max(1);
                continue;
            }

            let _ = writeln!(stdout);
            let _ = writeln!(stdout, "{}:", child.display_path.display());

            let Some(child_entries) = read_dir_entries(&child, options, stderr, &mut exit_code)
            else {
                continue;
            };
            write_entries(&child_entries, options, stdout);

            let mut child_dirs = directory_entries(child_entries);
            child_dirs.reverse();
            pending.extend(child_dirs);
        }
    }

    exit_code
}

fn read_dir_entries(
    dir: &Entry,
    options: &Options,
    stderr: &mut impl Write,
    exit_code: &mut i32,
) -> Option<Vec<Entry>> {
    let mut entries = Vec::new();
    if options.all {
        push_dot_entry(&mut entries, dir, ".");
        push_dot_entry(&mut entries, dir, "..");
    }

    match fs::read_dir(&dir.path) {
        Ok(read_dir) => {
            for child in read_dir.flatten() {
                let name = child.file_name();
                if should_skip(&name, options) {
                    continue;
                }
                let path = child.path();
                let display_path = if dir.display_path == Path::new(".") {
                    Path::new(".").join(&name)
                } else {
                    dir.display_path.join(&name)
                };
                match metadata_for(&path, options.dereference) {
                    Ok(metadata) => entries.push(Entry {
                        display_name: name,
                        display_path,
                        path,
                        metadata,
                    }),
                    Err(error) => {
                        let _ = writeln!(
                            stderr,
                            "ls: cannot access '{}': {error}",
                            os_lossy(&child.file_name())
                        );
                        *exit_code = 1;
                    }
                }
            }
        }
        Err(error) => {
            let _ = writeln!(
                stderr,
                "ls: cannot open directory '{}': {error}",
                dir.path.display()
            );
            *exit_code = 1;
            return None;
        }
    }

    sort_entries(&mut entries, options);
    Some(entries)
}

fn directory_entries(entries: Vec<Entry>) -> Vec<Entry> {
    entries
        .into_iter()
        .filter(|entry| {
            entry.metadata.is_dir()
                && entry.display_name.as_bytes() != b"."
                && entry.display_name.as_bytes() != b".."
        })
        .collect()
}

fn write_entries(entries: &[Entry], options: &Options, stdout: &mut impl Write) {
    match options.format {
        Format::One | Format::Long => {
            if options.format == Format::Long && !entries.is_empty() {
                let total: u64 = entries.iter().map(|entry| blocks(entry, options)).sum();
                let _ = writeln!(stdout, "total {total}");
            }
            for entry in entries {
                write_entry_line(entry, options, stdout);
                let _ = writeln!(stdout);
            }
        }
        Format::Columns => {
            for (index, entry) in entries.iter().enumerate() {
                if index > 0 {
                    let _ = stdout.write_all(b"  ");
                }
                write_name(entry, options, stdout);
            }
            if !entries.is_empty() {
                let _ = writeln!(stdout);
            }
        }
        Format::Commas => {
            for (index, entry) in entries.iter().enumerate() {
                if index > 0 {
                    let _ = stdout.write_all(b", ");
                }
                write_name(entry, options, stdout);
            }
            if !entries.is_empty() {
                let _ = writeln!(stdout);
            }
        }
    }
}

fn write_entry_line(entry: &Entry, options: &Options, stdout: &mut impl Write) {
    if options.inode {
        let _ = write!(stdout, "{} ", entry.metadata.ino());
    }
    if options.size {
        let _ = write!(stdout, "{} ", display_blocks(entry, options));
    }
    if options.format == Format::Long {
        write_long(entry, options, stdout);
    }
    write_name(entry, options, stdout);
}

fn write_long(entry: &Entry, options: &Options, stdout: &mut impl Write) {
    let mode = mode_string(&entry.metadata);
    let links = entry.metadata.nlink();
    let size = display_size(entry.metadata.len(), options);
    let time = display_time(entry, options);
    let _ = write!(stdout, "{mode} {links:>2} ");
    if !options.omit_owner {
        let _ = write!(stdout, "{} ", entry.metadata.uid());
    }
    if !options.omit_group {
        let _ = write!(stdout, "{} ", entry.metadata.gid());
    }
    let _ = write!(stdout, "{size:>5} {time} ");
}

fn write_name(entry: &Entry, options: &Options, stdout: &mut impl Write) {
    let _ = stdout.write_all(entry.display_name.as_bytes());
    if let Some(indicator) = indicator(entry, options.indicator) {
        let _ = stdout.write_all(&[indicator]);
    }
}

fn sort_entries(entries: &mut [Entry], options: &Options) {
    entries.sort_by(|left, right| compare_entries(left, right, options));
}

fn compare_entries(left: &Entry, right: &Entry, options: &Options) -> Ordering {
    if options.group_dirs_first {
        match (left.metadata.is_dir(), right.metadata.is_dir()) {
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            _ => {}
        }
    }
    let order = match options.sort {
        Sort::Name => compare_os(&left.display_name, &right.display_name),
        Sort::None => Ordering::Equal,
        Sort::Size => right
            .metadata
            .len()
            .cmp(&left.metadata.len())
            .then_with(|| compare_os(&left.display_name, &right.display_name)),
        Sort::Time => entry_time(right, options)
            .cmp(&entry_time(left, options))
            .then_with(|| compare_os(&left.display_name, &right.display_name)),
        Sort::Extension => extension(&left.display_name)
            .cmp(extension(&right.display_name))
            .then_with(|| compare_os(&left.display_name, &right.display_name)),
    };
    if options.reverse {
        order.reverse()
    } else {
        order
    }
}

fn should_skip(name: &OsStr, options: &Options) -> bool {
    let bytes = name.as_bytes();
    (!options.all && !options.almost_all && bytes.starts_with(b"."))
        || (options.almost_all && (bytes == b"." || bytes == b".."))
        || (options.ignore_backups && bytes.ends_with(b"~"))
}

fn push_dot_entry(entries: &mut Vec<Entry>, dir: &Entry, name: &str) {
    let path = if name == "." {
        dir.path.clone()
    } else {
        dir.path.parent().unwrap_or(&dir.path).to_path_buf()
    };
    let display_path = if name == "." {
        dir.display_path.clone()
    } else {
        dir.display_path.parent().unwrap_or(&dir.display_path).to_path_buf()
    };
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        entries.push(Entry {
            display_name: OsString::from(name),
            display_path,
            path,
            metadata,
        });
    }
}

fn metadata_for(path: &Path, dereference: bool) -> std::io::Result<Metadata> {
    if dereference {
        fs::metadata(path)
    } else {
        fs::symlink_metadata(path)
    }
}

fn resolve(cwd: &Path, operand: &OsStr) -> PathBuf {
    let path = PathBuf::from(operand);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn indicator(entry: &Entry, style: Indicator) -> Option<u8> {
    match style {
        Indicator::None => None,
        Indicator::Slash => entry.metadata.is_dir().then_some(b'/'),
        Indicator::FileType | Indicator::Classify => {
            let file_type = entry.metadata.file_type();
            if file_type.is_dir() {
                Some(b'/')
            } else if file_type.is_symlink() {
                Some(b'@')
            } else if file_type.is_fifo() {
                Some(b'|')
            } else if file_type.is_socket() {
                Some(b'=')
            } else if style == Indicator::Classify
                && entry.metadata.permissions().mode() & 0o111 != 0
            {
                Some(b'*')
            } else {
                None
            }
        }
    }
}

fn mode_string(metadata: &Metadata) -> String {
    let file_type = metadata.file_type();
    let kind = if file_type.is_dir() {
        'd'
    } else if file_type.is_symlink() {
        'l'
    } else if file_type.is_fifo() {
        'p'
    } else if file_type.is_socket() {
        's'
    } else if file_type.is_block_device() {
        'b'
    } else if file_type.is_char_device() {
        'c'
    } else {
        '-'
    };
    let mode = metadata.permissions().mode();
    let mut out = String::with_capacity(10);
    out.push(kind);
    out.push(if mode & 0o400 != 0 { 'r' } else { '-' });
    out.push(if mode & 0o200 != 0 { 'w' } else { '-' });
    out.push(exec_char(mode, 0o100, 0o4000, 's', 'S'));
    out.push(if mode & 0o040 != 0 { 'r' } else { '-' });
    out.push(if mode & 0o020 != 0 { 'w' } else { '-' });
    out.push(exec_char(mode, 0o010, 0o2000, 's', 'S'));
    out.push(if mode & 0o004 != 0 { 'r' } else { '-' });
    out.push(if mode & 0o002 != 0 { 'w' } else { '-' });
    out.push(exec_char(mode, 0o001, 0o1000, 't', 'T'));
    out
}

fn exec_char(
    mode: u32,
    exec_bit: u32,
    special_bit: u32,
    special_exec: char,
    special_no_exec: char,
) -> char {
    match (mode & exec_bit != 0, mode & special_bit != 0) {
        (true, true) => special_exec,
        (false, true) => special_no_exec,
        (true, false) => 'x',
        (false, false) => '-',
    }
}

fn blocks(entry: &Entry, options: &Options) -> u64 {
    let block_bytes = options.block_size.max(1);
    let bytes = entry.metadata.blocks().saturating_mul(512);
    bytes.div_ceil(block_bytes)
}

fn display_blocks(entry: &Entry, options: &Options) -> String {
    if options.human_readable {
        humanize(blocks(entry, options), 1024)
    } else {
        blocks(entry, options).to_string()
    }
}

fn display_size(size: u64, options: &Options) -> String {
    if options.human_readable {
        humanize(size, 1024)
    } else {
        size.to_string()
    }
}

fn humanize(value: u64, base: u64) -> String {
    let units = ["", "K", "M", "G", "T", "P", "E"];
    let mut scaled = value as f64;
    let mut unit = 0;
    while scaled >= base as f64 && unit + 1 < units.len() {
        scaled /= base as f64;
        unit += 1;
    }
    if unit == 0 {
        value.to_string()
    } else if scaled < 10.0 {
        format!("{scaled:.1}{}", units[unit])
    } else {
        format!("{scaled:.0}{}", units[unit])
    }
}

fn display_time(entry: &Entry, options: &Options) -> String {
    match entry_time(entry, options).duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(error) => format!("-{}", error.duration().as_secs()),
    }
}

fn entry_time(entry: &Entry, options: &Options) -> SystemTime {
    match options.time {
        TimeField::Modified => entry.metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        TimeField::Accessed => entry.metadata.accessed().unwrap_or(SystemTime::UNIX_EPOCH),
        TimeField::Changed => {
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(entry.metadata.ctime() as u64)
        }
    }
}

fn extension(name: &OsStr) -> &[u8] {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return b"";
    }
    let search_bytes = if bytes[0] == b'.' { &bytes[1..] } else { bytes };
    match search_bytes.iter().rposition(|&byte| byte == b'.') {
        Some(index) if index + 1 < search_bytes.len() => &search_bytes[index + 1..],
        _ => b"",
    }
}

fn compare_os(left: &OsStr, right: &OsStr) -> Ordering {
    left.as_bytes().cmp(right.as_bytes())
}



fn set_sort(value: &OsStr, options: &mut Options) -> Result<(), String> {
    match value.as_bytes() {
        b"none" => options.sort = Sort::None,
        b"size" => options.sort = Sort::Size,
        b"time" => options.sort = Sort::Time,
        b"extension" => options.sort = Sort::Extension,
        b"name" | b"width" | b"version" => options.sort = Sort::Name,
        other => return Err(format!("invalid sort: {}", bytes_lossy(other))),
    }
    Ok(())
}

fn set_time(value: &OsStr, options: &mut Options) -> Result<(), String> {
    match value.as_bytes() {
        b"atime" | b"access" | b"use" => options.time = TimeField::Accessed,
        b"ctime" | b"status" => options.time = TimeField::Changed,
        b"mtime" | b"modification" => options.time = TimeField::Modified,
        b"birth" | b"creation" => options.time = TimeField::Modified,
        other => return Err(format!("invalid time: {}", bytes_lossy(other))),
    }
    Ok(())
}

fn set_block_size(value: &OsStr, options: &mut Options) -> Result<(), String> {
    let bytes = value.as_bytes();
    options.block_size = match bytes {
        b"K" | b"KiB" => 1024,
        b"KB" => 1000,
        b"M" | b"MiB" => 1024 * 1024,
        b"MB" => 1000 * 1000,
        digits if digits.iter().all(u8::is_ascii_digit) => {
            bytes_lossy(digits).parse().unwrap_or(1024)
        }
        other => return Err(format!("invalid block size: {}", bytes_lossy(other))),
    };
    Ok(())
}

fn os_lossy(value: &OsStr) -> String {
    String::from_utf8_lossy(value.as_bytes()).into_owned()
}

fn bytes_lossy(value: &[u8]) -> String {
    String::from_utf8_lossy(value).into_owned()
}
