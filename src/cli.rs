//! Command routing for the `ahok` binary.
//!
//! This module maps each subcommand to its file in `cli/`. Each command owns
//! its own output; `main` maps the outcome onto an exit code.

mod git;
mod hook;
mod setup;
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
  /// Run the git hook rules; called from git's commit-msg and pre-push.
  #[command(subcommand, hide = true)]
  Git(git::Args),
  /// Answer one harness hook call: hook JSON on stdin, reply on stdout.
  #[command(hide = true)]
  Hook(hook::Args),
  /// Apply each workspace's `ahok.toml` to this machine.
  Setup(setup::Args),
  #[cfg(feature = "update")]
  /// Update the ahok binary to the latest release.
  Update(update::Args),
}

/// How a command that ran to completion ended.
pub enum Status {
  /// Nothing to report: exit 0.
  Clean,
  /// The command found a problem, such as a broken git rule: exit 1.
  Failed,
}

/// Why a command could not run.
#[derive(Debug, thiserror::Error)]
pub enum Error {
  /// `ahok.toml` could not be read or parsed.
  #[error(transparent)]
  Manifest(#[from] ahok_core::manifest::Error),
  /// A state file exists but could not be read.
  #[error("{}: {source}", path.display())]
  Read {
    /// The file that failed.
    path: path::PathBuf,
    /// Why reading it failed.
    source: io::Error,
  },
  #[cfg(feature = "update")]
  /// Self-update failed.
  #[error(transparent)]
  Update(#[from] update::Error),
}

impl Args {
  /// Runs the chosen subcommand.
  pub fn run(self) -> Result<Status, Error> {
    match self {
      Self::Git(args) => Ok(args.run()),
      Self::Hook(args) => Ok(args.run()),
      Self::Setup(args) => args.run(),
      #[cfg(feature = "update")]
      Self::Update(args) => args.run().map_err(Error::Update),
    }
  }
}
