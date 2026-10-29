//! `ahok link`: symlink a config repo's files into place.
//!
//! This module prints one row per link from the `link` crate and
//! reports `Failed` if any link did not resolve.

use std::path;

use clap;
use link;

use crate::cli;

/// Arguments for `ahok link`.
#[derive(clap::Args)]
pub struct Args {
  /// The repo holding `links.toml`.
  #[arg(default_value = ".")]
  repo: path::PathBuf,
  /// Which targets to link: linux, windows or cloud. Defaults to this
  /// machine, and to cloud when `CLAUDE_CODE_REMOTE` is true.
  #[arg(long)]
  platform: Option<link::Platform>,
  /// Show what would change, and change nothing.
  #[arg(long)]
  dry_run: bool,
}

impl Args {
  /// Links everything and prints the report.
  pub fn run(self) -> Result<cli::Status, cli::Error> {
    let repo =
      path::absolute(&self.repo).map_err(|source| cli::Error::Read {
        path: self.repo.clone(),
        source,
      })?;
    let platform = self.platform.unwrap_or_else(link::Platform::current);
    let rows = link::run(&repo, platform, self.dry_run)?;
    let labels: Vec<String> =
      rows.iter().map(|row| label(&row.outcome)).collect();
    let width = labels.iter().map(String::len).max().unwrap_or(0);
    for (row, label) in rows.iter().zip(&labels) {
      let line = format!("{label:<width$}  {}  {}", row.target, row.note);
      println!("{}", line.trim_end());
    }
    let failed = rows.iter().filter(|row| row.outcome.failed()).count();
    if failed == 0 {
      return Ok(cli::Status::Clean);
    }
    eprintln!("\n{failed} link(s) did not resolve.");
    Ok(cli::Status::Failed)
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
