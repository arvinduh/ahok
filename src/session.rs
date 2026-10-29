//! Per-session memory across hook calls.
//!
//! ahok runs once per hook call, so anything a session must remember (skills
//! it loaded, once-per-session rules that already fired) lives in
//! `sessions/<session_id>.json` under the state directory. This module owns
//! that file; deciding what to remember belongs to the hook command.

use std::collections;
use std::fs;
use std::io;
use std::path;

use serde_json;

/// What one harness session has done so far.
#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct State {
  /// Skills the session loaded.
  skills: collections::BTreeSet<String>,
  /// Once-per-session rules that already fired.
  fired: collections::BTreeSet<String>,
}

impl State {
  /// Reads the state of session `id`; a session with no file starts empty.
  ///
  /// # Errors
  ///
  /// Any I/O error other than a missing file, or `InvalidData` for a file
  /// that isn't valid state JSON.
  pub fn load(dir: &path::Path, id: &str) -> io::Result<Self> {
    match fs::read(file(dir, id)) {
      Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
      Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
      Err(err) => Err(err),
    }
  }

  /// Writes this state for session `id`, creating the sessions directory.
  ///
  /// # Errors
  ///
  /// Any I/O error creating the directory or writing the file.
  pub fn save(&self, dir: &path::Path, id: &str) -> io::Result<()> {
    let path = file(dir, id);
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec(self)?)
  }

  /// Whether the session loaded skill `name`.
  #[must_use]
  pub fn has_skill(&self, name: &str) -> bool {
    self.skills.contains(name)
  }

  /// Records that the session loaded skill `name`.
  pub fn add_skill(&mut self, name: &str) {
    self.skills.insert(name.to_owned());
  }

  /// Whether once-per-session rule `id` already fired in this session.
  #[must_use]
  pub fn has_fired(&self, id: &str) -> bool {
    self.fired.contains(id)
  }

  /// Records that once-per-session rule `id` fired.
  pub fn add_fired(&mut self, id: &str) {
    self.fired.insert(id.to_owned());
  }
}

/// The state file of session `id`, keeping only safe file-name characters
/// so a crafted id can't escape the sessions directory.
fn file(dir: &path::Path, id: &str) -> path::PathBuf {
  let name: String = id
    .chars()
    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    .collect();
  dir.join("sessions").join(format!("{name}.json"))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn round_trips_and_starts_empty() {
    let dir = std::env::temp_dir().join("ahok-session-test");
    drop(fs::remove_dir_all(&dir));
    let mut state = State::load(&dir, "s1").unwrap();
    assert!(!state.has_skill("rust-guide"));
    state.add_skill("rust-guide");
    state.add_fired("skill-dispatch-rust");
    state.save(&dir, "s1").unwrap();
    let state = State::load(&dir, "s1").unwrap();
    assert!(state.has_skill("rust-guide"));
    assert!(state.has_fired("skill-dispatch-rust"));
  }

  #[test]
  fn id_cannot_escape_the_directory() {
    let path = file(path::Path::new("/state"), "../../etc/passwd");
    assert_eq!(path, path::Path::new("/state/sessions/etcpasswd.json"));
  }
}
