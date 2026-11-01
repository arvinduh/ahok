//! Workspaces named by a git URL: where they live, and keeping them current.
//!
//! This module owns turning `ahok setup <url>` into a local checkout at
//! `~/.<name>`. Reading the checkout's manifest belongs to `manifest`.

use std::path;
use std::process;

use crate::link;

/// Whether `arg` names a git remote rather than a local path.
#[must_use]
pub fn is_url(arg: &str) -> bool {
  arg.contains("://") || (arg.starts_with("git@") && arg.contains(':'))
}

/// The checkout for `url`: `~/.<name>`, where `name` is its last path
/// segment without `.git`. `None` without a home directory or a name.
#[must_use]
pub fn checkout(url: &str) -> Option<path::PathBuf> {
  let trimmed = url.trim_end_matches('/');
  let name = trimmed.rsplit(['/', ':']).next()?;
  let name = name.strip_suffix(".git").unwrap_or(name);
  if name.is_empty() {
    return None;
  }
  Some(std::env::home_dir()?.join(format!(".{name}")))
}

/// Clones `url` into `dir`, or fast-forwards `dir` when it already holds
/// that remote.
///
/// A `dir` that holds another remote, or isn't a checkout, is a conflict
/// and left alone. Under `dry_run`, nothing is fetched.
///
/// # Side Effects
///
/// Unless `dry_run`, runs `git clone` or `git pull --ff-only`.
#[must_use]
pub fn sync(url: &str, dir: &path::Path, dry_run: bool) -> link::Row {
  let row = |outcome, note: String| link::Row {
    outcome,
    target: dir.display().to_string(),
    note,
  };
  if !dir.exists() {
    if dry_run {
      return row(link::Outcome::Would("clone"), url.to_owned());
    }
    let mut clone = process::Command::new("git");
    clone.args(["clone", "-q", url]).arg(dir);
    return match run(clone) {
      Ok(()) => row(link::Outcome::Synced, "cloned".to_owned()),
      Err(note) => row(link::Outcome::Failed, note),
    };
  }
  let mut origin = process::Command::new("git");
  origin
    .arg("-C")
    .arg(dir)
    .args(["remote", "get-url", "origin"]);
  let origin = origin.output().ok().filter(|out| out.status.success());
  let origin = origin.map(|out| String::from_utf8_lossy(&out.stdout).into());
  if origin.as_deref().map(same_remote) != Some(same_remote(url)) {
    let note = origin.map_or_else(
      || "not a git checkout".to_owned(),
      |origin: String| format!("holds {}", origin.trim()),
    );
    return row(link::Outcome::Conflict, note);
  }
  if dry_run {
    return row(link::Outcome::Ok, String::new());
  }
  let mut pull = process::Command::new("git");
  pull.arg("-C").arg(dir).args(["pull", "-q", "--ff-only"]);
  match run(pull) {
    Ok(()) => row(link::Outcome::Synced, "pulled".to_owned()),
    Err(note) => row(link::Outcome::Failed, note),
  }
}

/// A remote URL with the parts that don't change the repo dropped.
fn same_remote(url: &str) -> &str {
  let url = url.trim().trim_end_matches('/');
  url.strip_suffix(".git").unwrap_or(url)
}

/// Runs `command`; on failure returns its stderr's last line.
fn run(mut command: process::Command) -> Result<(), String> {
  let out = command.output().map_err(|err| err.to_string())?;
  if out.status.success() {
    return Ok(());
  }
  let stderr = String::from_utf8_lossy(&out.stderr);
  Err(
    stderr
      .trim()
      .lines()
      .last()
      .unwrap_or("git failed")
      .to_owned(),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn urls_and_checkouts() {
    assert!(is_url("https://github.com/arvinduh/agents.git"));
    assert!(is_url("git@github.com:arvinduh/agents.git"));
    assert!(!is_url("../agents"));
    let name =
      |url| checkout(url).and_then(|dir| dir.file_name().map(Into::into));
    let agents: Option<std::ffi::OsString> = Some(".agents".into());
    assert_eq!(name("https://github.com/arvinduh/agents.git"), agents);
    assert_eq!(name("git@github.com:arvinduh/agents"), agents);
    assert_eq!(name("https://github.com/arvinduh/agents/"), agents);
    assert_eq!(same_remote("https://x/a.git\n"), same_remote("https://x/a"));
  }
}
