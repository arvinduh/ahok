//! A commit's CI checks, read from GitHub through `gh api`.
//!
//! This crate runs `gh` and turns GitHub's check runs and commit statuses
//! into one list of checks. Waiting, polling and printing belong to the
//! binary's `ci wait` command.

#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::io;
use std::process;

use serde;
use serde_json;
use thiserror;

/// Why a GitHub query failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
  /// `gh` or `git` could not be started, e.g. because it is not installed.
  #[error("starting {program}: {source}")]
  Spawn {
    /// The program that failed to start.
    program: &'static str,
    /// Why it failed.
    source: io::Error,
  },
  /// `gh` or `git` ran and failed, e.g. with no login or no such PR.
  #[error("{command}: {stderr}")]
  Command {
    /// The command line that failed.
    command: String,
    /// What it printed on stderr.
    stderr: String,
  },
  /// GitHub answered with JSON of an unexpected shape.
  #[error(transparent)]
  Json(#[from] serde_json::Error),
}

/// One check on a commit: a check run or a commit status.
#[derive(Debug)]
pub struct Check {
  /// Name shown on GitHub, such as `check (ubuntu-latest)`.
  pub name: String,
  /// Where the check stands.
  pub state: State,
  /// Page with the check's details, if GitHub gave one.
  pub url: String,
  /// GitHub Actions job id, whose log can be fetched; `None` for checks
  /// from other apps and for commit statuses.
  pub job: Option<u64>,
}

/// Where a check stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
  /// Queued or running.
  Pending,
  /// Finished successfully.
  Passed,
  /// Finished without a verdict: skipped or neutral.
  Skipped,
  /// Finished unsuccessfully: failed, cancelled or timed out.
  Failed,
}

/// Runs `gh api <path>` in the current directory and returns its stdout.
///
/// `{owner}/{repo}` in `path` resolve to the current repository.
///
/// # Errors
///
/// `Error::Spawn` when `gh` can't start, `Error::Command` when it fails.
fn api(path: &str) -> Result<String, Error> {
  run("gh", &["api", path])
}

