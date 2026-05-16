use std::process::Command;

/// Path to the compiled binary.
fn echo_binary() -> std::path::PathBuf {
    let mut path = std::path::PathBuf::from(env!("CARGO_BIN_EXE_echo"));
    if path.is_dir() {
        path.push("echo");
    }
    path
}

#[test]
fn exhaustive_fuzz_diff_test() {
    let components = [
        "-n",
        "\\c",
        "hello",
        "world",
        "",
        " ",
        "-n hello",
        "world\\c",
        "foo\\cbar",
        "!",
        "--help",
    ];

    let rust_echo = echo_binary();
    let sys_echo = "gecho";

    let mut args_buffer = Vec::new();
    let mut tested_count = 0;

    // Recursive exhaustive generation up to length 3 (1 + 11 + 121 + 1331 = 1464 combinations)
    // We assert equality of raw byte output for every combination.
    fn run_combinations(
        depth: usize,
        max_depth: usize,
        components: &[&str],
        current_args: &mut Vec<String>,
        rust_echo: &std::path::Path,
        sys_echo: &str,
        tested_count: &mut usize,
    ) {
        if !(current_args.len() == 1 && current_args[0] == "--help") {
            let rust_out = Command::new(rust_echo)
                .args(current_args.as_slice())
                .output()
                .expect("Failed to run rust echo");

            let sys_out = Command::new(sys_echo)
                .args(current_args.as_slice())
                .output()
                .expect("Failed to run sys echo");

            assert_eq!(
                rust_out.stdout, sys_out.stdout,
                "\n❌ MISMATCH DETECTED!\nArguments: {:?}\nRust out : {:?}\nSys out  : {:?}",
                current_args, rust_out.stdout, sys_out.stdout
            );
            *tested_count += 1;
        }

        if depth < max_depth {
            for &comp in components {
                current_args.push(comp.to_string());
                run_combinations(
                    depth + 1,
                    max_depth,
                    components,
                    current_args,
                    rust_echo,
                    sys_echo,
                    tested_count,
                );
                current_args.pop();
            }
        }
    }

    run_combinations(
        0,
        3,
        &components,
        &mut args_buffer,
        &rust_echo,
        sys_echo,
        &mut tested_count,
    );

    println!(
        "Successfully fuzz-tested {} combinations against gecho",
        tested_count
    );
}

#[test]
fn posixly_correct_matches_gnu_echo_option_parsing() {
    let rust_echo = echo_binary();
    let cases: &[&[&str]] = &[
        &["--help"],
        &["--version"],
        &["-ne", "--"],
        &["-n", "-ne", "--"],
        &["-n", "-E", "a\\nb"],
        &["-n", "-e", "a\\nb"],
        &["-n", "-x", "--"],
    ];

    for args in cases {
        let rust_out = Command::new(&rust_echo)
            .env("POSIXLY_CORRECT", "1")
            .args(*args)
            .output()
            .expect("Failed to run rust echo");

        let gnu_out = Command::new("gecho")
            .env("POSIXLY_CORRECT", "1")
            .args(*args)
            .output()
            .expect("Failed to run gecho");

        assert_eq!(
            rust_out.stdout, gnu_out.stdout,
            "\nMISMATCH DETECTED!\nArguments: {:?}\nRust out : {:?}\nGNU out  : {:?}",
            args, rust_out.stdout, gnu_out.stdout
        );
    }
}
