//! `ahok setup [repo...]`: apply each workspace's `ahok.toml` to this machine.
//!
//! This module loads each manifest, links what it names, and prints one row
//! per link. Reading the manifest and linking belong to `ahok_core`.

use std::path;

use ahok_core::link;
use ahok_core::manifest;
use clap;

use crate::cli;

/// Arguments for `ahok setup`.
#[derive(clap::Args)]
pub struct Args {
  /// Workspaces holding an `ahok.toml`.
  #[arg(default_value = ".")]
  repos: Vec<path::PathBuf>,
  /// Which targets to link: linux, windows or cloud. Defaults to this
  /// machine, and to cloud when `CLAUDE_CODE_REMOTE` is true.
  #[arg(long)]
  platform: Option<link::Platform>,
  /// Change nothing; print what would change, and fail if anything would.
  #[arg(long)]
  check: bool,
}

impl Args {
  /// Applies every manifest and prints the report.
  ///
  /// `Failed` when a link did not resolve, or under `--check` when anything
  /// is out of date.
  pub fn run(self) -> Result<cli::Status, cli::Error> {
    let platform = self.platform.unwrap_or_else(link::Platform::current);
    let mut rows = Vec::new();
    for repo in &self.repos {
      let repo = path::absolute(repo).map_err(|source| cli::Error::Read {
        path: repo.clone(),
        source,
      })?;
      let manifest = manifest::load(&repo)?;
      rows.extend(link::apply(&repo, &manifest.link, platform, self.check));
    }
    let labels: Vec<String> =
      rows.iter().map(|row| label(&row.outcome)).collect();
    let width = labels.iter().map(String::len).max().unwrap_or(0);
    for (row, label) in rows.iter().zip(&labels) {
      let line = format!("{label:<width$}  {}  {}", row.target, row.note);
      println!("{}", line.trim_end());
    }
    let failed = rows.iter().filter(|row| row.outcome.failed()).count();
    let stale = rows
      .iter()
      .filter(|row| matches!(row.outcome, link::Outcome::Would(_)))
      .count();
    if failed > 0 {
      eprintln!("\n{failed} link(s) did not resolve.");
      return Ok(cli::Status::Failed);
    }
    if stale > 0 {
      eprintln!("\n{stale} link(s) out of date; run `ahok setup`.");
      return Ok(cli::Status::Failed);
    }
    Ok(cli::Status::Clean)
  }
}

/// The report label: lowercase for fine, uppercase for failures.
fn label(outcome: &link::Outcome) -> String {
  match outcome {
    link::Outcome::Ok => "ok".to_owned(),
    link::Outcome::Linked => "linked".to_owned(),
    link::Outcome::Would(action) => format!("would {action}"),
    link::Outcome::Missing => "MISSING".to_owned(),
    link::Outcome::Conflict => "CONFLICT".to_owned(),
    link::Outcome::Failed => "FAILED".to_owned(),
  }
}
