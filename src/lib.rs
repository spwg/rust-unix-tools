//! Rust implementations of Unix command-line utilities.
//!
//! The crate exposes command implementations as ordinary library functions so
//! each tool can be tested without spawning a process. Binary targets in
//! `src/bin` are intentionally thin wrappers around these modules.

pub mod getopt;
pub mod tools;
