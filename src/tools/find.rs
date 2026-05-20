//! GNU-style `find`.
//!
//! This module implements the core logic of the `find` command, including
//! a recursive descent parser for expressions and directory traversal.
//! 
//! [find.rs](file:///Users/spencergreene/github/rust-unix-tools/src/tools/find.rs)

use regex::{Regex, RegexBuilder};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

#[derive(Clone)]
pub enum ExprNode {
    And(Vec<ExprNode>),
    Or(Vec<ExprNode>),
    Not(Box<ExprNode>),
    Comma(Vec<ExprNode>),
    // Tests
    Name { regex: Regex },
    Path { regex: Regex },
    Type { file_type: char },
    Size { compare: char, size_val: u64, unit: u64 },
    MTime { compare: char, days: i64 },
    ATime { compare: char, days: i64 },
    CTime { compare: char, days: i64 },
    Perm { mode_type: char, mode: u32 },
    Newer { reference_mtime: SystemTime },
    User { uid: u32 },
    Group { gid: u32 },
    // Actions
    Print,
    Print0,
    Delete,
    Prune,
    Exec { command: Vec<String> },
    // Dummy node for options that always return true
    TrueNode,
}

impl ExprNode {
    fn has_action(&self) -> bool {
        match self {
            ExprNode::And(children) | ExprNode::Or(children) | ExprNode::Comma(children) => {
                children.iter().any(|c| c.has_action())
            }
            ExprNode::Not(child) => child.has_action(),
            ExprNode::Print | ExprNode::Print0 | ExprNode::Delete | ExprNode::Exec { .. } => true,
            _ => false,
        }
    }

    fn eval(&self, state: &mut EvalState) -> bool {
        match self {
            ExprNode::And(children) => {
                for child in children {
                    if !child.eval(state) {
                        return false;
                    }
                }
                true
            }
            ExprNode::Or(children) => {
                for child in children {
                    if child.eval(state) {
                        return true;
                    }
                }
                false
            }
            ExprNode::Not(child) => !child.eval(state),
            ExprNode::Comma(children) => {
                let mut last = false;
                for child in children {
                    last = child.eval(state);
                }
                last
            }
            ExprNode::Name { regex } => {
                let name = state.path.file_name().unwrap_or_else(|| state.path.as_os_str());
                regex.is_match(&name.to_string_lossy())
            }
            ExprNode::Path { regex } => {
                regex.is_match(&state.display_path.to_string_lossy())
            }
            ExprNode::Type { file_type } => {
                let m = state.metadata;
                match *file_type {
                    'b' => m.file_type().is_block_device(),
                    'c' => m.file_type().is_char_device(),
                    'd' => m.is_dir(),
                    'p' => m.file_type().is_fifo(),
                    'f' => m.is_file(),
                    'l' => m.file_type().is_symlink(),
                    's' => m.file_type().is_socket(),
                    _ => false,
                }
            }
            ExprNode::Size { compare, size_val, unit } => {
                let bytes = state.metadata.len();
                if *unit == 512 {
                    // Block size rounding
                    let blocks = if bytes == 0 {
                        0
                    } else {
                        (bytes - 1) / 512 + 1
                    };
                    match *compare {
                        '+' => blocks > *size_val,
                        '-' => blocks < *size_val,
                        '=' => blocks == *size_val,
                        _ => false,
                    }
                } else {
                    // Direct byte comparison
                    let target_bytes = size_val * unit;
                    match *compare {
                        '+' => bytes > target_bytes,
                        '-' => bytes < target_bytes,
                        '=' => bytes == target_bytes,
                        _ => false,
                    }
                }
            }
            ExprNode::MTime { compare, days } => {
                if let Ok(modified) = state.metadata.modified() {
                    check_time(modified, *compare, *days)
                } else {
                    false
                }
            }
            ExprNode::ATime { compare, days } => {
                if let Ok(accessed) = state.metadata.accessed() {
                    check_time(accessed, *compare, *days)
                } else {
                    false
                }
            }
            ExprNode::CTime { compare, days } => {
                let ctime_sec = state.metadata.ctime();
                let ctime_time = SystemTime::UNIX_EPOCH + Duration::from_secs(ctime_sec as u64);
                check_time(ctime_time, *compare, *days)
            }
            ExprNode::Perm { mode_type, mode } => {
                let file_mode = state.metadata.permissions().mode() & 0o7777;
                match *mode_type {
                    '=' => file_mode == *mode,
                    '-' => (file_mode & *mode) == *mode,
                    '/' => (file_mode & *mode) != 0,
                    _ => false,
                }
            }
            ExprNode::Newer { reference_mtime } => {
                if let Ok(modified) = state.metadata.modified() {
                    modified > *reference_mtime
                } else {
                    false
                }
            }
            ExprNode::User { uid } => state.metadata.uid() == *uid,
            ExprNode::Group { gid } => state.metadata.gid() == *gid,
            ExprNode::Print => {
                let _ = writeln!(state.stdout, "{}", state.display_path.to_string_lossy());
                true
            }
            ExprNode::Print0 => {
                let _ = write!(state.stdout, "{}\0", state.display_path.to_string_lossy());
                true
            }
            ExprNode::Delete => {
                if state.metadata.is_dir() {
                    if let Err(e) = fs::remove_dir(state.path) {
                        let _ = writeln!(state.stderr, "find: cannot delete '{}': {}", state.display_path.to_string_lossy(), e);
                        false
                    } else {
                        true
                    }
                } else {
                    if let Err(e) = fs::remove_file(state.path) {
                        let _ = writeln!(state.stderr, "find: cannot delete '{}': {}", state.display_path.to_string_lossy(), e);
                        false
                    } else {
                        true
                    }
                }
            }
            ExprNode::Prune => {
                state.pruned = true;
                true
            }
            ExprNode::Exec { command } => {
                let mut cmd_args = Vec::new();
                for arg in command {
                    if arg == "{}" {
                        cmd_args.push(state.display_path.to_string_lossy().into_owned());
                    } else {
                        cmd_args.push(arg.clone());
                    }
                }
                if cmd_args.is_empty() {
                    return false;
                }
                let mut child = Command::new(&cmd_args[0]);
                child.args(&cmd_args[1..]).current_dir(state.cwd);
                child.stdout(std::process::Stdio::piped());
                child.stderr(std::process::Stdio::piped());
                match child.output() {
                    Ok(output) => {
                        let _ = state.stdout.write_all(&output.stdout);
                        let _ = state.stderr.write_all(&output.stderr);
                        output.status.success()
                    }
                    Err(e) => {
                        let _ = writeln!(state.stderr, "find: exec failed: {}", e);
                        false
                    }
                }
            }
            ExprNode::TrueNode => true,
        }
    }
}

