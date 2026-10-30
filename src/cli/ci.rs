//! `ahok ci wait [pr]`: block until a commit's CI finishes, then print one
//! summary.
//!
//! This module owns waiting: the backoff schedule, the time limit, and the
//! summary the agent reads. Reading checks from GitHub belongs to `ci`.

use std::env;
use std::thread;
use std::time;

use ci;
use clap;

use crate::cli;

/// First delay between polls.
const MIN_DELAY: time::Duration = time::Duration::from_secs(10);
/// Longest delay between polls.
const MAX_DELAY: time::Duration = time::Duration::from_mins(1);
/// How long a commit may show no checks before `wait` gives up on it.
const NO_CHECKS: time::Duration = time::Duration::from_mins(2);
/// Default limit outside Claude Code, under common command timeouts.
const DEFAULT_LIMIT: u64 = 540;
/// Log lines shown per failed job.
const LOG_LINES: usize = 30;

/// Subcommands of `ahok ci`.
#[derive(clap::Subcommand)]
pub enum Args {
  /// Wait silently until every check is final, then print one summary.
  Wait(Wait),
}

/// Arguments of `ahok ci wait`.
#[derive(clap::Args)]
pub struct Wait {
  /// PR number; defaults to the checked-out commit.
  pr: Option<u64>,
  /// Seconds to wait before exiting 75 to be resumed. Defaults to no limit
  /// in Claude Code, which backgrounds long commands, and 540 elsewhere.
  #[arg(long)]
  timeout: Option<u64>,
}

impl Args {
  /// Runs the chosen `ci` subcommand.
  pub fn run(self) -> Result<cli::Status, ci::Error> {
    match self {
      Self::Wait(wait) => wait.run(),
    }
  }
}

impl Wait {
  /// Polls with backoff until no check is pending, then prints a summary.
  ///
  /// `Clean` when every check passed or was skipped, `Failed` when one
  /// failed or none appeared, `Retry` when the time limit ran out first.
  fn run(self) -> Result<cli::Status, ci::Error> {
    let limit = self
      .timeout
      .or_else(|| env::var_os("CLAUDECODE").is_none().then_some(DEFAULT_LIMIT))
      .map(time::Duration::from_secs);
    let start = time::Instant::now();
    let sha = ci::head(self.pr)?;
    let short = &sha[..sha.len().min(7)];
    let mut delay = MIN_DELAY;
    loop {
      let checks = ci::checks(&sha)?;
      let pending =
        checks.iter().any(|check| check.state == ci::State::Pending);
      if !checks.is_empty() && !pending {
        return Ok(summarize(short, &checks));
      }
      let elapsed = start.elapsed();
      if checks.is_empty() && elapsed >= NO_CHECKS {
        println!("No CI checks on {short} after {}s.", elapsed.as_secs());
        return Ok(cli::Status::Failed);
      }
      let remaining = limit.map(|limit| limit.saturating_sub(elapsed));
      if remaining == Some(time::Duration::ZERO) {
        let pr = self.pr.map(|pr| format!(" {pr}")).unwrap_or_default();
        println!(
          "CI still running on {short} after {}s. Run `ahok ci wait{pr}` \
           again to keep waiting.",
          elapsed.as_secs()
        );
        return Ok(cli::Status::Retry);
      }
      thread::sleep(remaining.map_or(delay, |left| delay.min(left)));
      delay = (delay * 3 / 2).min(MAX_DELAY);
    }
  }
}

/// Prints one summary of settled `checks`, with each failure's log tail.
fn summarize(short: &str, checks: &[ci::Check]) -> cli::Status {
  let count = |state| checks.iter().filter(|c| c.state == state).count();
  let (passed, skipped) = (count(ci::State::Passed), count(ci::State::Skipped));
  let failed: Vec<_> = checks
    .iter()
    .filter(|check| check.state == ci::State::Failed)
    .collect();
  if failed.is_empty() {
    println!("CI passed on {short}: {passed} passed, {skipped} skipped.");
    return cli::Status::Clean;
  }
  println!(
    "CI failed on {short}: {} of {} checks failed ({passed} passed, \
     {skipped} skipped).",
    failed.len(),
    checks.len()
  );
  for check in failed {
    println!("\n--- {} {}", check.name, check.url);
    match check.job.map(|job| ci::log_tail(job, LOG_LINES)) {
      Some(Ok(log)) => println!("{log}"),
      Some(Err(err)) => println!("(log unavailable: {err})"),
      None => {}
    }
  }
  cli::Status::Failed
}
