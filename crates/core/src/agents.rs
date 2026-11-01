//! The `[agents]` table: which harnesses a workspace wires up, and with what.
//!
//! This module owns the `[agents]` schema and turning it into links and
//! config keys. Where each harness reads its files belongs to the harness
//! adapters; applying the result belongs to `ahok setup`.

use std::fs;
use std::io;
use std::path;

use serde;
use serde_json;
use thiserror;

use crate::check;
use crate::harness;
use crate::harness::claude;
use crate::harness::codex;
use crate::link;

/// Why a harness config could not be built.
#[derive(Debug, thiserror::Error)]
pub enum Error {
  /// The `claude_settings` file could not be read.
  #[error("{}: {source}", path.display())]
  Read {
    /// The settings file.
    path: path::PathBuf,
    /// Why reading failed.
    source: io::Error,
  },
  /// The `claude_settings` file is not a JSON object.
  #[error("{}: {source}", path.display())]
  Parse {
    /// The settings file.
    path: path::PathBuf,
    /// What the parser rejected.
    source: serde_json::Error,
  },
}

/// The `[agents]` table, exactly as written.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agents {
  /// The harnesses to wire up.
  #[serde(default)]
  harnesses: Vec<harness::Harness>,
  /// The instructions file every harness reads, relative to the workspace.
  instructions: Option<String>,
  /// Directory of skills, one per child, relative to the workspace.
  skills: Option<String>,
  /// Directory of `*.toml` rule files, relative to the workspace.
  rules: Option<String>,
  /// JSON object whose keys ahok sets in Claude Code's settings, relative
  /// to the workspace.
  claude_settings: Option<String>,
}

impl Agents {
  /// The rules directory inside `workspace`, if the table names one.
  #[must_use]
  pub fn rules(&self, workspace: &path::Path) -> Option<path::PathBuf> {
    self.rules.as_ref().map(|dir| workspace.join(dir))
  }

  /// Links the instructions and each skill into every harness's home.
  #[must_use]
  pub fn links(&self) -> Vec<link::Link> {
    let mut links = Vec::new();
    for harness in &self.harnesses {
      if let (Some(from), Some(to)) =
        (&self.instructions, harness.instructions())
      {
        links.push(link::Link::new(from.clone(), to));
      }
      if let (Some(dir), Some(to)) = (&self.skills, harness.skills()) {
        let from = format!("{}/*", dir.trim_end_matches('/'));
        links.push(link::Link::new(from, to));
      }
    }
    links
  }

  /// The config keys that wire each harness's hooks to `workspace`.
  ///
  /// Claude Code gets `claude_settings` plus `hooks`, and Codex gets
  /// `hooks`. Each registers only the events the rules and `checks` need,
  /// so an empty `hooks` clears stale entries. Antigravity gets none until
  /// its hook format is known.
  ///
  /// # Errors
  ///
  /// `Error::Read` or `Error::Parse` when `claude_settings` is unreadable
  /// or not a JSON object.
  pub fn configs(
    &self,
    workspace: &path::Path,
    checks: &[check::Check],
  ) -> Result<Vec<link::Json>, Error> {
    let wiring = harness::Wiring {
      workspace,
      rules: self.rules.is_some(),
      edit: checks.iter().any(|check| check.runs_on(check::On::Edit)),
      stop: checks.iter().any(|check| check.runs_on(check::On::Stop)),
    };
    let mut configs = Vec::new();
    for harness in &self.harnesses {
      let (to, mut keys, hooks) = match harness {
        harness::Harness::Claude => (
          claude::SETTINGS,
          self.claude_settings(workspace)?,
          claude::hooks(&wiring),
        ),
        harness::Harness::Codex => {
          (codex::HOOKS, serde_json::Map::new(), codex::hooks(&wiring))
        }
        harness::Harness::Agy | harness::Harness::Git => continue,
      };
      keys.insert("hooks".to_owned(), hooks);
      configs.push(link::Json {
        to: to.to_owned(),
        keys,
      });
    }
    Ok(configs)
  }

  /// The keys of the `claude_settings` file, or none when unset.
  fn claude_settings(
    &self,
    workspace: &path::Path,
  ) -> Result<serde_json::Map<String, serde_json::Value>, Error> {
    let Some(file) = &self.claude_settings else {
      return Ok(serde_json::Map::new());
    };
    let path = workspace.join(file);
    let text = fs::read_to_string(&path).map_err(|source| Error::Read {
      path: path.clone(),
      source,
    })?;
    serde_json::from_str(&text).map_err(|source| Error::Parse { path, source })
  }
}