fn check_time(t: SystemTime, compare: char, days: i64) -> bool {
    let now = SystemTime::now();
    let diff = match now.duration_since(t) {
        Ok(d) => d.as_secs(),
        Err(_) => 0,
    };
    let file_days = (diff / (24 * 3600)) as i64;
    match compare {
        '+' => file_days > days,
        '-' => file_days < days,
        '=' => file_days == days,
        _ => false,
    }
}

struct EvalState<'a> {
    path: &'a Path,
    display_path: &'a Path,
    metadata: &'a fs::Metadata,
    pruned: bool,
    stdout: &'a mut dyn Write,
    stderr: &'a mut dyn Write,
    cwd: &'a Path,
}

struct TraversalOptions {
    maxdepth: Option<usize>,
    mindepth: Option<usize>,
    depth_first: bool,
    mount: bool,
    follow_symlinks: bool,
}

struct Parser<'a> {
    args: &'a [OsString],
    index: usize,
    cwd: &'a Path,
    options: &'a mut TraversalOptions,
}

impl<'a> Parser<'a> {
    fn new(args: &'a [OsString], cwd: &'a Path, options: &'a mut TraversalOptions) -> Self {
        Self {
            args,
            index: 0,
            cwd,
            options,
        }
    }

    fn peek(&self) -> Option<&OsStr> {
        if self.index < self.args.len() {
            Some(&self.args[self.index])
        } else {
            None
        }
    }

    fn next(&mut self) -> Option<&OsStr> {
        if self.index < self.args.len() {
            let val = &self.args[self.index];
            self.index += 1;
            Some(val)
        } else {
            None
        }
    }

    fn parse_expression(&mut self) -> Result<ExprNode, String> {
        self.parse_comma()
    }

    fn parse_comma(&mut self) -> Result<ExprNode, String> {
        let mut nodes = vec![self.parse_or()?];
        while self.peek() == Some(OsStr::new(",")) {
            self.next();
            nodes.push(self.parse_or()?);
        }
        if nodes.len() == 1 {
            Ok(nodes.remove(0))
        } else {
            Ok(ExprNode::Comma(nodes))
        }
    }

    fn parse_or(&mut self) -> Result<ExprNode, String> {
        let mut nodes = vec![self.parse_and()?];
        while let Some(peeked) = self.peek() {
            if peeked == OsStr::new("-o") || peeked == OsStr::new("-or") {
                self.next();
                nodes.push(self.parse_and()?);
            } else {
                break;
            }
        }
        if nodes.len() == 1 {
            Ok(nodes.remove(0))
        } else {
            Ok(ExprNode::Or(nodes))
        }
    }

    fn parse_and(&mut self) -> Result<ExprNode, String> {
        let mut nodes = vec![self.parse_not()?];
        while let Some(peeked) = self.peek() {
            if peeked == OsStr::new(")") || peeked == OsStr::new(",") || peeked == OsStr::new("-o") || peeked == OsStr::new("-or") {
                break;
            }
            if peeked == OsStr::new("-a") || peeked == OsStr::new("-and") {
                self.next();
            }
            nodes.push(self.parse_not()?);
        }
        if nodes.len() == 1 {
            Ok(nodes.remove(0))
        } else {
            Ok(ExprNode::And(nodes))
        }
    }

