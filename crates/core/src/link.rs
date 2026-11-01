//! Puts a workspace's files into place: symlinks, and keys in JSON configs.
//!
//! This module owns the `[[link]]` schema, target path expansion, and the
//! safe write steps: it never overwrites a real file it doesn't own, moving
//! anything in the way to `<target>.bak-<stamp>` first. Reading `ahok.toml`
//! belongs to `manifest`; printing the rows belongs to `ahok setup`.

use std::env;
use std::fs;
use std::io;
use std::path;
use std::process;
use std::time;

use regex;
use serde;
use serde_json;

/// Which machine the links are for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
  /// Linux and WSL.
  Linux,
  /// Native Windows.
  Windows,
  /// Claude Code cloud sessions, which use the Linux target.
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

/// One `[[link]]` table, exactly as written.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
  /// Path inside the repo, `/`-separated. A trailing `/*` links each child
  /// into the target directory instead of the source itself.
  from: String,
  /// Where the link goes.
  to: To,
  /// Platforms to link on; every platform when unset.
  on: Option<Vec<Platform>>,
}

/// A link's target: one path everywhere, or one per OS.
#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum To {
  /// The same path on every platform.
  Everywhere(String),
  /// A path per OS; an OS without one is skipped.
  PerOs(PerOs),
}

/// A link's target per OS, as `to = { linux = "...", windows = "..." }`.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PerOs {
  /// Target on Linux and in cloud sessions.
  linux: Option<String>,
  /// Target on Windows.
  windows: Option<String>,
}

/// How one link came out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
  /// Already a link to the right source.
  Ok,
  /// Linked now.
  Linked,
  /// Written now.
  Written,
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

