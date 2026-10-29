//! Command routing for the `ahok` binary.
//!
//! This module maps each subcommand to its file in `cli/`. Each command owns
//! its own output; `main` maps the outcome onto an exit code.

mod doctor;
mod hook;
mod link;
mod stats;

use std::io;
use std::path;

use clap;
use thiserror;

/// Control plane for coding-agent harness hooks.
#[derive(clap::Parser)]
#[command(name = "ahok", version)]
pub enum Args {
  /// Answer one harness hook call: hook JSON on stdin, reply on stdout.
  Hook(hook::Args),
  /// Check that rules load, tools are installed, and harnesses call ahok.
  Doctor,
  /// Symlink a config repo's files into place, from its `links.toml`.
  Link(link::Args),
  /// Print hook latency percentiles and rule hit counts.
  Stats,
}

/// How a command that ran to completion ended.
pub enum Status {
  /// Nothing to report: exit 0.
  Clean,
  /// The command found a problem, such as a failed doctor check: exit 1.
  Failed,
}

/// Why a command could not run.
#[derive(Debug, thiserror::Error)]
pub enum Error {
  /// No state directory could be located.
  #[error("no state directory: set XDG_STATE_HOME or HOME")]
  NoState,
  /// `links.toml` could not be read or parsed.
  #[error(transparent)]
  Link(#[from] ahok::link::Error),
  /// A state file exists but could not be read.
  #[error("{}: {source}", path.display())]
  Read {
    /// The file that failed.
    path: path::PathBuf,
    /// Why reading it failed.
    source: io::Error,
  },
}

impl Args {
  /// Runs the chosen subcommand.
  pub fn run(self) -> Result<Status, Error> {
    match self {
      Self::Doctor => Ok(doctor::run()),
      Self::Hook(args) => Ok(args.run()),
      Self::Link(args) => args.run(),
      Self::Stats => stats::run(),
    }
  }
}
