use rust_unix_tools::tools::ls;
use std::env;
use std::io;
use std::process;

fn main() {
    let args = env::args_os().skip(1);
    let cwd = env::current_dir().unwrap_or_else(|error| {
        eprintln!("ls: {error}");
        process::exit(1);
    });
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();

    process::exit(ls::run(args, &cwd, &mut stdout, &mut stderr));
}
