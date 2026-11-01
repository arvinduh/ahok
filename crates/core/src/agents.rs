//! The `[agents]` table: which harnesses a workspace wires up, and with what.
//!
//! This module owns the `[agents]` schema and turning it into links and
//! config keys. Where each harness reads its files belongs to the harness
//! adapters; applying the result belongs to `ahok setup`.

use std::path;

use serde;

use crate::harness;
use crate::link;

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
      if let Some(from) = &self.instructions {
        links.push(link::Link::new(from.clone(), harness.instructions()));
      }
      if let Some(dir) = &self.skills {
        let from = format!("{}/*", dir.trim_end_matches('/'));
        links.push(link::Link::new(from, harness.skills()));
      }
    }
    links
  }
}