    fn parse_not(&mut self) -> Result<ExprNode, String> {
        if let Some(peeked) = self.peek() {
            if peeked == OsStr::new("!") || peeked == OsStr::new("-not") {
                self.next();
                let child = self.parse_not()?;
                return Ok(ExprNode::Not(Box::new(child)));
            }
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<ExprNode, String> {
        let arg = match self.next() {
            Some(a) => a.to_string_lossy().into_owned(),
            None => return Err("Unexpected end of expression".to_string()),
        };

        if arg == "(" {
            let inner = self.parse_expression()?;
            if self.next() != Some(OsStr::new(")")) {
                return Err("Expected ')'".to_string());
            }
            return Ok(inner);
        }

        match arg.as_str() {
            "-maxdepth" => {
                let val = self.next_arg_string("-maxdepth")?;
                if let Ok(n) = val.parse::<usize>() {
                    self.options.maxdepth = Some(n);
                    Ok(ExprNode::TrueNode)
                } else {
                    Err(format!("invalid maxdepth '{}'", val))
                }
            }
            "-mindepth" => {
                let val = self.next_arg_string("-mindepth")?;
                if let Ok(n) = val.parse::<usize>() {
                    self.options.mindepth = Some(n);
                    Ok(ExprNode::TrueNode)
                } else {
                    Err(format!("invalid mindepth '{}'", val))
                }
            }
            "-depth" => {
                self.options.depth_first = true;
                Ok(ExprNode::TrueNode)
            }
            "-mount" | "-xdev" => {
                self.options.mount = true;
                Ok(ExprNode::TrueNode)
            }
            "-name" => {
                let pattern = self.next_arg_string("-name")?;
                let regex = compile_glob(&pattern, false)?;
                Ok(ExprNode::Name { regex })
            }
            "-iname" => {
                let pattern = self.next_arg_string("-iname")?;
                let regex = compile_glob(&pattern, true)?;
                Ok(ExprNode::Name { regex })
            }
            "-path" | "-wholename" => {
                let pattern = self.next_arg_string("-path")?;
                let regex = compile_glob(&pattern, false)?;
                Ok(ExprNode::Path { regex })
            }
            "-ipath" | "-iwholename" => {
                let pattern = self.next_arg_string("-ipath")?;
                let regex = compile_glob(&pattern, true)?;
                Ok(ExprNode::Path { regex })
            }
            "-type" => {
                let val = self.next_arg_string("-type")?;
                if val.len() == 1 {
                    let c = val.chars().next().unwrap();
                    if "bcdpfls".contains(c) {
                        Ok(ExprNode::Type { file_type: c })
                    } else {
                        Err(format!("unknown file type '{}'", val))
                    }
                } else {
                    Err(format!("invalid file type '{}'", val))
                }
            }
            "-size" => {
                let val = self.next_arg_string("-size")?;
                let (compare, rest) = if val.starts_with('+') {
                    ('+', &val[1..])
                } else if val.starts_with('-') {
                    ('-', &val[1..])
                } else {
                    ('=', &val[..])
                };

                let mut digit_len = 0;
                for c in rest.chars() {
                    if c.is_ascii_digit() {
                        digit_len += 1;
                    } else {
                        break;
                    }
                }

                if digit_len == 0 {
                    return Err(format!("invalid size '{}'", val));
                }

                let size_val: u64 = rest[..digit_len].parse().map_err(|_| format!("invalid size '{}'", val))?;
                let unit_char = rest[digit_len..].chars().next().unwrap_or('b');
                let unit = match unit_char {
                    'c' => 1,
                    'k' => 1024,
                    'M' => 1024 * 1024,
                    'G' => 1024 * 1024 * 1024,
                    'b' => 512,
                    _ => return Err(format!("unknown size unit '{}'", unit_char)),
                };

                Ok(ExprNode::Size { compare, size_val, unit })
            }
            "-mtime" => {
                let val = self.next_arg_string("-mtime")?;
                let (compare, days) = parse_time_arg(&val)?;
                Ok(ExprNode::MTime { compare, days })
            }
            "-atime" => {
                let val = self.next_arg_string("-atime")?;
                let (compare, days) = parse_time_arg(&val)?;
                Ok(ExprNode::ATime { compare, days })
            }
            "-ctime" => {
                let val = self.next_arg_string("-ctime")?;
                let (compare, days) = parse_time_arg(&val)?;
                Ok(ExprNode::CTime { compare, days })
            }
            "-perm" => {
                let val = self.next_arg_string("-perm")?;
                let (mode_type, rest) = if val.starts_with('-') {
                    ('-', &val[1..])
                } else if val.starts_with('/') {
                    ('/', &val[1..])
                } else {
                    ('=', &val[..])
                };

                let mode = u32::from_str_radix(rest, 8).map_err(|_| format!("invalid mode '{}'", val))?;
                Ok(ExprNode::Perm { mode_type, mode })
            }
            "-newer" => {
                let val = self.next_arg_string("-newer")?;
                let ref_path = self.cwd.join(&val);
                let meta = fs::metadata(ref_path).map_err(|e| format!("cannot stat '{}': {}", val, e))?;
                let reference_mtime = meta.modified().map_err(|e| format!("cannot get modified time: {}", e))?;
                Ok(ExprNode::Newer { reference_mtime })
            }
            "-user" => {
                let val = self.next_arg_string("-user")?;
                let uid = get_uid_by_name(&val).ok_or_else(|| format!("'{}' is not a valid user", val))?;
                Ok(ExprNode::User { uid })
            }
            "-group" => {
                let val = self.next_arg_string("-group")?;
                let gid = get_gid_by_name(&val).ok_or_else(|| format!("'{}' is not a valid group", val))?;
                Ok(ExprNode::Group { gid })
            }
            "-print" => Ok(ExprNode::Print),
            "-print0" => Ok(ExprNode::Print0),
            "-delete" => {
                self.options.depth_first = true;
                Ok(ExprNode::Delete)
            }
            "-prune" => Ok(ExprNode::Prune),
            "-exec" => {
                let mut cmd = Vec::new();
                loop {
                    match self.next() {
                        Some(arg) => {
                            let s = arg.to_string_lossy().into_owned();
                            if s == ";" {
                                break;
                            }
                            cmd.push(s);
                        }
                        None => return Err("find: missing argument to '-exec'".to_string()),
                    }
                }
                Ok(ExprNode::Exec { command: cmd })
            }
            _ => Err(format!("unknown predicate '{}'", arg)),
        }
    }

    fn next_arg_string(&mut self, option: &str) -> Result<String, String> {
        match self.next() {
            Some(a) => Ok(a.to_string_lossy().into_owned()),
            None => Err(format!("find: missing argument to '{}'", option)),
        }
    }
}

fn parse_time_arg(val: &str) -> Result<(char, i64), String> {
    let (compare, rest) = if val.starts_with('+') {
        ('+', &val[1..])
    } else if val.starts_with('-') {
        ('-', &val[1..])
    } else {
        ('=', &val[..])
    };
    let days: i64 = rest.parse().map_err(|_| format!("invalid time value '{}'", val))?;
    Ok((compare, days))
}

fn compile_glob(glob: &str, case_insensitive: bool) -> Result<Regex, String> {
    let mut regex = String::new();
    regex.push_str("^(?:");
    let mut chars = glob.chars().peekable();
    let mut in_char_class = false;
    while let Some(c) = chars.next() {
        match c {
            '*' if !in_char_class => regex.push_str(".*"),
            '?' if !in_char_class => regex.push('.'),
            '[' if !in_char_class => {
                in_char_class = true;
                regex.push('[');
                if chars.peek() == Some(&'!') {
                    regex.push('^');
                    chars.next();
                }
            }
            ']' if in_char_class => {
                in_char_class = false;
                regex.push(']');
            }
            c if in_char_class => {
                if c == '\\' || c == '-' || c == '^' || c == '[' {
                    regex.push('\\');
                }
                regex.push(c);
            }
            c if c.is_alphanumeric() => regex.push(c),
            c => {
                regex.push('\\');
                regex.push(c);
            }
        }
    }
    regex.push_str(")$");
    let mut builder = RegexBuilder::new(&regex);
    builder.case_insensitive(case_insensitive);
    builder.build().map_err(|e| format!("invalid glob '{}': {}", glob, e))
}

fn get_uid_by_name(name: &str) -> Option<u32> {
    if let Ok(uid) = name.parse::<u32>() {
        return Some(uid);
    }
    if let Ok(content) = std::fs::read_to_string("/etc/passwd") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 && parts[0] == name {
                if let Ok(uid) = parts[2].parse::<u32>() {
                    return Some(uid);
                }
            }
        }
    }
    None
}

