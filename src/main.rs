//! The `ahok` binary: the process host.
//!
//! This file parses arguments and returns the command's exit code. Commands
//! live in `cli`; policy lives in the `ahok` library.

mod cli;

use std::process;

use clap::Parser;

fn main() -> process::ExitCode {
  cli::Args::parse().run()
}
