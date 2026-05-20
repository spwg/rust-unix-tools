# rust-unix-tools

Rust implementations of Unix command-line utilities.

## Layout

Each command has a small binary wrapper in `src/bin/` and reusable library implementation code in `src/tools/`:

- `src/bin/echo.rs` -> `src/tools/echo.rs`
- `src/bin/ls.rs` -> `src/tools/ls.rs`
- `src/bin/find.rs` -> `src/tools/find.rs`
- `src/bin/grep.rs` -> `src/tools/grep.rs`
- `src/bin/cat.rs` -> `src/tools/cat.rs`


## Reusable Argument Parsing: `getopt`

The project utilizes a custom, GNU-compatible option parsing library implemented in `src/getopt.rs`.

Key features:
- Short options (e.g., `-a`) and option grouping (e.g., `-al`).
- Short options with attached or separate arguments (e.g., `-bval` or `-b val`).
- Long options (e.g., `--all`, `--sort=size`) with required or optional arguments.
- Standard option terminator (`--`).
- GNU-style argument permutation (intermixed options and operands) which can be optionally disabled via a `POSIXLY_CORRECT` mode flag.

---

## Utilities & Conformance Targets

### 1. `cat` Conformance Target
The `cat` implementation matches standard GNU `cat` behavior as shipped by GNU coreutils.
- **Supported Options**:
  - `-A`, `--show-all`: Equivalent to `-vET`.
  - `-b`, `--number-nonblank`: Number nonempty output lines, overrides `-n`.
  - `-e`: Equivalent to `-vE`.
  - `-E`, `--show-ends`: Display `$` at the end of each line.
  - `-n`, `--number`: Number all output lines.
  - `-s`, `--squeeze-blank`: Suppress repeated empty output lines.
  - `-t`: Equivalent to `-vT`.
  - `-T`, `--show-tabs`: Display TAB characters as `^I`.
  - `-u`: Ignored (for POSIX compatibility).
  - `-v`, `--show-nonprinting`: Use `^` and `M-` notation (except for LFD and TAB).
- **Behavior**: Efficiently handles disk page buffering (4096-byte buffers) and streams inputs (including stdin `-` and files) sequentially.

### 2. `echo` Conformance Target
The `echo` implementation targets GNU coreutils 9.9 `echo` behavior.
- **Behavior**:
  - Matches GNU `echo` output semantics for ordinary operands.
  - Matches GNU parsing of `-n`, `-e`, `-E`, and combined short option groups such as `-ne`.
  - Matches GNU backslash escape handling when escape interpretation is enabled.
  - Matches GNU behavior when `POSIXLY_CORRECT` is set, including its different option parsing and default escape interpretation.
  - Preserves raw Unix argument bytes; arguments are not required to be valid UTF-8.
- **Intentional exceptions**: Standalone `--help` and `--version` are supported, but their exact text is project-specific and not expected to be byte-for-byte identical to GNU `gecho`.

### 3. `find` Conformance Target
The `find` implementation targets GNU `find` behaviors, featuring a complete recursive descent expression parser and traversal system.
- **Symlink Options**: Support for `-H` (dereference command-line arguments only), `-L` (dereference all symlinks), and `-P` (never dereference symlinks; default).
- **Supported Predicates**:
  - **Tests**: `-name`, `-iname`, `-path`/`-wholename`, `-ipath`/`-iwholename`, `-type` (supports `b`, `c`, `d`, `p`, `f`, `l`, `s`), `-size` (supports units `c`, `k`, `M`, `G`, `b`), `-mtime`, `-atime`, `-ctime`, `-perm`, `-newer`, `-user`, `-group`.
  - **Actions**: `-print`, `-print0`, `-delete`, `-prune`, `-exec`.
  - **Operators**: Parentheses `( )`, negation `!` / `-not`, logical AND `-a` / `-and` (implied between consecutive predicates), logical OR `-o` / `-or`, and comma `,`.
  - **Global Options**: `-maxdepth`, `-mindepth`, `-depth`, `-mount` / `-xdev`.

### 4. `grep` Conformance Target
The `grep` implementation targets GNU `grep` behavior with recursive search and pattern-matching.
- **Supported Options**:
  - **Regexp Selection**: `-E` / `--extended-regexp` (ignored for default regex engine support), `-F` / `--fixed-strings` (literal matches), `-G` / `--basic-regexp` (ignored).
  - **Matching Control**: `-e` / `--regexp` (multiple patterns), `-f` / `--file` (read patterns from file), `-i` / `-y` / `--ignore-case`, `-v` / `--invert-match`, `-w` / `--word-regexp`, `-x` / `--line-regexp`.
  - **Output Control**: `-c` / `--count`, `-L` / `--files-without-match`, `-l` / `--files-with-matches`, `-m` / `--max-count`, `-o` / `--only-matching`, `-q` / `--quiet` / `--silent`, `-s` / `--no-messages`, `-b` / `--byte-offset`, `-n` / `--line-number`, `-H` / `--with-filename`, `-h` / `--no-filename`, `-Z` / `--null`.
  - **Directory Traversal**: `-r` / `--recursive`, `-R` / `--dereference-recursive`.

### 5. `ls` Conformance Target
The `ls` implementation targets GNU coreutils `ls` behavior documented by the GNU coreutils 9.9 man page.
- **Behavior**: Focuses on deterministic non-terminal output (one entry per line by default).
- **Supported Options**: Covers GNU option families for hidden entries (`-a`, `-A`), directory operands (`-d`), indicators (`-F`, `--indicator-style`, `--file-type`), sorting (`--sort`, `-t`, `-S`, `-X`, `-r`), recursive traversal (`-R`), inode/block prefixes (`-i`, `-s`), long output (`-l`, `-g`, `-o`, `-n`, `--no-group`), human-readable sizes (`-h`, `--si`), symlink dereferencing (`-L`), and block sizes (`--block-size`).

---

## Development and Testing

### 1. General Test Execution
All standard unit, integration, and conformance tests can be run via:
```sh
cargo test
```

### 2. Differential Testing (`echo`)
Differential tests compare `echo` behavior against `gecho` (GNU `echo` from Homebrew `coreutils` on macOS):
```sh
# On macOS, install gecho first:
brew install coreutils

# Run fuzz and differential tests
cargo test --test fuzz_test
```
*Note: Standalone `--help` and `--version` differential testing may exclude exact branded output check.*

