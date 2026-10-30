//! Built-in checks the hook runs after edits and at the end of a turn.
//!
//! This crate runs `fml`, `cargo` and `basedpyright` as subprocesses and
//! returns their failures as data. How a failure is shown to the agent
//! belongs to the harness adapters.

#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::ffi;
use std::io;
use std::iter;
use std::path;
use std::process;

/// Lines of output kept per failure, from the end.
const TAIL_LINES: usize = 40;

/// A check that failed, with the end of its output.
#[derive(Debug)]
pub struct Failure {
  /// The command line that failed, so the agent can rerun it.
  pub command: String,
  /// The last lines of its stdout, then stderr.
  pub output: String,
}

/// Formats `paths` with `fml fmt`.
///
/// Returns `None` on success, and also when `fml` is not installed, which
/// `ahok doctor` reports instead of every edit.
///
/// # Side Effects
///
/// Runs `fml`, which rewrites the files.
#[must_use]
pub fn format(cwd: &path::Path, paths: &[path::PathBuf]) -> Option<Failure> {
  let mut command = process::Command::new("fml");
  command.arg("fmt").args(paths);
  run(cwd, command)
}

/// Runs the end-of-turn gate in `cwd` and returns every failure.
///
/// Always runs `fml lint --changed`; adds `cargo check` when a `.rs` file
/// changed and `basedpyright` when a `.py` file changed, per `git status`.
/// Runs nothing outside a git repository, where nothing can have changed.
///
/// # Side Effects
///
/// Runs `git`, `fml`, and possibly `cargo` and `basedpyright`.
#[must_use]
pub fn gate(cwd: &path::Path) -> Vec<Failure> {
  let Some(changed) = changed_files(cwd) else {
    return Vec::new();
  };
  let changed_ext = |ext: &str| {
    changed
      .iter()
      .any(|file| path::Path::new(file).extension() == Some(ext.as_ref()))
  };
  let mut checks = vec![command("fml", &["lint", "--changed"])];
  if changed_ext("rs") {
    checks.push(command("cargo", &["check", "--message-format=short"]));
  }
  if changed_ext("py") {
    checks.push(command("basedpyright", &[]));
  }
  checks
    .into_iter()
    .filter_map(|check| run(cwd, check))
    .collect()
}

/// Paths `git status` reports as changed or untracked in `cwd`.
///
/// Returns `None` when `git status` fails, as it does outside a repository.
fn changed_files(cwd: &path::Path) -> Option<Vec<String>> {
  let mut git = command("git", &["status", "--porcelain"]);
  in_dir(&mut git, cwd);
  let output = git.output().ok().filter(|output| output.status.success())?;
  let files = String::from_utf8_lossy(&output.stdout)
    .lines()
    .filter_map(|line| line.get(3..))
    .map(|file| file.rsplit(" -> ").next().unwrap_or(file).to_owned())
    .collect();
  Some(files)
}

/// Builds `program args...`.
fn command(program: &str, args: &[&str]) -> process::Command {
  let mut command = process::Command::new(program);
  command.args(args);
  command
}

/// Runs `command` in `cwd` and returns its failure, if any.
///
/// A program that is not installed is not a failure.
fn run(cwd: &path::Path, mut command: process::Command) -> Option<Failure> {
  in_dir(&mut command, cwd);
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

/// Runs `command` in `cwd`, unless `cwd` is empty (the harness didn't say).
fn in_dir(command: &mut process::Command, cwd: &path::Path) {
  if !cwd.as_os_str().is_empty() {
    command.current_dir(cwd);
  }
}

/// The last `TAIL_LINES` lines of `text`.
fn tail(text: &str) -> String {
  let lines: Vec<_> = text.trim_end().lines().collect();
  lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[cfg(unix)]
  #[test]
  fn failing_command_keeps_output_tail() {
    let script = "seq 1 50; echo boom >&2; exit 1";
    let failure = run(path::Path::new(""), command("sh", &["-c", script]));
    let failure = failure.unwrap();
    assert!(failure.command.starts_with("sh -c"));
    assert_eq!(failure.output.lines().count(), TAIL_LINES);
    assert!(failure.output.ends_with("50\nboom"));
  }

  #[cfg(unix)]
  #[test]
  fn passing_command_is_not_a_failure() {
    assert!(run(path::Path::new(""), command("true", &[])).is_none());
  }

  #[test]
  fn gate_passes_outside_a_git_repo() {
    let dir = tempfile::tempdir().expect("create temp dir");
    assert!(
      gate(dir.path()).is_empty(),
      "gate ran checks outside a repo"
    );
  }

  #[test]
  fn missing_program_is_not_a_failure() {
    let missing = command("ahok-no-such-program", &[]);
    assert!(run(path::Path::new(""), missing).is_none());
  }
}
