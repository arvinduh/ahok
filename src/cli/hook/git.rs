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

use ahok_core::manifest;
use ahok_core::rules;

use crate::cli;

/// Runs git hook `name` with git's `args`; `Failed` stops git.
///
/// `commit-msg` checks the message file and the staged diff; `pre-push`
/// checks the refs git writes to stdin. Any other hook passes.
pub fn run(workspace: &path::Path, name: &str, args: &[String]) -> cli::Status {
  let Some(rules) = load(workspace) else {
    return cli::Status::Clean;
  };
  let violations = match (name, args.first()) {
    ("commit-msg", Some(file)) => {
      let message = fs::read_to_string(file).unwrap_or_default();
      let numstat = staged_numstat();
      let changes = numstat.as_deref().map(changes);
      let var_set = |name: &str| env::var_os(name).is_some();
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
      rules::pre_push(&rules, refs.as_deref())
    }
    _ => {
      eprintln!("[ahok] git hook `{name}` has no rules; skipped");
      return cli::Status::Clean;
    }
  };
  for violation in &violations {
    eprintln!("[ahok {}] {}", violation.id, violation.reason);
  }
  if violations.is_empty() {
    cli::Status::Clean
  } else {
    cli::Status::Failed
  }
}

/// Loads the workspace's rules, or warns and returns `None` so the hook
/// passes.
fn load(workspace: &path::Path) -> Option<Vec<rules::Rule>> {
  let manifest = manifest::load(workspace)
    .inspect_err(|err| eprintln!("[ahok] git rules skipped: {err}"))
    .ok()?;
  let dir = manifest.agents?.rules(workspace)?;
  rules::load(&dir)
    .inspect_err(|err| eprintln!("[ahok] git rules skipped: {err}"))
    .ok()
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
