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
use ahok_core::remote;
use clap;

use crate::cli;

/// Arguments for `ahok setup`.
#[derive(clap::Args)]
pub struct Args {
  /// Workspaces holding an `ahok.toml`: paths, or git URLs cloned to
  /// `~/.<name>` and fast-forwarded on later runs.
  #[arg(default_value = ".")]
  repos: Vec<String>,
  /// Which targets to link: linux, windows or cloud. Defaults to this
  /// machine, and to cloud when `CLAUDE_CODE_REMOTE` is true.
  #[arg(long)]
  platform: Option<link::Platform>,
  /// Change nothing; print what would change, and fail if anything would.
  #[arg(long)]
  check: bool,
}

impl Args {
  /// Fetches any URLs, loads every manifest, then applies them and prints
  /// the report.
  ///
  /// Every manifest loads before anything is placed, so a broken one
  /// changes nothing. `Failed` when a target could not be placed, or under
  /// `--check` when anything is out of date.
  pub fn run(self) -> Result<cli::Status, cli::Error> {
    let platform = self.platform.unwrap_or_else(link::Platform::current);
    let mut rows = Vec::new();
    let mut workspaces = Vec::new();
    for repo in &self.repos {
      if !remote::is_url(repo) {
        let dir = path::absolute(repo).map_err(|source| cli::Error::Read {
          path: repo.into(),
          source,
        })?;
        workspaces.push(dir);
        continue;
      }
      let dir = remote::checkout(repo)
        .ok_or_else(|| cli::Error::NoCheckout { url: repo.clone() })?;
      let row = remote::sync(repo, &dir, self.check);
      // Under --check a missing clone has no manifest to read yet.
      if dir.exists() && !matches!(row.outcome, link::Outcome::Conflict) {
        workspaces.push(dir);
      }
      rows.push(row);
    }
    let mut loaded = Vec::new();
    let mut agents_in: Option<&path::Path> = None;
    for repo in &workspaces {
      let manifest = manifest::load(repo)?;
      if manifest.agents.is_some() {
        if let Some(first) = agents_in {
          return Err(cli::Error::TwoAgents {
            first: first.to_owned(),
            second: repo.clone(),
          });
        }
        agents_in = Some(repo);
      }
      let configs = match &manifest.agents {
        Some(agents) => agents.configs(repo, &manifest.check)?,
        None => Vec::new(),
      };
      loaded.push((repo, manifest, configs));
    }
    for (repo, manifest, configs) in loaded {
      let mut links = manifest.link;
      let mut wires_git = false;
      if let Some(agents) = &manifest.agents {
        links.extend(agents.links());
        wires_git = agents.harnesses().contains(&harness::Harness::Git);
      }
      rows.extend(link::apply(repo, &links, platform, self.check));
      rows.extend(link::write(&configs, self.check));
      if wires_git {
        rows.extend(link::write_scripts(&git::shims(repo), self.check));
        rows.push(git::hooks_path(self.check));
      }
    }
    Ok(report(&rows))
  }
}

/// Prints one line per row and a summary; `Failed` when any row failed or,
/// under `--check`, would change.
fn report(rows: &[link::Row]) -> cli::Status {
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
    return cli::Status::Failed;
  }
  if stale > 0 {
    eprintln!("\n{stale} target(s) out of date; run `ahok setup`.");
    return cli::Status::Failed;
  }
  cli::Status::Clean
}

/// The report label: lowercase for fine, uppercase for failures.
fn label(outcome: &link::Outcome) -> String {
  match outcome {
    link::Outcome::Ok => "ok".to_owned(),
    link::Outcome::Linked => "linked".to_owned(),
    link::Outcome::Written => "written".to_owned(),
    link::Outcome::Synced => "synced".to_owned(),
    link::Outcome::Would(action) => format!("would {action}"),
    link::Outcome::Missing => "MISSING".to_owned(),
    link::Outcome::Conflict => "CONFLICT".to_owned(),
    link::Outcome::Failed => "FAILED".to_owned(),
  }
}