fn get_gid_by_name(name: &str) -> Option<u32> {
    if let Ok(gid) = name.parse::<u32>() {
        return Some(gid);
    }
    if let Ok(content) = std::fs::read_to_string("/etc/group") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 && parts[0] == name {
                if let Ok(gid) = parts[2].parse::<u32>() {
                    return Some(gid);
                }
            }
        }
    }
    None
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
    let mut starting_points = Vec::new();
    let mut expression_args = Vec::new();

    let args_iter = args.into_iter();
    let mut follow_symlinks = false;
    let mut command_line_symlinks = false;

    let mut remaining = Vec::new();
    for arg in args_iter {
        remaining.push(arg);
    }

    let mut i = 0;
    while i < remaining.len() {
        let arg_str = remaining[i].to_string_lossy();
        if arg_str == "-H" {
            command_line_symlinks = true;
            follow_symlinks = false;
            i += 1;
        } else if arg_str == "-L" {
            follow_symlinks = true;
            command_line_symlinks = false;
            i += 1;
        } else if arg_str == "-P" {
            follow_symlinks = false;
            command_line_symlinks = false;
            i += 1;
        } else {
            break;
        }
    }

    while i < remaining.len() {
        let arg_str = remaining[i].to_string_lossy();
        if arg_str.starts_with('-') || arg_str == "!" || arg_str == "(" || arg_str == ")" || arg_str == "," {
            break;
        }
        starting_points.push(remaining[i].clone());
        i += 1;
    }

    while i < remaining.len() {
        expression_args.push(remaining[i].clone());
        i += 1;
    }

    if starting_points.is_empty() {
        starting_points.push(OsString::from("."));
    }

    let mut options = TraversalOptions {
        maxdepth: None,
        mindepth: None,
        depth_first: false,
        mount: false,
        follow_symlinks,
    };

    let expr = if expression_args.is_empty() {
        ExprNode::Print
    } else {
        let mut parser = Parser::new(&expression_args, cwd, &mut options);
        match parser.parse_expression() {
            Ok(node) => {
                if parser.peek().is_some() {
                    let _ = writeln!(stderr, "find: extra expression arguments");
                    return 1;
                }
                node
            }
            Err(e) => {
                let _ = writeln!(stderr, "find: {}", e);
                return 1;
            }
        }
    };

    let has_action = expr.has_action();
    let mut exit_code = 0;

    for start in starting_points {
        let path = cwd.join(&start);
        let metadata_res = if follow_symlinks || command_line_symlinks {
            fs::metadata(&path)
        } else {
            fs::symlink_metadata(&path)
        };

        let metadata = match metadata_res {
            Ok(m) => m,
            Err(e) => {
                let _ = writeln!(stderr, "find: '{}': {}", start.to_string_lossy(), e);
                exit_code = 1;
                continue;
            }
        };

        let mut visited = HashSet::new();
        let dev_id = metadata.dev();

        traverse(
            &path,
            Path::new(&start),
            0,
            dev_id,
            &options,
            &expr,
            has_action,
            &mut visited,
            &mut exit_code,
            stdout,
            stderr,
            cwd,
            Some(&metadata),
        );
    }

    exit_code
}

