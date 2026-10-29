//! Command routing for the `ahok` binary.
//!
//! This module maps each subcommand to its file in `cli/`. Each command owns
//! its own output and exit code.

mod hook;
mod stats;

use std::process;

/// Control plane for coding-agent harness hooks.
#[derive(clap::Parser)]
#[command(name = "ahok", version)]
pub enum Args {
  /// Answer one harness hook call: hook JSON on stdin, reply on stdout.
  Hook(hook::Args),
  /// Print hook latency percentiles and rule hit counts.
  Stats,
}

impl Args {
  /// Runs the chosen subcommand.
  pub fn run(self) -> process::ExitCode {
    match self {
      Self::Hook(args) => args.run(),
      Self::Stats => stats::run(),
    }
  }
}
