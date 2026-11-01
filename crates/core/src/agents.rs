//! The `[agents]` table: which harnesses a workspace wires up, and with what.
//!
//! This module owns the `[agents]` schema and resolving its paths against
//! the workspace. Where each harness reads these files belongs to the
//! harness adapters; applying them belongs to `ahok setup`.

use std::path;

use serde;

/// The `[agents]` table, exactly as written.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agents {
  /// Directory of `*.toml` rule files, relative to the workspace.
  rules: Option<String>,
}

impl Agents {
  /// The rules directory inside `workspace`, if the table names one.
  #[must_use]
  pub fn rules(&self, workspace: &path::Path) -> Option<path::PathBuf> {
    self.rules.as_ref().map(|dir| workspace.join(dir))
  }
}
