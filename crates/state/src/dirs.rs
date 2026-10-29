//! Where ahok reads config and writes state.
//!
//! This module owns the environment variables and per-OS conventions that
//! locate the agents config directory and the state directory. Writing files
//! there belongs to `journal`.

use std::env;
use std::path;

/// The agents config directory: `$AHOK_CONFIG_DIR`, else `~/.agents`.
///
/// `None` when neither the variable nor a home directory is set.
#[must_use]
pub fn config() -> Option<path::PathBuf> {
  var("AHOK_CONFIG_DIR").or_else(|| Some(env::home_dir()?.join(".agents")))
}

/// The state directory for logs and session files.
///
/// `%LOCALAPPDATA%\ahok` on Windows; elsewhere `$XDG_STATE_HOME/ahok`, else
/// `~/.local/state/ahok`. `None` when no base directory is known.
#[must_use]
pub fn state() -> Option<path::PathBuf> {
  let base = if cfg!(windows) {
    var("LOCALAPPDATA")
  } else {
    var("XDG_STATE_HOME")
      .or_else(|| Some(env::home_dir()?.join(".local").join("state")))
  };
  base.map(|base| base.join("ahok"))
}

/// A non-empty environment variable, as a path.
fn var(name: &str) -> Option<path::PathBuf> {
  env::var_os(name)
    .filter(|value| !value.is_empty())
    .map(path::PathBuf::from)
}
