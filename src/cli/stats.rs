//! `ahok stats`: print hook latency and rule hits from the decision log.
//!
//! This module reads `decisions.jsonl` and prints one compact, ANSI-free
//! summary. The arithmetic lives in the library's `stats` module.

use std::fs;
use std::io;
use std::process;

use ahok::dirs;
use ahok::stats;

/// Prints the summary; exits 1 if the log can't be read.
///
/// A free function rather than `Args::run`, since the command takes no
/// arguments.
pub fn run() -> process::ExitCode {
  let Some(state) = dirs::state() else {
    eprintln!("[ERROR] no state directory: set XDG_STATE_HOME or HOME");
    return process::ExitCode::FAILURE;
  };
  let path = state.join("decisions.jsonl");
  let log = match fs::read_to_string(&path) {
    Ok(log) => log,
    Err(err) if err.kind() == io::ErrorKind::NotFound => {
      println!("no decisions logged yet ({})", path.display());
      return process::ExitCode::SUCCESS;
    }
    Err(err) => {
      eprintln!("[ERROR] {}: {err}", path.display());
      return process::ExitCode::FAILURE;
    }
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
  for (rule, hits) in &summary.rules {
    println!("{rule:<32} {hits:>7} hits");
  }
  if summary.skipped > 0 {
    println!("skipped {} malformed line(s)", summary.skipped);
  }
  process::ExitCode::SUCCESS
}

/// Formats microseconds as milliseconds with one decimal.
fn millis(micros: u64) -> String {
  format!("{}.{} ms", micros / 1000, micros % 1000 / 100)
}
