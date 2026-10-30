//! Command routing for the `ahok` binary.
//!
//! This module maps each subcommand to its file in `cli/`. Each command owns
//! its own output; `main` maps the outcome onto an exit code.

mod ci;
mod doctor;
mod git;
mod hook;
mod link;
mod stats;
#[cfg(feature = "update")]
mod update;

use std::io;
use std::path;

use clap;
use thiserror;

/// Control plane for coding-agent harness hooks.
#[derive(clap::Parser)]
#[command(name = "ahok", version)]
pub enum Args {
  /// Wait for CI and print one summary.
  #[command(subcommand)]
  Ci(ci::Args),
  /// Run the git hook rules; called from git's commit-msg and pre-push.
  #[command(subcommand)]
  Git(git::Args),
  /// Answer one harness hook call: hook JSON on stdin, reply on stdout.
  Hook(hook::Args),
  /// Check that rules load, tools are installed, and harnesses call ahok.
  Doctor,
  /// Symlink a config repo's files into place, from its `links.toml`.
  Link(link::Args),
  /// Print hook latency percentiles and rule hit counts.
  Stats,
  #[cfg(feature = "update")]
  /// Update the ahok binary to the latest release.
  Update(update::Args),
}

/// How a command that ran to completion ended.
pub enum Status {
  /// Nothing to report: exit 0.
  Clean,
  /// The command found a problem, such as a failed doctor check: exit 1.
  Failed,
  /// The command ran out of time and should be run again: exit 75
  /// (`EX_TEMPFAIL`).
  Retry,
}

/// Why a command could not run.
#[derive(Debug, thiserror::Error)]
pub enum Error {
  /// No state directory could be located.
  #[error("no state directory: set XDG_STATE_HOME or HOME")]
  NoState,
  /// `links.toml` could not be read or parsed.
  #[error(transparent)]
  Link(#[from] ::link::Error),
  /// A state file exists but could not be read.
  #[error("{}: {source}", path.display())]
  Read {
    /// The file that failed.
    path: path::PathBuf,
    /// Why reading it failed.
    source: io::Error,
  },
  /// Reading CI checks from GitHub failed.
  #[error(transparent)]
  Ci(#[from] ::ci::Error),
  #[cfg(feature = "update")]
  /// Self-update failed.
  #[error(transparent)]
  Update(#[from] update::Error),
}

impl Args {
  /// Runs the chosen subcommand.
  pub fn run(self) -> Result<Status, Error> {
    match self {
      Self::Ci(args) => Ok(args.run()?),
      Self::Doctor => Ok(doctor::run()),
      Self::Git(args) => Ok(args.run()),
      Self::Hook(args) => Ok(args.run()),
      Self::Link(args) => args.run(),
      Self::Stats => stats::run(),
      #[cfg(feature = "update")]
      Self::Update(args) => args.run().map_err(Error::Update),
    }
  }
}