/// Links each of `links`, whose sources live in `repo`, for `platform`.
///
/// Returns one row per link; a link that fails is a row, not an error.
///
/// # Side Effects
///
/// Unless `dry_run`, creates symlinks and parent directories, and renames
/// files in the way to `<name>.bak-<unix seconds>`.
#[must_use]
pub fn apply(
  repo: &path::Path,
  links: &[Link],
  platform: Platform,
  dry_run: bool,
) -> Vec<Row> {
  let stamp = time::SystemTime::now()
    .duration_since(time::UNIX_EPOCH)
    .map_or(0, |d| d.as_secs())
    .to_string();
  let mut rows = Vec::new();
  for link in links {
    let Some(raw) = link.target(platform) else {
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
    for (source, target) in pairs(repo, &link.from, &target) {
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
  rows
}

impl Link {
  /// Links `from` to the same `to` on every platform.
  pub(crate) fn new(from: String, to: &str) -> Self {
    Self {
      from,
      to: To::Everywhere(to.to_owned()),
      on: None,
    }
  }

  /// The raw target for `platform`, or `None` when the link skips it.
  fn target(&self, platform: Platform) -> Option<&str> {
    if self.on.as_ref().is_some_and(|on| !on.contains(&platform)) {
      return None;
    }
    match (&self.to, platform) {
      (To::Everywhere(to), _) => Some(to.as_str()),
      (To::PerOs(per_os), Platform::Windows) => per_os.windows.as_deref(),
      (To::PerOs(per_os), _) => per_os.linux.as_deref(),
    }
  }
}

/// Keys ahok owns in a JSON config file; it leaves every other key alone.
#[derive(Debug)]
pub struct Json {
  /// The file, written like a link's `to`.
  pub to: String,
  /// Each key ahok sets, with its value.
  pub keys: serde_json::Map<String, serde_json::Value>,
}

/// Sets each file's keys, keeping the rest of the file; one row per file.
///
/// A symlink at the target is replaced by a real file holding the link's
/// keys, so the write never lands in the link's source. A file that isn't a JSON object is moved to
/// `<name>.bak-<unix seconds>` first.
///
/// # Side Effects
///
/// Unless `dry_run`, writes the files and creates their directories.
#[must_use]
pub fn write(files: &[Json], dry_run: bool) -> Vec<Row> {
  let stamp = time::SystemTime::now()
    .duration_since(time::UNIX_EPOCH)
    .map_or(0, |d| d.as_secs())
    .to_string();
  files
    .iter()
    .map(|file| {
      let target = match expand(&file.to) {
        Ok(target) => target,
        Err(note) => {
          return Row {
            outcome: Outcome::Failed,
            target: file.to.clone(),
            note,
          };
        }
      };
      let (outcome, note) =
        match write_one(&target, &file.keys, dry_run, &stamp) {
          Ok(result) => result,
          Err(err) => (Outcome::Failed, err.to_string()),
        };
      Row {
        outcome,
        target: target.display().to_string(),
        note,
      }
    })
    .collect()
}

/// Sets `keys` in the JSON object at `target`; returns the outcome and a
/// note.
fn write_one(
  target: &path::Path,
  keys: &serde_json::Map<String, serde_json::Value>,
  dry_run: bool,
  stamp: &str,
) -> io::Result<(Outcome, String)> {
  let is_link = target.symlink_metadata().is_ok_and(|m| m.is_symlink());
  // A link is read through, so its keys carry over into the real file.
  let text = match fs::read_to_string(target) {
    Ok(text) => Some(text),
    Err(err) if err.kind() != io::ErrorKind::NotFound && !is_link => {
      return Err(err);
    }
    Err(_) => None,
  };
  let current = text
    .as_deref()
    .map(serde_json::from_str::<serde_json::Value>);
  let (mut object, invalid) = match current {
    Some(Ok(serde_json::Value::Object(object))) => (object, false),
    Some(_) => (serde_json::Map::new(), true),
    None => (serde_json::Map::new(), false),
  };
  let before = object.clone();
  object.extend(keys.clone());
  if !is_link && !invalid && text.is_some() && object == before {
    return Ok((Outcome::Ok, "up to date".to_owned()));
  }
  let name = target.file_name().unwrap_or_default().to_string_lossy();
  let (action, note) = if is_link {
    ("relink", "replaced link with file".to_owned())
  } else if invalid {
    ("backup", format!("saved {name}.bak-{stamp}"))
  } else {
    ("write", String::new())
  };
  if dry_run {
    return Ok((Outcome::Would(action), note));
  }
  if let Some(parent) = target.parent() {
    fs::create_dir_all(parent)?;
  }
  let mut json = serde_json::to_string_pretty(&object)?;
  json.push('\n');
  // Write beside the target first, so a failed write leaves it intact.
  let staged = target.with_file_name(format!("{name}.ahok-{stamp}"));
  fs::write(&staged, json)?;
  let swapped = match action {
    "relink" => remove_link(target),
    "backup" => {
      fs::rename(target, target.with_file_name(format!("{name}.bak-{stamp}")))
    }
    _ => Ok(()),
  }
  .and_then(|()| fs::rename(&staged, target));
  if let Err(err) = swapped {
    drop(fs::remove_file(&staged));
    return Err(err);
  }
  Ok((Outcome::Written, note))
}

/// An executable file ahok owns whole, such as a generated git hook.
#[derive(Debug)]
pub struct Script {
  /// The file, written like a link's `to`.
  pub to: String,
  /// Its full contents.
  pub text: String,
}

/// Writes each script and makes it executable; one row per script.
///
/// ahok owns these files, so a stale one is replaced without a backup.
///
/// # Side Effects
///
/// Unless `dry_run`, writes the files and creates their directories.
#[must_use]
pub fn write_scripts(scripts: &[Script], dry_run: bool) -> Vec<Row> {
  scripts
    .iter()
    .map(|script| {
      let (outcome, target, note) = match expand(&script.to) {
        Ok(target) => {
          let (outcome, note) =
            match write_script(&target, &script.text, dry_run) {
              Ok(outcome) => (outcome, String::new()),
              Err(err) => (Outcome::Failed, err.to_string()),
            };
          (outcome, target.display().to_string(), note)
        }
        Err(note) => (Outcome::Failed, script.to.clone(), note),
      };
      Row {
        outcome,
        target,
        note,
      }
    })
    .collect()
}

/// Writes `text` to `target` as an executable, unless it already holds it.
fn write_script(
  target: &path::Path,
  text: &str,
  dry_run: bool,
) -> io::Result<Outcome> {
  let is_file = target.symlink_metadata().is_ok_and(|m| m.is_file());
  if is_file && fs::read_to_string(target).is_ok_and(|old| old == text) {
    return Ok(Outcome::Ok);
  }
  if dry_run {
    return Ok(Outcome::Would("write"));
  }
  if let Some(parent) = target.parent() {
    fs::create_dir_all(parent)?;
  }
  if target.symlink_metadata().is_ok() && !is_file {
    remove_link(target)?;
  }
  fs::write(target, text)?;
  #[cfg(unix)]
  {
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    fs::set_permissions(target, mode)?;
  }
  Ok(Outcome::Written)
}

/// Expands a leading `~` and every `%NAME%` in a target.
///
/// `%DOCUMENTS%` falls back to the Windows Documents folder, following
/// folder redirection. Errors name the variable that is not set.
pub(crate) fn expand(raw: &str) -> Result<path::PathBuf, String> {
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
  // A directory symlink on Windows must be removed with `remove_dir`.
  // `is_dir()` follows the link and returns false for a broken link, so
  // try `remove_dir` first, then fall back to `remove_file`.
  if cfg!(windows) {
    fs::remove_dir(link).or_else(|_| fs::remove_file(link))
  } else {
    fs::remove_file(link)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn hooks_file(to: &path::Path) -> Json {
    let mut keys = serde_json::Map::new();
    keys.insert("hooks".to_owned(), serde_json::json!({ "Stop": [] }));
    Json {
      to: to.display().to_string(),
      keys,
    }
  }

  fn read(path: &path::Path) -> serde_json::Value {
    let text = fs::read_to_string(path).expect("written file");
    serde_json::from_str(&text).expect("valid JSON")
  }

  #[test]
  fn write_sets_keys_and_keeps_the_rest() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("settings.json");
    fs::write(&target, r#"{"tui":"full","hooks":{"old":1}}"#).expect("seed");
    let files = [hooks_file(&target)];
    let rows = write(&files, false);
    assert!(matches!(rows[0].outcome, Outcome::Written), "{:?}", rows[0]);
    let json = read(&target);
    assert_eq!(json["tui"], "full");
    assert_eq!(json["hooks"], serde_json::json!({ "Stop": [] }));
    let rows = write(&files, false);
    assert!(matches!(rows[0].outcome, Outcome::Ok), "{:?}", rows[0]);
  }

  #[test]
  fn write_creates_a_missing_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("new/hooks.json");
    let files = [hooks_file(&target)];
    let rows = write(&files, true);
    assert!(matches!(rows[0].outcome, Outcome::Would("write")));
    assert!(!target.exists(), "dry run wrote {}", target.display());
    let rows = write(&files, false);
    assert!(matches!(rows[0].outcome, Outcome::Written), "{:?}", rows[0]);
    assert_eq!(read(&target)["hooks"]["Stop"], serde_json::json!([]));
  }

  #[test]
  fn write_backs_up_a_file_that_is_not_an_object() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("settings.json");
    fs::write(&target, "not json").expect("seed");
    let rows = write(&[hooks_file(&target)], false);
    assert!(matches!(rows[0].outcome, Outcome::Written), "{:?}", rows[0]);
    let backups = fs::read_dir(dir.path())
      .expect("list dir")
      .filter_map(Result::ok)
      .filter(|entry| entry.file_name().to_string_lossy().contains(".bak-"))
      .count();
    assert_eq!(backups, 1, "expected one backup");
    assert!(read(&target)["hooks"].is_object(), "hooks set");
  }

  #[cfg(unix)]
  #[test]
  fn write_replaces_a_symlink_with_a_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let source = dir.path().join("repo.json");
    fs::write(&source, r#"{"tui":"full"}"#).expect("seed");
    let target = dir.path().join("settings.json");
    std::os::unix::fs::symlink(&source, &target).expect("link");
    let rows = write(&[hooks_file(&target)], false);
    assert!(matches!(rows[0].outcome, Outcome::Written), "{:?}", rows[0]);
    assert!(
      !target.symlink_metadata().expect("meta").is_symlink(),
      "file"
    );
    assert_eq!(read(&source), serde_json::json!({ "tui": "full" }));
    assert_eq!(read(&target)["tui"], "full");
  }
}
