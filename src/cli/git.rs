//! `ahok git <hook>`: run the `GitHook` rules from a git hook.
//!
//! This module owns the git hook contract: read what git passes (a message
//! file, the staged diff, pushed refs on stdin), print each broken rule to
//! stderr, and exit 1 to stop git. When ahok itself can't load the rules it
//! warns and exits 0, so a broken setup never blocks work. Deciding belongs
//! to `rules`.

use std::env;
use std::fs;
use std::io;
use std::path;
use std::process;

use clap;
use rules;
use state::dirs;

use crate::cli;

/// The git hooks ahok runs in; git passes the arguments.
#[derive(clap::Subcommand)]
pub enum Args {
  /// Check the commit message and the staged diff.
  CommitMsg {
    /// File holding the proposed commit message.
    file: path::PathBuf,
  },
  /// Check the branches a push updates; git writes them to stdin.
  PrePush {
    /// Name of the remote, unused.
    remote: Option<String>,
    /// URL of the remote, unused.
    url: Option<String>,
  },
}

impl Args {
  /// Runs the hook: `Failed` when a rule is broken, which stops git.
  pub fn run(self) -> cli::Status {
    let Some(rules) = load() else {
      return cli::Status::Clean;
    };
    let violations = match self {
      Self::CommitMsg { file } => {
        let message = fs::read_to_string(&file).unwrap_or_default();
        let numstat = staged_numstat();
        let changes = numstat.as_deref().map(changes);
        let var_set = |name: &str| env::var_os(name).is_some();
        rules::commit_msg(&rules, &message, changes.as_deref(), &var_set)
      }
      Self::PrePush { .. } => {
        let input = io::read_to_string(io::stdin()).ok();
        let refs: Option<Vec<_>> = input.as_deref().map(|input| {
          input
            .lines()
            .filter_map(|line| line.split_whitespace().nth(2))
            .collect()
        });
        rules::pre_push(&rules, refs.as_deref())
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
}

/// Loads the rules, or warns and returns `None` so the hook passes.
fn load() -> Option<Vec<rules::Rule>> {
  let Some(config) = dirs::config() else {
    eprintln!("[ahok] no config dir; git rules skipped");
    return None;
  };
  rules::load(&config.join("rules"))
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