fn traverse(
    path: &Path,
    display_path: &Path,
    depth: usize,
    start_dev: u64,
    options: &TraversalOptions,
    expr: &ExprNode,
    has_action: bool,
    visited: &mut HashSet<PathBuf>,
    exit_code: &mut i32,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    cwd: &Path,
    starting_metadata: Option<&fs::Metadata>,
) {
    if let Some(max) = options.maxdepth {
        if depth > max {
            return;
        }
    }

    let metadata = match starting_metadata {
        Some(m) => m.clone(),
        None => {
            let follow = options.follow_symlinks;
            let metadata_res = if follow {
                fs::metadata(path)
            } else {
                fs::symlink_metadata(path)
            };
            match metadata_res {
                Ok(m) => m,
                Err(e) => {
                    let _ = writeln!(stderr, "find: '{}': {}", display_path.to_string_lossy(), e);
                    *exit_code = 1;
                    return;
                }
            }
        }
    };

    if options.mount && metadata.dev() != start_dev {
        return;
    }

    let is_dir = metadata.is_dir();
    let evaluate_here = options.mindepth.map_or(true, |min| depth >= min);
    let mut pruned = false;

    // Pre-order evaluation
    if !options.depth_first && evaluate_here {
        let mut eval_state = EvalState {
            path,
            display_path,
            metadata: &metadata,
            pruned: false,
            stdout,
            stderr,
            cwd,
        };
        let matched = expr.eval(&mut eval_state);
        pruned = eval_state.pruned;
        if !has_action && matched {
            let _ = writeln!(stdout, "{}", display_path.to_string_lossy());
        }
    }

    // Loop detection for directories
    if is_dir && !pruned {
        let canonical = match fs::canonicalize(path) {
            Ok(c) => c,
            Err(e) => {
                let _ = writeln!(stderr, "find: '{}': {}", display_path.to_string_lossy(), e);
                *exit_code = 1;
                return;
            }
        };

        if !visited.insert(canonical.clone()) {
            let _ = writeln!(stderr, "find: File system loop detected; '{}' is part of the same file system loop.", display_path.to_string_lossy());
            *exit_code = 1;
            return;
        }

        let entries = match fs::read_dir(path) {
            Ok(read) => {
                let mut list = Vec::new();
                for entry in read {
                    match entry {
                        Ok(e) => list.push(e),
                        Err(err) => {
                            let _ = writeln!(stderr, "find: '{}': {}", display_path.to_string_lossy(), err);
                            *exit_code = 1;
                        }
                    }
                }
                list.sort_by_key(|e| e.file_name());
                list
            }
            Err(e) => {
                let _ = writeln!(stderr, "find: '{}': {}", display_path.to_string_lossy(), e);
                *exit_code = 1;
                Vec::new()
            }
        };

        for entry in entries {
            let entry_path = entry.path();
            let entry_display = display_path.join(entry.file_name());
            traverse(
                &entry_path,
                &entry_display,
                depth + 1,
                start_dev,
                options,
                expr,
                has_action,
                visited,
                exit_code,
                stdout,
                stderr,
                cwd,
                None,
            );
        }

        visited.remove(&canonical);
    }

    // Post-order evaluation
    if options.depth_first && evaluate_here {
        let mut eval_state = EvalState {
            path,
            display_path,
            metadata: &metadata,
            pruned: false,
            stdout,
            stderr,
            cwd,
        };
        let matched = expr.eval(&mut eval_state);
        if !has_action && matched {
            let _ = writeln!(stdout, "{}", display_path.to_string_lossy());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    struct FindTempFixture {
        root: PathBuf,
    }

    impl FindTempFixture {
        fn new(name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!("find-test-{}-{}-{}", name, std::process::id(), nanos));
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn file(&self, name: &str, size: usize) -> PathBuf {
            let p = self.root.join(name);
            let content = vec![0u8; size];
            fs::write(&p, content).unwrap();
            p
        }

        fn dir(&self, name: &str) -> PathBuf {
            let p = self.root.join(name);
            fs::create_dir_all(&p).unwrap();
            p
        }

        fn symlink(&self, target: &str, link_name: &str) -> PathBuf {
            let p = self.root.join(link_name);
            let _ = fs::remove_file(&p);
            std::os::unix::fs::symlink(target, &p).unwrap();
            p
        }
    }

    impl Drop for FindTempFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn test_has_action() {
        assert!(ExprNode::Print.has_action());
        assert!(ExprNode::Print0.has_action());
        assert!(ExprNode::Delete.has_action());
        assert!(ExprNode::Exec { command: vec![] }.has_action());
        assert!(!ExprNode::TrueNode.has_action());

        let and_act = ExprNode::And(vec![ExprNode::TrueNode, ExprNode::Print]);
        assert!(and_act.has_action());

        let and_no_act = ExprNode::And(vec![ExprNode::TrueNode, ExprNode::TrueNode]);
        assert!(!and_no_act.has_action());

        let or_act = ExprNode::Or(vec![ExprNode::Print, ExprNode::TrueNode]);
        assert!(or_act.has_action());

        let not_act = ExprNode::Not(Box::new(ExprNode::Print));
        assert!(not_act.has_action());

        let comma_act = ExprNode::Comma(vec![ExprNode::TrueNode, ExprNode::Print]);
        assert!(comma_act.has_action());
    }

    #[test]
    fn test_evaluate_operators() {
        let fix = FindTempFixture::new("operators");
        let f_path = fix.file("a.txt", 10);
        let meta = fs::metadata(&f_path).unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut state = EvalState {
            path: &f_path,
            display_path: &f_path,
            metadata: &meta,
            pruned: false,
            stdout: &mut stdout,
            stderr: &mut stderr,
            cwd: &fix.root,
        };

        // TrueNode
        assert!(ExprNode::TrueNode.eval(&mut state));

        // Not operator
        let not_node = ExprNode::Not(Box::new(ExprNode::TrueNode));
        assert!(!not_node.eval(&mut state));

        // And operator
        let and_node = ExprNode::And(vec![ExprNode::TrueNode, ExprNode::TrueNode]);
        assert!(and_node.eval(&mut state));
        let and_node_false = ExprNode::And(vec![ExprNode::TrueNode, ExprNode::Not(Box::new(ExprNode::TrueNode))]);
        assert!(!and_node_false.eval(&mut state));

        // Or operator
        let or_node = ExprNode::Or(vec![ExprNode::Not(Box::new(ExprNode::TrueNode)), ExprNode::TrueNode]);
        assert!(or_node.eval(&mut state));
        let or_node_false = ExprNode::Or(vec![ExprNode::Not(Box::new(ExprNode::TrueNode)), ExprNode::Not(Box::new(ExprNode::TrueNode))]);
        assert!(!or_node_false.eval(&mut state));

        // Comma operator
        let comma_node = ExprNode::Comma(vec![ExprNode::TrueNode, ExprNode::Not(Box::new(ExprNode::TrueNode))]);
        assert!(!comma_node.eval(&mut state));
        let comma_node_true = ExprNode::Comma(vec![ExprNode::Not(Box::new(ExprNode::TrueNode)), ExprNode::TrueNode]);
        assert!(comma_node_true.eval(&mut state));
    }

    #[test]
    fn test_file_types() {
        let fix = FindTempFixture::new("types");
        
        // Regular File
        let reg_path = fix.file("regular.txt", 10);
        let reg_meta = fs::symlink_metadata(&reg_path).unwrap();
        
        // Directory
        let dir_path = fix.dir("mydir");
        let dir_meta = fs::symlink_metadata(&dir_path).unwrap();
        
        // Symlink
        let sym_path = fix.symlink("regular.txt", "mylink");
        let sym_meta = fs::symlink_metadata(&sym_path).unwrap();

        // Socket
        let sock_path = fix.root.join("mysock.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
        let sock_meta = fs::symlink_metadata(&sock_path).unwrap();

        // FIFO
        let fifo_path = fix.root.join("myfifo.fifo");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo_path)
            .status()
            .unwrap();
        assert!(status.success());
        let fifo_meta = fs::symlink_metadata(&fifo_path).unwrap();

        // Character device (/dev/null)
        let char_path = Path::new("/dev/null");
        let char_meta = fs::metadata(char_path).unwrap();

        // Block device (/dev/disk0)
        let block_path = Path::new("/dev/disk0");
        let block_meta_opt = fs::metadata(block_path).ok();

        // Helper to evaluate Type predicate
        let eval_type = |meta: &fs::Metadata, t: char| -> bool {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut state = EvalState {
                path: Path::new("dummy"),
                display_path: Path::new("dummy"),
                metadata: meta,
                pruned: false,
                stdout: &mut stdout,
                stderr: &mut stderr,
                cwd: Path::new("."),
            };
            ExprNode::Type { file_type: t }.eval(&mut state)
        };

        assert!(eval_type(&reg_meta, 'f'));
        assert!(!eval_type(&reg_meta, 'd'));

        assert!(eval_type(&dir_meta, 'd'));
        assert!(!eval_type(&dir_meta, 'f'));

        assert!(eval_type(&sym_meta, 'l'));
        assert!(!eval_type(&sym_meta, 'f'));

        assert!(eval_type(&sock_meta, 's'));
        assert!(!eval_type(&sock_meta, 'f'));

        assert!(eval_type(&fifo_meta, 'p'));
        assert!(!eval_type(&fifo_meta, 'f'));

        assert!(eval_type(&char_meta, 'c'));
        assert!(!eval_type(&char_meta, 'f'));

        if let Some(block_meta) = block_meta_opt {
            assert!(eval_type(&block_meta, 'b'));
            assert!(!eval_type(&block_meta, 'f'));
        }

        // Unknown type option 'z'
        assert!(!eval_type(&reg_meta, 'z'));
    }

    #[test]
    fn test_size_rounding_exact() {
        let fix = FindTempFixture::new("size");
        
        let file_0 = fix.file("file_0.txt", 0);
        let file_1 = fix.file("file_1.txt", 1);
        let file_512 = fix.file("file_512.txt", 512);
        let file_513 = fix.file("file_513.txt", 513);
        let file_1024 = fix.file("file_1024.txt", 1024);

        let eval_size = |path: &Path, compare: char, size_val: u64, unit: u64| -> bool {
            let meta = fs::metadata(path).unwrap();
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut state = EvalState {
                path,
                display_path: path,
                metadata: &meta,
                pruned: false,
                stdout: &mut stdout,
                stderr: &mut stderr,
                cwd: Path::new("."),
            };
            ExprNode::Size { compare, size_val, unit }.eval(&mut state)
        };

        // 512-byte blocks unit (block unit = 512)
        // 0 bytes = 0 blocks
        assert!(eval_size(&file_0, '=', 0, 512));
        assert!(eval_size(&file_0, '-', 1, 512));
        assert!(!eval_size(&file_0, '+', 0, 512));

        // 1 byte = 1 block
        assert!(eval_size(&file_1, '=', 1, 512));
        assert!(eval_size(&file_1, '-', 2, 512));

        // 512 bytes = 1 block
        assert!(eval_size(&file_512, '=', 1, 512));
        
        // 513 bytes = 2 blocks
        assert!(eval_size(&file_513, '=', 2, 512));
        assert!(eval_size(&file_513, '+', 1, 512));

        // Exact bytes comparison (unit = 1)
        assert!(eval_size(&file_1, '=', 1, 1));
        assert!(eval_size(&file_513, '=', 513, 1));
        assert!(eval_size(&file_513, '+', 500, 1));
        assert!(eval_size(&file_513, '-', 600, 1));

        // kbytes comparison (unit = 1024)
        assert!(eval_size(&file_1024, '=', 1, 1024));
        assert!(!eval_size(&file_1024, '=', 2, 1024));

        // Unknown size compare char
        assert!(!eval_size(&file_1024, '?', 1, 1024));
        assert!(!eval_size(&file_512, '?', 1, 512));
    }

    #[test]
    fn test_perm_modes() {
        let fix = FindTempFixture::new("perms");
        let f_path = fix.file("perm.txt", 0);
        fs::set_permissions(&f_path, fs::Permissions::from_mode(0o755)).unwrap();
        let meta = fs::metadata(&f_path).unwrap();

        let eval_perm = |mode_type: char, mode: u32| -> bool {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut state = EvalState {
                path: &f_path,
                display_path: &f_path,
                metadata: &meta,
                pruned: false,
                stdout: &mut stdout,
                stderr: &mut stderr,
                cwd: Path::new("."),
            };
            ExprNode::Perm { mode_type, mode }.eval(&mut state)
        };

        // Exact mode (=)
        assert!(eval_perm('=', 0o755));
        assert!(!eval_perm('=', 0o644));

        // All bits set (-)
        assert!(eval_perm('-', 0o700));
        assert!(eval_perm('-', 0o755));
        assert!(!eval_perm('-', 0o777));

        // Any bits set (/)
        assert!(eval_perm('/', 0o001));
        assert!(eval_perm('/', 0o700));
        assert!(!eval_perm('/', 0o002));

        // Unknown perm compare char
        assert!(!eval_perm('?', 0o755));
    }

    #[test]
    fn test_exec_evaluation() {
        let fix = FindTempFixture::new("exec");
        let f_path = fix.file("file.txt", 0);
        let meta = fs::metadata(&f_path).unwrap();

        let eval_exec = |command: Vec<String>| -> (bool, String, String) {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut state = EvalState {
                path: &f_path,
                display_path: &f_path,
                metadata: &meta,
                pruned: false,
                stdout: &mut stdout,
                stderr: &mut stderr,
                cwd: &fix.root,
            };
            let matched = ExprNode::Exec { command }.eval(&mut state);
            (matched, String::from_utf8_lossy(&stdout).into_owned(), String::from_utf8_lossy(&stderr).into_owned())
        };

        // Success exec
        let (matched, out, err) = eval_exec(vec!["echo".to_string(), "hello".to_string(), "{}".to_string()]);
        assert!(matched);
        assert!(out.contains("hello"));
        assert!(out.contains("file.txt"));
        assert!(err.is_empty());

        // Failed exec
        let (matched, out, err) = eval_exec(vec!["false".to_string()]);
        assert!(!matched);
        assert!(out.is_empty());
        assert!(err.is_empty());

        // Non-existent command exec
        let (matched, out, err) = eval_exec(vec!["non_existent_command_123_abc".to_string()]);
        assert!(!matched);
        assert!(out.is_empty());
        assert!(err.contains("exec failed"));

        // Empty command
        let (matched, out, err) = eval_exec(vec![]);
        assert!(!matched);
        assert!(out.is_empty());
        assert!(err.is_empty());
    }

    #[test]
    fn test_delete_action() {
        let fix = FindTempFixture::new("delete");
        let f = fix.file("file.txt", 0);
        let d = fix.dir("subdir");
        
        let f_meta = fs::metadata(&f).unwrap();
        let d_meta = fs::metadata(&d).unwrap();

        let mut stdout_f = Vec::new();
        let mut stderr_f = Vec::new();
        
        // Delete file
        let mut state_f = EvalState {
            path: &f,
            display_path: &f,
            metadata: &f_meta,
            pruned: false,
            stdout: &mut stdout_f,
            stderr: &mut stderr_f,
            cwd: &fix.root,
        };
        assert!(ExprNode::Delete.eval(&mut state_f));
        assert!(!f.exists());

        let mut stdout_d = Vec::new();
        let mut stderr_d = Vec::new();

        // Delete directory
        let mut state_d = EvalState {
            path: &d,
            display_path: &d,
            metadata: &d_meta,
            pruned: false,
            stdout: &mut stdout_d,
            stderr: &mut stderr_d,
            cwd: &fix.root,
        };
        assert!(ExprNode::Delete.eval(&mut state_d));
        assert!(!d.exists());

        // Try deleting again (should fail because they do not exist)
        assert!(!ExprNode::Delete.eval(&mut state_f));
        assert!(!ExprNode::Delete.eval(&mut state_d));
        assert!(String::from_utf8_lossy(&stderr_f).contains("cannot delete") || String::from_utf8_lossy(&stderr_d).contains("cannot delete"));
    }

    #[test]
    fn test_newer_nodes() {
        let fix = FindTempFixture::new("newer");
        let ref_file = fix.file("ref.txt", 0);
        let _new_file = fix.file("new.txt", 0);

        // Sleep to ensure time difference
        std::thread::sleep(Duration::from_millis(10));
        let newer_file = fix.file("newer.txt", 0);

        let ref_meta = fs::metadata(&ref_file).unwrap();
        let reference_mtime = ref_meta.modified().unwrap();

        let eval_newer = |path: &Path| -> bool {
            let meta = fs::metadata(path).unwrap();
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut state = EvalState {
                path,
                display_path: path,
                metadata: &meta,
                pruned: false,
                stdout: &mut stdout,
                stderr: &mut stderr,
                cwd: &fix.root,
            };
            ExprNode::Newer { reference_mtime }.eval(&mut state)
        };

        // newer_file is newer than ref_file
        assert!(eval_newer(&newer_file));
        // ref_file is not newer than itself
        assert!(!eval_newer(&ref_file));
    }

    #[test]
    fn test_time_nodes() {
        let fix = FindTempFixture::new("time");
        let f = fix.file("file.txt", 0);
        let meta = fs::metadata(&f).unwrap();

        let eval_time = |node: ExprNode| -> bool {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut state = EvalState {
                path: &f,
                display_path: &f,
                metadata: &meta,
                pruned: false,
                stdout: &mut stdout,
                stderr: &mut stderr,
                cwd: &fix.root,
            };
            node.eval(&mut state)
        };

        // mtime: modified time check (compare: '+', '-', '=')
        assert!(eval_time(ExprNode::MTime { compare: '-', days: 1 }));
        assert!(!eval_time(ExprNode::MTime { compare: '+', days: 1 }));
        assert!(eval_time(ExprNode::MTime { compare: '=', days: 0 }));

        // atime: accessed time check
        assert!(eval_time(ExprNode::ATime { compare: '-', days: 1 }));
        assert!(!eval_time(ExprNode::ATime { compare: '+', days: 1 }));

        // ctime: ctime check
        assert!(eval_time(ExprNode::CTime { compare: '-', days: 1 }));
        assert!(!eval_time(ExprNode::CTime { compare: '+', days: 1 }));

        // Invalid compare chars
        assert!(!eval_time(ExprNode::CTime { compare: '?', days: 1 }));
        assert!(!eval_time(ExprNode::ATime { compare: '?', days: 1 }));
        assert!(!eval_time(ExprNode::MTime { compare: '?', days: 1 }));
    }

    #[test]
    fn test_users_groups() {
        // Parse numerical IDs
        assert_eq!(get_uid_by_name("0"), Some(0));
        assert_eq!(get_gid_by_name("0"), Some(0));

        // Parse root/wheel user/group names
        let root_uid = get_uid_by_name("root");
        assert!(root_uid.is_some());
        
        let wheel_gid = get_gid_by_name("wheel").or_else(|| get_gid_by_name("root"));
        assert!(wheel_gid.is_some());

        // Invalid names
        assert_eq!(get_uid_by_name("nonexistent_user_xyz_123"), None);
        assert_eq!(get_gid_by_name("nonexistent_group_xyz_123"), None);
    }

    #[test]
    fn test_parser_predicate_arguments() {
        let mut options = TraversalOptions {
            maxdepth: None,
            mindepth: None,
            depth_first: false,
            mount: false,
            follow_symlinks: false,
        };

        let args = vec![
            "-maxdepth", "3",
            "-mindepth", "1",
            "-depth",
            "-mount",
            "-name", "*.txt",
            "-iname", "*.log",
            "-path", "/a/b/*",
            "-ipath", "/a/c/*",
            "-type", "f",
            "-size", "+100c",
            "-mtime", "+2",
            "-atime", "-5",
            "-ctime", "1",
            "-perm", "-0755",
            "-print",
            "-print0",
            "-prune",
        ].into_iter().map(OsString::from).collect::<Vec<_>>();

        let mut parser = Parser::new(&args, Path::new("."), &mut options);
        let res = parser.parse_expression();
        assert!(res.is_ok());

        assert_eq!(options.maxdepth, Some(3));
        assert_eq!(options.mindepth, Some(1));
        assert!(options.depth_first);
        assert!(options.mount);
    }

    #[test]
    fn test_parser_errors() {
        let mut options = TraversalOptions {
            maxdepth: None,
            mindepth: None,
            depth_first: false,
            mount: false,
            follow_symlinks: false,
        };

        let mut check_err = |args: Vec<&str>| -> String {
            let o_args = args.into_iter().map(OsString::from).collect::<Vec<_>>();
            let mut parser = Parser::new(&o_args, Path::new("."), &mut options);
            let res = parser.parse_expression();
            assert!(res.is_err());
            res.err().unwrap()
        };

        assert!(check_err(vec!["(", "-print"]).contains("Expected ')'"));
        assert!(check_err(vec!["-maxdepth", "abc"]).contains("invalid maxdepth"));
        assert!(check_err(vec!["-mindepth", "abc"]).contains("invalid mindepth"));
        assert!(check_err(vec!["-type", "xyz"]).contains("invalid file type"));
        assert!(check_err(vec!["-type", "x"]).contains("unknown file type"));
        assert!(check_err(vec!["-size", "abc"]).contains("invalid size"));
        assert!(check_err(vec!["-size", "+abc"]).contains("invalid size"));
        assert!(check_err(vec!["-size", "+10x"]).contains("unknown size unit"));
        assert!(check_err(vec!["-perm", "999"]).contains("invalid mode"));
        assert!(check_err(vec!["-mtime", "abc"]).contains("invalid time value"));
        assert!(check_err(vec!["-newer", "nonexistent_file_xyz_123"]).contains("cannot stat"));
        assert!(check_err(vec!["-user", "nonexistent_user_xyz_123"]).contains("not a valid user"));
        assert!(check_err(vec!["-group", "nonexistent_group_xyz_123"]).contains("not a valid group"));
        assert!(check_err(vec!["-exec", "echo", "{}"]).contains("missing argument to '-exec'"));
        assert!(check_err(vec!["-maxdepth"]).contains("missing argument"));
        assert!(check_err(vec![]).contains("Unexpected end of expression"));
        assert!(check_err(vec!["-unknown-pred"]).contains("unknown predicate"));
    }

    #[test]
    fn test_run_options_and_errors() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        
        // Extra arguments in run
        let code = run(
            vec![OsString::from("."), OsString::from("-print"), OsString::from(")")],
            Path::new("."),
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(code, 1);
        assert!(String::from_utf8_lossy(&stderr).contains("extra expression arguments"));

        // Missing argument in predicate parsing in run
        stderr.clear();
        let code2 = run(
            vec![OsString::from("."), OsString::from("-maxdepth")],
            Path::new("."),
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(code2, 1);
        assert!(String::from_utf8_lossy(&stderr).contains("missing argument"));
    }

    #[test]
    fn test_glob_compilation() {
        assert!(compile_glob("a*b", false).is_ok());
        assert!(compile_glob("a?b", false).is_ok());
        assert!(compile_glob("a[!c]b", false).is_ok());
        assert!(compile_glob("a[c]b", false).is_ok());
    }

    #[test]
    fn test_traverse_loop_detection() {
        let fix = FindTempFixture::new("trav_loop");
        let _d = fix.dir("d");
        let _link = fix.symlink("../d", "d/link");

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(
            vec!["-L".into(), "d".into()],
            &fix.root,
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(code, 1);
        assert!(String::from_utf8_lossy(&stderr).contains("loop detected"));
    }
}
