//! `ahok setup [repo...]`: apply each workspace's `ahok.toml` to this machine.
//!
//! This module loads each manifest, links what it names, writes the
//! harness configs `[agents]` implies, and prints one row per target.
//! Reading the manifest, linking and writing belong to `ahok_core`.

use std::path;

use ahok_core::harness;
use ahok_core::harness::git;
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
  /// `Failed` when a target could not be placed, or under `--check` when
  /// anything is out of date.
  pub fn run(self) -> Result<cli::Status, cli::Error> {
    let platform = self.platform.unwrap_or_else(link::Platform::current);
    let mut rows = Vec::new();
    let mut agents_in: Option<path::PathBuf> = None;
    for repo in &self.repos {
      let repo = path::absolute(repo).map_err(|source| cli::Error::Read {
        path: repo.clone(),
        source,
      })?;
      let manifest = manifest::load(&repo)?;
      let mut links = manifest.link;
      let mut configs = Vec::new();
      let mut wires_git = false;
      if let Some(agents) = &manifest.agents {
        if let Some(first) = agents_in {
          return Err(cli::Error::TwoAgents {
            first,
            second: repo,
          });
        }
        links.extend(agents.links());
        configs = agents.configs(&repo, &manifest.check)?;
        wires_git = agents.harnesses().contains(&harness::Harness::Git);
        agents_in = Some(repo.clone());
      }
      rows.extend(link::apply(&repo, &links, platform, self.check));
      rows.extend(link::write(&configs, self.check));
      if wires_git {
        rows.extend(link::write_scripts(&git::shims(&repo), self.check));
        rows.push(git::hooks_path(self.check));
      }
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
      eprintln!("\n{failed} target(s) could not be placed.");
      return Ok(cli::Status::Failed);
    }
    if stale > 0 {
      eprintln!("\n{stale} target(s) out of date; run `ahok setup`.");
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
    link::Outcome::Written => "written".to_owned(),
    link::Outcome::Would(action) => format!("would {action}"),
    link::Outcome::Missing => "MISSING".to_owned(),
    link::Outcome::Conflict => "CONFLICT".to_owned(),
    link::Outcome::Failed => "FAILED".to_owned(),
  }
}
