//! `[[check]]` tables: commands run on an event's changed files.
//!
//! This module owns the check schema, the set of files each event changed,
//! and running the commands. Their failures come back as data; how a
//! failure is shown to the agent belongs to the harness adapters.

use std::ffi;
use std::io;
use std::iter;
use std::path;
use std::process;

use globset;
use serde;

use crate::de;

/// Lines of output kept per failure, from the end.
const TAIL_LINES: usize = 40;

/// The token in `run` replaced by the event's changed files.
const FILES: &str = "{files}";

/// One `[[check]]` table, exactly as written.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
  /// Events the check runs on.
  on: Vec<On>,
  /// The command, split on whitespace; a `{files}` word becomes one
  /// argument per changed file.
  run: String,
  /// Globs a changed file must match; the check is skipped when none does.
  #[serde(default, deserialize_with = "de::globs")]
  changed: Option<globset::GlobSet>,
}

impl Check {
  /// Whether the check runs on `on`.
  pub(crate) fn runs_on(&self, on: On) -> bool {
    self.on.contains(&on)
  }
}

/// An event a check runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum On {
  /// After an agent edits files: the edited files.
  Edit,
  /// When the agent ends its turn: every file changed since the branch's
  /// base, committed or not.
  Stop,
  /// Before git records a commit: the staged files.
  Commit,
}

/// A check that failed, with the end of its output.
#[derive(Debug)]
pub struct Failure {
  /// The command line that failed, so the agent can rerun it.
  pub command: String,
  /// The last lines of its stdout, then stderr.
  pub output: String,
}

/// Runs every check on `on` against `files`, in `cwd`; returns the failures.
///
/// A check is skipped when no file matches its `changed` globs, or when its
/// program is not installed. Nothing runs when `files` is empty.
///
/// # Side Effects
///
/// Runs each check's command, which may rewrite files.
#[must_use]
pub fn run(
  checks: &[Check],
  on: On,
  cwd: &path::Path,
  files: &[path::PathBuf],
) -> Vec<Failure> {
  checks
    .iter()
    .filter(|check| check.runs_on(on))
    .filter_map(|check| {
      let files: Vec<&path::PathBuf> = files
        .iter()
        .filter(|file| check.changed.as_ref().is_none_or(|g| g.is_match(file)))
        .collect();
      if files.is_empty() {
        return None;
      }
      execute(cwd, command(&check.run, &files)?)
    })
    .collect()
}

/// The repo root of `cwd` and every file changed since the branch's base.
///
/// The base is where `HEAD` forked from the remote's default branch, or
/// `HEAD` itself when there is no remote. Untracked files count; deleted
/// files don't. Returns `None` outside a git repository.
///
/// # Side Effects
///
/// Runs `git`.
#[must_use]
pub fn changed_since_base(
  cwd: &path::Path,
) -> Option<(path::PathBuf, Vec<path::PathBuf>)> {
  let root = path::PathBuf::from(git(cwd, &["rev-parse", "--show-toplevel"])?);
  let base = git(&root, &["merge-base", "HEAD", "origin/HEAD"])
    .unwrap_or_else(|| "HEAD".to_owned());
  let tracked = git(&root, &["diff", "--name-only", "--diff-filter=d", &base])
    .unwrap_or_default();
  let untracked = git(&root, &["ls-files", "--others", "--exclude-standard"])
    .unwrap_or_default();
  let files = tracked
    .lines()
    .chain(untracked.lines())
    .map(|file| root.join(file))
    .collect();
  Some((root, files))
}

/// The repo root of `cwd` and the files staged for commit, deleted files
/// aside. Returns `None` outside a git repository.
///
/// # Side Effects
///
/// Runs `git`.
#[must_use]
pub fn staged(cwd: &path::Path) -> Option<(path::PathBuf, Vec<path::PathBuf>)> {
  let root = path::PathBuf::from(git(cwd, &["rev-parse", "--show-toplevel"])?);
  let staged = git(
    &root,
    &["diff", "--cached", "--name-only", "--diff-filter=d"],
  )?;
  let files = staged.lines().map(|file| root.join(file)).collect();
  Some((root, files))
}

