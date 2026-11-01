//! The append-only error log in the state directory.
//!
//! This module owns the on-disk format of `errors.log`, where hook calls
//! that failed open record why. Locating the directory belongs to `dirs`.

use std::fs;
use std::io;
use std::path;

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
  let mut file = fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(path)?;
  io::Write::write_all(&mut file, bytes)
}
