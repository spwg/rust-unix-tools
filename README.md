# rust-unix-tools

Rust implementations of Unix command-line utilities.

## Layout

Each command has a small binary wrapper in `src/bin/` and reusable
implementation code in `src/tools/`:

- `src/bin/echo.rs` -> `src/tools/echo.rs`
- `src/bin/ls.rs` -> `src/tools/ls.rs`

This keeps CLI process handling separate from command behavior, which lets the
tests call each tool directly and makes it straightforward to add more
utilities.

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

## `ls` Conformance Target

The `ls` implementation targets GNU coreutils `ls` behavior documented by the
GNU coreutils 9.9 man page. A downloaded reference copy from man7.org is stored
at `tests/fixtures/gnu-ls-9.9-manpage.html`; tests assert this fixture is the GNU
coreutils reference and mine option coverage from it.

The current implementation focuses on deterministic non-terminal output and
covers GNU option families for hidden entries, directory operands, indicators,
sorting, recursive traversal, inode/block prefixes, long output, numeric IDs,
human-readable sizes, symlink dereferencing, and documented help/version/error
shapes.

Coverage for the `ls` implementation is enforced with:

```sh
cargo tarpaulin --test ls_manpage_tests --ignore-tests --include-files src/tools/ls.rs --fail-under 100
```
