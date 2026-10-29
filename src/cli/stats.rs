//! `ahok stats`: print hook latency and rule hits from the decision log.
//!
//! This module reads `decisions.jsonl` and prints one compact, ANSI-free
//! summary. The arithmetic lives in the library's `stats` module.

use std::fs;
use std::io;

use ahok::dirs;
use ahok::stats;

use crate::cli;

/// Prints the summary; errors if the log can't be read.
///
/// A free function rather than `Args::run`, since the command takes no
/// arguments.
pub fn run() -> Result<cli::Status, cli::Error> {
  let state = dirs::state().ok_or(cli::Error::NoState)?;
  let path = state.join("decisions.jsonl");
  let log = match fs::read_to_string(&path) {
    Ok(log) => log,
    Err(err) if err.kind() == io::ErrorKind::NotFound => {
      println!("no decisions logged yet ({})", path.display());
      return Ok(cli::Status::Clean);
    }
    Err(source) => return Err(cli::Error::Read { path, source }),
  };
  let summary = stats::summarize(&log);
  for event in &summary.events {
    println!(
      "{:<20} {:>7} calls  p50 {:>8}  p99 {:>8}",
      event.name,
      event.calls,
      millis(event.p50_micros),
      millis(event.p99_micros)
    );
  }
  for rule in &summary.rules {
    println!("{:<32} {:>7} hits", rule.id, rule.hits);
  }
  if summary.skipped > 0 {
    println!("skipped {} malformed line(s)", summary.skipped);
  }
  Ok(cli::Status::Clean)
}

/// Formats microseconds as milliseconds with one decimal.
fn millis(micros: u64) -> String {
  format!("{}.{} ms", micros / 1000, micros % 1000 / 100)
}
