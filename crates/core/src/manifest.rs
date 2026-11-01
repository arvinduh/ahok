//! A workspace's `ahok.toml`: its schema and loading.
//!
//! This module owns the top level of the manifest and reading it from a
//! repo. Each section's schema belongs to the module that acts on it, such
//! as `link` for `[[link]]`.

use std::fs;
use std::io;
use std::path;

use serde;
use thiserror;
use toml;

use crate::agents;
use crate::check;
use crate::link;

/// The manifest's file name, at the root of a workspace.
const FILE: &str = "ahok.toml";

/// Why a manifest could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum Error {
  /// The manifest could not be read.
  #[error("{}: {source}", path.display())]
  Read {
    /// The manifest file.
    path: path::PathBuf,
    /// Why reading failed.
    source: io::Error,
  },
  /// The manifest is not valid TOML or does not match the schema.
  #[error("{}: {source}", path.display())]
  Parse {
    /// The manifest file.
    path: path::PathBuf,
    /// What the parser rejected.
    source: toml::de::Error,
  },
}

/// A parsed `ahok.toml`.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
  /// The `[agents]` table; absent in a workspace that wires up no harness.
  pub agents: Option<agents::Agents>,
  /// Every `[[link]]` table, in file order.
  #[serde(default)]
  pub link: Vec<link::Link>,
  /// Every `[[check]]` table, in file order.
  #[serde(default)]
  pub check: Vec<check::Check>,
}

/// Loads `<repo>/ahok.toml`.
///
/// # Errors
///
/// `Error::Read` when the file can't be read, including when it is
/// missing, and `Error::Parse` when it doesn't match the schema.
pub fn load(repo: &path::Path) -> Result<Manifest, Error> {
  let path = repo.join(FILE);
  let text = fs::read_to_string(&path).map_err(|source| Error::Read {
    path: path.clone(),
    source,
  })?;
  toml::from_str(&text).map_err(|source| Error::Parse { path, source })
}