/// Runs `program` with `args` in the current directory; returns stdout.
///
/// # Errors
///
/// `Error::Spawn` when it can't start, `Error::Command` when it fails.
fn run(program: &'static str, args: &[&str]) -> Result<String, Error> {
  let output = process::Command::new(program)
    .args(args)
    .stdin(process::Stdio::null())
    .output()
    .map_err(|source| Error::Spawn { program, source })?;
  if !output.status.success() {
    return Err(Error::Command {
      command: format!("{program} {}", args.join(" ")),
      stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    });
  }
  Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The check-runs response: `GET /repos/{o}/{r}/commits/{sha}/check-runs`.
#[derive(serde::Deserialize)]
struct CheckRuns {
  /// Every check run on the commit, newest attempt per name.
  check_runs: Vec<CheckRun>,
}

/// One check run.
#[derive(serde::Deserialize)]
struct CheckRun {
  /// Check run id; equals the job id for GitHub Actions.
  id: u64,
  /// Job name.
  name: String,
  /// `queued`, `in_progress`, `completed`, ...
  status: String,
  /// Set once `status` is `completed`: `success`, `failure`, ...
  conclusion: Option<String>,
  /// Page for the run.
  #[serde(default)]
  html_url: String,
  /// The app that created the run.
  app: App,
}

/// The GitHub App behind a check run.
#[derive(serde::Deserialize)]
struct App {
  /// `github-actions` for workflow jobs.
  slug: String,
}

/// The combined-status response: `GET /repos/{o}/{r}/commits/{sha}/status`.
#[derive(serde::Deserialize)]
struct Combined {
  /// The latest status per context.
  statuses: Vec<Status>,
}

/// One commit status, as set by an external CI.
#[derive(serde::Deserialize)]
struct Status {
  /// Status name, such as `ci/circleci`.
  context: String,
  /// `pending`, `success`, `failure` or `error`.
  state: String,
  /// Page with details.
  #[serde(default)]
  target_url: Option<String>,
}

/// Maps a check run's `status` and `conclusion` onto a `State`.
fn run_state(status: &str, conclusion: Option<&str>) -> State {
  match (status, conclusion) {
    ("completed", Some("success")) => State::Passed,
    ("completed", Some("neutral" | "skipped" | "stale")) => State::Skipped,
    ("completed", _) => State::Failed,
    _ => State::Pending,
  }
}

/// Turns the check-runs and combined-status JSON into one list.
///
/// # Errors
///
/// `Error::Json` when either response has an unexpected shape.
fn parse(runs: &str, combined: &str) -> Result<Vec<Check>, Error> {
  let runs: CheckRuns = serde_json::from_str(runs)?;
  let combined: Combined = serde_json::from_str(combined)?;
  let runs = runs.check_runs.into_iter().map(|run| Check {
    state: run_state(&run.status, run.conclusion.as_deref()),
    job: (run.app.slug == "github-actions").then_some(run.id),
    name: run.name,
    url: run.html_url,
  });
  let statuses = combined.statuses.into_iter().map(|status| Check {
    state: match status.state.as_str() {
      "success" => State::Passed,
      "pending" => State::Pending,
      _ => State::Failed,
    },
    name: status.context,
    url: status.target_url.unwrap_or_default(),
    job: None,
  });
  Ok(runs.chain(statuses).collect())
}

/// Reads every check on commit `sha` of the current repository.
///
/// # Errors
///
/// Any `Error` from `gh` or from parsing its output.
///
/// # Side Effects
///
/// Runs `gh api` twice.
pub fn checks(sha: &str) -> Result<Vec<Check>, Error> {
  let runs = api(&format!(
    "repos/{{owner}}/{{repo}}/commits/{sha}/check-runs?per_page=100"
  ))?;
  let combined =
    api(&format!("repos/{{owner}}/{{repo}}/commits/{sha}/status"))?;
  parse(&runs, &combined)
}

/// The PR fields `head` reads: `GET /repos/{o}/{r}/pulls/{n}`.
#[derive(serde::Deserialize)]
struct Pull {
  /// The PR's head branch.
  head: Ref,
}

/// A branch tip.
#[derive(serde::Deserialize)]
struct Ref {
  /// The commit at the tip.
  sha: String,
}

/// Resolves PR `pr` of the current repository to its head commit, or the
/// checked-out commit when there is no PR.
///
/// # Errors
///
/// Any `Error` from `gh` or `git`, or from parsing their output.
///
/// # Side Effects
///
/// Runs `gh api` for a PR, else `git rev-parse`.
pub fn head(pr: Option<u64>) -> Result<String, Error> {
  let Some(number) = pr else {
    return Ok(run("git", &["rev-parse", "HEAD"])?.trim().to_owned());
  };
  let pull: Pull = serde_json::from_str(&api(&format!(
    "repos/{{owner}}/{{repo}}/pulls/{number}"
  ))?)?;
  Ok(pull.head.sha)
}

/// The last `lines` lines of GitHub Actions job `job`'s log, without the
/// timestamp GitHub puts on every line.
///
/// # Errors
///
/// Any `Error` from `gh`, e.g. once the log has expired.
///
/// # Side Effects
///
/// Runs `gh api`, which downloads the whole log.
pub fn log_tail(job: u64, lines: usize) -> Result<String, Error> {
  let log = api(&format!("repos/{{owner}}/{{repo}}/actions/jobs/{job}/logs"))?;
  Ok(tail(&log, lines))
}

/// The last `lines` lines of `log`, each with its leading timestamp cut.
fn tail(log: &str, lines: usize) -> String {
  let all: Vec<_> = log
    .trim_end()
    .lines()
    .map(|line| match line.split_once(' ') {
      Some((stamp, rest)) if stamp.ends_with('Z') && stamp.contains('T') => {
        rest
      }
      _ => line,
    })
    .collect();
  all[all.len().saturating_sub(lines)..].join("\n")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tail_cuts_timestamps_and_keeps_the_end() {
    let log = "2026-10-30T06:38:25.1Z one\n2026-10-30T06:38:26.2Z two\nthree\n";
    assert_eq!(tail(log, 2), "two\nthree");
  }

  #[test]
  fn runs_and_statuses_merge_into_checks() {
    let runs = r#"{"total_count":4,"check_runs":[
      {"id":1,"name":"test","status":"completed","conclusion":"success",
       "html_url":"https://x/1","app":{"slug":"github-actions"}},
      {"id":2,"name":"lint","status":"completed","conclusion":"timed_out",
       "html_url":"https://x/2","app":{"slug":"github-actions"}},
      {"id":3,"name":"docs","status":"completed","conclusion":"skipped",
       "html_url":"https://x/3","app":{"slug":"github-actions"}},
      {"id":4,"name":"scan","status":"in_progress","conclusion":null,
       "html_url":"https://x/4","app":{"slug":"codeql"}}]}"#;
    let combined = r#"{"state":"failure","statuses":[
      {"context":"ci/ext","state":"error","target_url":null}]}"#;
    let checks = parse(runs, combined).unwrap();
    let states: Vec<_> = checks.iter().map(|check| check.state).collect();
    assert_eq!(
      states,
      [
        State::Passed,
        State::Failed,
        State::Skipped,
        State::Pending,
        State::Failed
      ]
    );
    assert_eq!(checks[1].job, Some(2));
    assert_eq!(checks[3].job, None, "only Actions jobs have logs");
    assert_eq!(checks[4].name, "ci/ext");
  }
}
