//! GNU-style `find`.
//!
//! This module implements the core logic of the `find` command, including
//! a recursive descent parser for expressions and directory traversal.

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
    while let Some(c) = chars.next() {
        match c {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '[' => {
                regex.push('[');
                if chars.peek() == Some(&'!') {
                    regex.push('^');
                    chars.next();
                }
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
