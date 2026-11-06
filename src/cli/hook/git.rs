//! `ahok hook -C <workspace> git <hook> [args]`: run the rules from a git
//! hook.
//!
//! This module owns the git hook contract: read what git passes (a message
//! file, the staged diff, pushed refs on stdin), print each broken rule to
//! stderr, and fail to stop git. When ahok itself can't load the rules it
//! warns and passes, so a broken setup never blocks work. Deciding belongs
//! to `rules`.

use std::env;
use std::fs;
use std::io;
use std::path;
use std::process;

use ahok_core::agents;
use ahok_core::check;
use ahok_core::manifest;
use ahok_core::rules;

use crate::cli;

/// Runs git hook `name` with git's `args`; `Failed` stops git.
///
/// `commit-msg` checks the message file and the staged diff against the
/// rules, then runs the commit checks on the staged files; `pre-push`
/// checks the refs git writes to stdin. Any other hook passes.
pub fn run(workspace: &path::Path, name: &str, args: &[String]) -> cli::Status {
  let manifest = match manifest::load(workspace) {
    Ok(manifest) => manifest,
    Err(err) => {
      eprintln!("[ahok] git hook skipped: {err}");
      return cli::Status::Clean;
    }
  };
  let rules = load(workspace, manifest.agents.as_ref());
  let var_set = |name: &str| env::var_os(name).is_some();
  let violations = match (name, args.first()) {
    ("commit-msg", Some(file)) => {
      let message = fs::read_to_string(file).unwrap_or_default();
      // A first commit imports what exists, so it skips the diff budget.
      let numstat = has_head().then(staged_numstat).flatten();
      let changes = numstat.as_deref().map(changes);
      rules::commit_msg(&rules, &message, changes.as_deref(), &var_set)
    }
    ("pre-push", _) => {
      let input = io::read_to_string(io::stdin()).ok();
      let refs: Option<Vec<_>> = input.as_deref().map(|input| {
        input
          .lines()
          .filter_map(|line| line.split_whitespace().nth(2))
          .collect()
      });
      rules::pre_push(&rules, refs.as_deref(), &var_set)
    }
    _ => {
      eprintln!("[ahok] git hook `{name}` has no rules; skipped");
      return cli::Status::Clean;
    }
  };
  for violation in &violations {
    eprintln!("[ahok {}] {}", violation.id, violation.reason);
  }
  if !violations.is_empty() {
    return cli::Status::Failed;
  }
  // Checks are slow, so they run only once the rules pass.
  let cwd = env::current_dir().unwrap_or_default();
  let staged = (name == "commit-msg").then(|| check::staged(&cwd));
  let failures = match staged.flatten() {
    Some((root, files)) => {
      check::run(&manifest.check, check::On::Commit, &root, &files)
    }
    None => Vec::new(),
  };
  for failure in &failures {
    eprintln!(
      "[ahok] check failed: {}\n{}",
      failure.command, failure.output
    );
  }
  if failures.is_empty() {
    cli::Status::Clean
  } else {
    cli::Status::Failed
  }
}

/// Loads the workspace's rules; warns and returns none when they fail to
/// load, so a broken setup never blocks git.
fn load(
  workspace: &path::Path,
  agents: Option<&agents::Agents>,
) -> Vec<rules::Rule> {
  let Some(dir) = agents.and_then(|agents| agents.rules(workspace)) else {
    return Vec::new();
  };
  rules::load(&dir).unwrap_or_else(|err| {
    eprintln!("[ahok] git rules skipped: {err}");
    Vec::new()
  })
}

/// Whether `HEAD` names a commit; false before a repository's first.
fn has_head() -> bool {
  process::Command::new("git")
    .args(["rev-parse", "--verify", "--quiet", "HEAD"])
    .stdout(process::Stdio::null())
    .status()
    .is_ok_and(|status| status.success())
}

/// `git diff --cached --numstat` output, or `None` if git failed.
fn staged_numstat() -> Option<String> {
  let output = process::Command::new("git")
    .args(["diff", "--cached", "--numstat", "--no-renames"])
    .output()
    .ok()
    .filter(|output| output.status.success())?;
  String::from_utf8(output.stdout).ok()
}

/// Parses numstat lines (`added<TAB>deleted<TAB>path`); binary files,
/// shown as `-`, count as 0 lines.
fn changes(numstat: &str) -> Vec<rules::Change<'_>> {
  numstat
    .lines()
    .filter_map(|line| {
      let mut fields = line.splitn(3, '\t');
      let mut count = || fields.next()?.parse::<u32>().ok();
      let lines = count().unwrap_or(0) + count().unwrap_or(0);
      Some(rules::Change {
        path: line.rsplit('\t').next()?,
        lines,
      })
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn numstat_counts_both_sides_and_binaries_as_zero() {
    let found = changes("3\t2\tsrc/a.rs\n-\t-\tlogo.png\n");
    let rows: Vec<_> = found.iter().map(|c| (c.path, c.lines)).collect();
    assert_eq!(rows, [("src/a.rs", 5), ("logo.png", 0)]);
  }
}
