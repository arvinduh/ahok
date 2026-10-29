//! Symlinks a config repo's files into place, from its `links.toml`.
//!
//! This module owns the `links.toml` schema, target path expansion, and the
//! safe link step: it never overwrites a real file, moving anything in the
//! way to `<target>.bak-<stamp>` first. It returns one row per link as data;
//! printing them belongs to `ahok link`.

use std::env;
use std::fs;
use std::io;
use std::path;
use std::process;
use std::time;

use regex;
use serde;
use thiserror;
use toml;

/// Why a repo's links could not be planned.
#[derive(Debug, thiserror::Error)]
pub enum Error {
  /// `links.toml` could not be read.
  #[error("{}: {source}", path.display())]
  Read {
    /// The links file.
    path: path::PathBuf,
    /// Why reading failed.
    source: io::Error,
  },
  /// `links.toml` is not valid TOML or does not match the schema.
  #[error("{}: {source}", path.display())]
  Parse {
    /// The links file.
    path: path::PathBuf,
    /// What the parser rejected.
    source: toml::de::Error,
  },
}

/// Which machine the links are for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
  /// Linux and WSL.
  Linux,
  /// Native Windows.
  Windows,
  /// Claude Code cloud sessions: only `cloud = true` entries, at the Linux
  /// target.
  Cloud,
}

impl std::str::FromStr for Platform {
  type Err = String;

  fn from_str(name: &str) -> Result<Self, Self::Err> {
    match name {
      "linux" => Ok(Self::Linux),
      "windows" => Ok(Self::Windows),
      "cloud" => Ok(Self::Cloud),
      _ => Err(format!("expected linux, windows or cloud, got {name}")),
    }
  }
}

impl Platform {
  /// The platform this process runs on; `Cloud` when `CLAUDE_CODE_REMOTE` is
  /// `true`.
  #[must_use]
  pub fn current() -> Self {
    if cfg!(windows) {
      Self::Windows
    } else if env::var("CLAUDE_CODE_REMOTE").is_ok_and(|v| v == "true") {
      Self::Cloud
    } else {
      Self::Linux
    }
  }
}

/// The whole `links.toml` file.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
  /// Every `[[link]]` table, in order.
  #[serde(default)]
  link: Vec<Entry>,
}

/// One `[[link]]` table, exactly as written.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
  /// Path inside the repo, `/`-separated. A trailing `/*` links each child
  /// into the target directory instead of the source itself.
  source: String,
  /// Target on both Linux and Windows.
  home: Option<String>,
  /// Target override for Linux.
  linux: Option<String>,
  /// Target override for Windows.
  windows: Option<String>,
  /// Also link in cloud sessions, at the Linux target.
  #[serde(default)]
  cloud: bool,
}

/// How one link came out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
  /// Already a link to the right source.
  Ok,
  /// Linked now.
  Linked,
  /// Dry run: what would happen (`create`, `backup` or `relink`).
  Would(&'static str),
  /// The source is not in the repo.
  Missing,
  /// The target's parent resolves into the repo, so linking would write
  /// inside it.
  Conflict,
  /// The target could not be expanded or the OS refused the link.
  Failed,
}

impl Outcome {
  /// Whether this outcome counts as a failure.
  #[must_use]
  pub fn failed(&self) -> bool {
    matches!(self, Self::Missing | Self::Conflict | Self::Failed)
  }
}

/// One line of the report.
#[derive(Debug)]
pub struct Row {
  /// How it came out.
  pub outcome: Outcome,
  /// The target path, or the raw target when it could not be expanded.
  pub target: String,
  /// Detail for the reader, possibly empty.
  pub note: String,
}

