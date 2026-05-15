# rust-unix-tools

Rust implementations of Unix command-line utilities.

## `echo` Conformance Target

The `echo` implementation targets GNU coreutils `echo` behavior as shipped by
GNU coreutils 9.9, with the exceptions listed below.

For behavior checks, `gecho` from Homebrew `coreutils` is the reference oracle on
macOS:

```sh
brew install coreutils
cargo test
```

The intended target is:

- Match GNU `echo` output semantics for ordinary operands.
- Match GNU parsing of `-n`, `-e`, `-E`, and combined short option groups such as
  `-ne`.
- Match GNU backslash escape handling when escape interpretation is enabled.
- Match GNU behavior when `POSIXLY_CORRECT` is set, including its different
  option parsing and default escape interpretation.
- Preserve raw Unix argument bytes; arguments are not required to be valid UTF-8.

Intentional exceptions:

- Standalone `--help` and `--version` are supported, but their exact text is
  project-specific and is not expected to be byte-for-byte identical to GNU
  `gecho`.
- This project does not claim POSIX `echo` portability semantics as the primary
  target. POSIX leaves important `echo` cases implementation-defined, especially
  `-n` and backslash-containing operands.

Testing policy:

- Differential tests should compare behavior against `gecho`, not `/bin/echo`,
  because `/bin/echo` on macOS/BSD has different semantics.
- Differential tests may exclude standalone `--help` and `--version` text unless
  the test is specifically checking this project's branded output.
- New edge cases should be covered in both normal mode and `POSIXLY_CORRECT`
  mode when relevant.