/// Runs `git args` in `dir`; returns its trimmed stdout, or `None` if it
/// failed.
fn git(dir: &path::Path, args: &[&str]) -> Option<String> {
  let output = process::Command::new("git")
    .args(args)
    .current_dir(dir)
    .stderr(process::Stdio::null())
    .output()
    .ok()
    .filter(|output| output.status.success())?;
  Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Builds the command `run` names, with `{files}` expanded to `files`.
///
/// Returns `None` for an empty `run`.
fn command(run: &str, files: &[&path::PathBuf]) -> Option<process::Command> {
  let mut words = run.split_whitespace();
  let mut command = process::Command::new(words.next()?);
  for word in words {
    if word == FILES {
      command.args(files);
    } else {
      command.arg(word);
    }
  }
  Some(command)
}

/// Runs `command` in `cwd` and returns its failure, if any.
///
/// A program that is not installed is not a failure.
fn execute(cwd: &path::Path, mut command: process::Command) -> Option<Failure> {
  if !cwd.as_os_str().is_empty() {
    command.current_dir(cwd);
  }
  let output = match command.stdin(process::Stdio::null()).output() {
    Ok(output) if output.status.success() => return None,
    Ok(output) => output,
    Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
    Err(err) => {
      return Some(Failure {
        command: describe(&command),
        output: err.to_string(),
      });
    }
  };
  let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
  text.push_str(&String::from_utf8_lossy(&output.stderr));
  Some(Failure {
    command: describe(&command),
    output: tail(&text),
  })
}

/// The command line `command` runs, for showing to the agent.
fn describe(command: &process::Command) -> String {
  iter::once(command.get_program())
    .chain(command.get_args())
    .map(ffi::OsStr::to_string_lossy)
    .collect::<Vec<_>>()
    .join(" ")
}

/// The last `TAIL_LINES` lines of `text`.
fn tail(text: &str) -> String {
  let lines: Vec<_> = text.trim_end().lines().collect();
  lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n")
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Parses `[[check]]` tables from TOML.
  fn checks(text: &str) -> Vec<Check> {
    #[derive(serde::Deserialize)]
    struct File {
      check: Vec<Check>,
    }
    toml::from_str::<File>(text).expect("valid checks").check
  }

  fn files(names: &[&str]) -> Vec<path::PathBuf> {
    names.iter().map(path::PathBuf::from).collect()
  }

  #[test]
  fn expands_files_in_place() {
    let files = files(&["a.rs", "b.rs"]);
    let refs: Vec<_> = files.iter().collect();
    let command = command("fml lint {files} --quiet", &refs).unwrap();
    assert_eq!(describe(&command), "fml lint a.rs b.rs --quiet");
  }

  #[cfg(unix)]
  #[test]
  fn failing_check_keeps_output_tail() {
    let checks =
      checks("[[check]]\non = [\"stop\"]\nrun = \"sh -c {files}\"\n");
    let script = files(&["seq 1 50; echo boom >&2; exit 1"]);
    let failures = run(&checks, On::Stop, path::Path::new(""), &script);
    assert_eq!(failures.len(), 1, "the script fails");
    assert_eq!(failures[0].output.lines().count(), TAIL_LINES);
    assert!(failures[0].output.ends_with("50\nboom"));
  }

  #[cfg(unix)]
  #[test]
  fn skips_other_events_and_unmatched_files() {
    let checks = checks(concat!(
      "[[check]]\non = [\"edit\"]\nrun = \"false\"\n",
      "[[check]]\non = [\"stop\"]\nrun = \"false\"\nchanged = [\"*.py\"]\n",
    ));
    let cwd = path::Path::new("");
    assert!(run(&checks, On::Stop, cwd, &files(&["a.rs"])).is_empty());
    assert_eq!(run(&checks, On::Edit, cwd, &files(&["a.rs"])).len(), 1);
    assert!(run(&checks, On::Edit, cwd, &[]).is_empty(), "no files");
  }

  #[test]
  fn missing_program_is_not_a_failure() {
    let checks = checks("[[check]]\non = [\"edit\"]\nrun = \"ahok-nope\"\n");
    let cwd = path::Path::new("");
    assert!(run(&checks, On::Edit, cwd, &files(&["a"])).is_empty());
  }

  #[test]
  fn lists_committed_modified_and_untracked_files() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let root = dir.path();
    let run_git = |args: &[&str]| {
      let ok = process::Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .current_dir(root)
        .status()
        .expect("git runs")
        .success();
      assert!(ok, "git {args:?} must succeed");
    };
    run_git(&["init", "-q"]);
    std::fs::write(root.join("a.txt"), "a").expect("write a");
    run_git(&["add", "a.txt"]);
    run_git(&["commit", "-qm", "a"]);
    std::fs::write(root.join("a.txt"), "changed").expect("write a");
    std::fs::write(root.join("b.txt"), "new").expect("write b");
    let (top, mut changed) = changed_since_base(root).expect("a repo");
    changed.sort();
    let names: Vec<_> = changed
      .iter()
      .map(|file| file.strip_prefix(&top).expect("under root"))
      .collect();
    assert_eq!(names, [path::Path::new("a.txt"), path::Path::new("b.txt")]);
    run_git(&["add", "b.txt"]);
    let (top, staged) = staged(root).expect("a repo");
    assert_eq!(staged, [top.join("b.txt")], "only the staged file");
  }

  #[test]
  fn nothing_changed_outside_a_git_repo() {
    let dir = tempfile::tempdir().expect("create temp dir");
    assert!(changed_since_base(dir.path()).is_none());
    assert!(staged(dir.path()).is_none());
  }
}