/// Links every entry of `<repo>/links.toml` for `platform`.
///
/// # Errors
///
/// When `links.toml` is missing or invalid. Per-link problems are rows,
/// not errors.
///
/// # Side Effects
///
/// Unless `dry_run`, creates symlinks and parent directories, and renames
/// files in the way to `<name>.bak-<unix seconds>`.
pub fn run(
  repo: &path::Path,
  platform: Platform,
  dry_run: bool,
) -> Result<Vec<Row>, Error> {
  let path = repo.join("links.toml");
  let text = fs::read_to_string(&path).map_err(|source| Error::Read {
    path: path.clone(),
    source,
  })?;
  let file: File =
    toml::from_str(&text).map_err(|source| Error::Parse { path, source })?;
  let stamp = time::SystemTime::now()
    .duration_since(time::UNIX_EPOCH)
    .map_or(0, |d| d.as_secs())
    .to_string();
  let mut rows = Vec::new();
  for entry in &file.link {
    let Some(raw) = entry.target(platform) else {
      continue;
    };
    let target = match expand(raw) {
      Ok(target) => target,
      Err(note) => {
        rows.push(Row {
          outcome: Outcome::Failed,
          target: raw.to_owned(),
          note,
        });
        continue;
      }
    };
    for (source, target) in pairs(repo, &entry.source, &target) {
      let (outcome, note) =
        match link_one(repo, &source, &target, dry_run, &stamp) {
          Ok(result) => result,
          Err(err) => (Outcome::Failed, refusal(&err)),
        };
      rows.push(Row {
        outcome,
        target: target.display().to_string(),
        note,
      });
    }
  }
  Ok(rows)
}

impl Entry {
  /// The raw target for `platform`, or `None` when the entry skips it.
  fn target(&self, platform: Platform) -> Option<&str> {
    let linux = self.linux.as_ref().or(self.home.as_ref());
    match platform {
      Platform::Linux => linux,
      Platform::Cloud => linux.filter(|_| self.cloud),
      Platform::Windows => self.windows.as_ref().or(self.home.as_ref()),
    }
    .map(String::as_str)
  }
}

/// Expands a leading `~` and every `%NAME%` in a target.
///
/// `%DOCUMENTS%` falls back to the Windows Documents folder, following
/// folder redirection. Errors name the variable that is not set.
fn expand(raw: &str) -> Result<path::PathBuf, String> {
  let pattern = regex::Regex::new(r"%(\w+)%").expect("valid regex");
  let mut missing = None;
  let expanded = pattern.replace_all(raw, |caps: &regex::Captures<'_>| {
    let name = &caps[1];
    let value = env::var(name).ok().filter(|v| !v.is_empty());
    let value = value.or_else(|| (name == "DOCUMENTS").then(documents)?);
    value.unwrap_or_else(|| {
      missing.get_or_insert_with(|| format!("%{name}% is not set"));
      String::new()
    })
  });
  if let Some(note) = missing {
    return Err(note);
  }
  match expanded.strip_prefix('~') {
    Some(rest) if rest.is_empty() || rest.starts_with('/') => {
      let home = env::home_dir().ok_or("no home directory")?;
      Ok(home.join(native(rest.trim_start_matches('/'))))
    }
    _ => Ok(native(&expanded)),
  }
}

/// Turns a `/`-separated path into a native one.
fn native(path: &str) -> path::PathBuf {
  path.split('/').filter(|part| !part.is_empty()).fold(
    path::PathBuf::from(if path.starts_with('/') { "/" } else { "" }),
    |acc, part| acc.join(part),
  )
}

/// The Windows Documents folder, read from the registry with `reg query`.
fn documents() -> Option<String> {
  if !cfg!(windows) {
    return None;
  }
  let output = process::Command::new("reg")
    .args([
      "query",
      concat!(
        r"HKCU\Software\Microsoft\Windows\CurrentVersion",
        r"\Explorer\User Shell Folders",
      ),
      "/v",
      "Personal",
    ])
    .output()
    .ok()?;
  let text = String::from_utf8_lossy(&output.stdout);
  let value = text
    .lines()
    .find_map(|line| line.trim().split_once("REG_EXPAND_SZ"))
    .or_else(|| text.lines().find_map(|l| l.trim().split_once("REG_SZ")))?
    .1
    .trim();
  let pattern = regex::Regex::new(r"%(\w+)%").expect("valid regex");
  let value = pattern.replace_all(value, |caps: &regex::Captures<'_>| {
    env::var(&caps[1]).unwrap_or_default()
  });
  Some(value.into_owned())
}

/// (source, target) pairs for one entry, with `dir/*` expanded.
fn pairs(
  repo: &path::Path,
  source: &str,
  target: &path::Path,
) -> Vec<(path::PathBuf, path::PathBuf)> {
  let Some(dir) = source.strip_suffix("/*") else {
    return vec![(repo.join(native(source)), target.to_owned())];
  };
  let dir = repo.join(native(dir));
  let Ok(read) = fs::read_dir(&dir) else {
    // Reported as missing by `link_one`.
    return vec![(dir, target.to_owned())];
  };
  let mut children: Vec<_> =
    read.filter_map(|e| Some(e.ok()?.path())).collect();
  children.sort();
  children
    .into_iter()
    .filter_map(|child| {
      let name = child.file_name()?.to_owned();
      Some((child, target.join(name)))
    })
    .collect()
}

