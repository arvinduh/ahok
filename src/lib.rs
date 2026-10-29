//! Policy engine for coding-agent harness hooks.
//!
//! The library turns harness hook input into decisions and returns data only:
//! it never prints, never exits, and has no dependency on `clap`. The `ahok`
//! binary owns argument parsing, I/O streams, and exit codes.

#![deny(clippy::print_stdout, clippy::print_stderr)]

pub mod doctor;
