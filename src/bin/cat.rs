use rust_unix_tools::tools::cat;
use std::env;
use std::io;
use std::process;

fn main() {
    let args = env::args_os().skip(1);
    let stdout = io::stdout();
    let mut handle_out = stdout.lock();
    let stderr = io::stderr();
    let mut handle_err = stderr.lock();
    let cwd = env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    let code = cat::run(args, &cwd, &mut handle_out, &mut handle_err);
    process::exit(code);
}
