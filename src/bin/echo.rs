use rust_unix_tools::tools::echo;
use std::env;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::process;

fn main() {
    let args = env::args_os().skip(1).map(|os_str| os_str.into_vec());
    let stdout = io::stdout();
    let mut handle = stdout.lock();

    if let Err(error) = echo::echo(args, &mut handle) {
        eprintln!("echo: {error}");
        process::exit(1);
    }
}
