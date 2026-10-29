//! Append-only logs in the state directory.
//!
//! This module owns the on-disk format of `decisions.jsonl`, one JSON object
//! per hook call, and of `errors.log`. Locating the directory belongs to
//! `dirs`; reading the decisions back belongs to `ahok stats`.

use std::fs;
use std::io;
use std::io::Write;
use std::path;

use serde_json;

use crate::rules;

/// One hook call, as logged to `decisions.jsonl`.
#[derive(serde::Serialize)]
pub struct Decision<'a> {
  /// When the call started, in milliseconds since the Unix epoch.
  pub unix_ms: u64,
  /// Harness name from the command line.
  pub harness: &'a str,
  /// Event name from the command line.
  pub event: &'a str,
  /// Id of the deciding rule; absent when the call was allowed.
  pub rule: Option<&'a str>,
  /// The decision; absent means allow.
  pub decision: Option<rules::Decision>,
  /// Time spent in ahok, in microseconds.
  pub micros: u64,
}

/// Appends `entry` as one line of `dir/decisions.jsonl`.
///
/// # Errors
///
/// Any I/O error creating `dir` or writing the file.
pub fn decision(dir: &path::Path, entry: &Decision) -> io::Result<()> {
  let mut line = serde_json::to_vec(entry)?;
  line.push(b'\n');
  append(&dir.join("decisions.jsonl"), &line)
}

/// Appends `message` as one line of `dir/errors.log`, after `unix_ms`.
///
/// # Errors
///
/// Any I/O error creating `dir` or writing the file.
pub fn error(dir: &path::Path, unix_ms: u64, message: &str) -> io::Result<()> {
  let line = format!("{unix_ms} {message}\n");
  append(&dir.join("errors.log"), line.as_bytes())
}

/// Appends `bytes` to `path` in one write, creating its directory.
fn append(path: &path::Path, bytes: &[u8]) -> io::Result<()> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent)?;
  }
  fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(path)?
    .write_all(bytes)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn appends_one_json_line_per_decision() {
    let dir = std::env::temp_dir().join("ahok-journal-test");
    let _ = fs::remove_dir_all(&dir);
    let entry = Decision {
      unix_ms: 1,
      harness: "claude",
      event: "PreToolUse",
      rule: Some("format-via-fml"),
      decision: Some(rules::Decision::Deny),
      micros: 420,
    };
    decision(&dir, &entry).unwrap();
    decision(&dir, &entry).unwrap();
    let text = fs::read_to_string(dir.join("decisions.jsonl")).unwrap();
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(
      lines[0],
      r#"{"unix_ms":1,"harness":"claude","event":"PreToolUse","rule":"format-via-fml","decision":"deny","micros":420}"#
    );
  }
}