/// Links `target` to `source`; returns the outcome and a note.
fn link_one(
  repo: &path::Path,
  source: &path::Path,
  target: &path::Path,
  dry_run: bool,
  stamp: &str,
) -> io::Result<(Outcome, String)> {
  let shown = source.strip_prefix(repo).unwrap_or(source).display();
  if !source.exists() {
    return Ok((Outcome::Missing, format!("no {shown} in repo")));
  }
  if same_link(target, source) {
    return Ok((Outcome::Ok, "already linked".to_owned()));
  }
  // A parent that resolves into the repo (an old stow fold) would make every
  // write below land inside the repo, and the backup would move a tracked
  // file.
  if inside(target.parent(), repo) {
    let note = "parent dir is a link into the repo; run `stow -D` once";
    return Ok((Outcome::Conflict, note.to_owned()));
  }
  let name = target.file_name().unwrap_or_default().to_string_lossy();
  let backup = target.with_file_name(format!("{name}.bak-{stamp}"));
  let is_link = target.symlink_metadata().is_ok_and(|m| m.is_symlink());
  let (action, note) = if is_link {
    ("relink", "replaced stale link".to_owned())
  } else if target.exists() {
    ("backup", format!("saved {name}.bak-{stamp}"))
  } else {
    ("create", String::new())
  };
  if dry_run {
    return Ok((Outcome::Would(action), format!("-> {shown}")));
  }
  // Create the link under a temporary name first: if the OS refuses it,
  // nothing has been moved or deleted yet.
  if let Some(parent) = target.parent() {
    fs::create_dir_all(parent)?;
  }
  let staged = target.with_file_name(format!("{name}.link-{stamp}"));
  symlink(source, &staged)?;
  let swapped = match action {
    "relink" => remove_link(target),
    "backup" => fs::rename(target, &backup),
    _ => Ok(()),
  }
  .and_then(|()| fs::rename(&staged, target));
  if let Err(err) = swapped {
    drop(remove_link(&staged));
    return Err(err);
  }
  Ok((Outcome::Linked, note))
}

/// Whether `target` is already a symlink to `source`.
fn same_link(target: &path::Path, source: &path::Path) -> bool {
  let Ok(current) = fs::read_link(target) else {
    return false;
  };
  let current = current.to_string_lossy();
  let current = current.strip_prefix(r"\\?\").unwrap_or(&current);
  normalize(current) == normalize(&source.to_string_lossy())
}

/// A path string for comparison: one separator, case-folded on Windows.
fn normalize(path: &str) -> String {
  if cfg!(windows) {
    path.replace('/', "\\").to_lowercase()
  } else {
    path.to_owned()
  }
}

/// Whether `path` resolves to somewhere inside `repo`.
fn inside(path: Option<&path::Path>, repo: &path::Path) -> bool {
  let (Some(Ok(path)), Ok(repo)) =
    (path.map(fs::canonicalize), fs::canonicalize(repo))
  else {
    return false;
  };
  path.starts_with(repo)
}

/// A note for a refused link, with the Windows fix spelled out.
fn refusal(err: &io::Error) -> String {
  // ERROR_PRIVILEGE_NOT_HELD: symlinks need Developer Mode or elevation.
  if cfg!(windows) && err.raw_os_error() == Some(1314) {
    return "no symlink permission: turn on Developer Mode, or run elevated"
      .to_owned();
  }
  err.to_string()
}

/// Creates a symlink at `link` pointing to `source`.
#[cfg(unix)]
fn symlink(source: &path::Path, link: &path::Path) -> io::Result<()> {
  std::os::unix::fs::symlink(source, link)
}

/// Creates a symlink at `link` pointing to `source`.
#[cfg(windows)]
fn symlink(source: &path::Path, link: &path::Path) -> io::Result<()> {
  if source.is_dir() {
    std::os::windows::fs::symlink_dir(source, link)
  } else {
    std::os::windows::fs::symlink_file(source, link)
  }
}

/// Removes a symlink, whether it points to a file or a directory.
fn remove_link(link: &path::Path) -> io::Result<()> {
  // A directory symlink on Windows is removed like a directory.
  if cfg!(windows) && link.is_dir() {
    fs::remove_dir(link)
  } else {
    fs::remove_file(link)
  }
}
