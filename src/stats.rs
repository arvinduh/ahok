//! Summaries of the hook decision log.
//!
//! This module turns the lines of `decisions.jsonl` into per-event latency
//! percentiles and per-rule hit counts. The log format is written by
//! `journal`; printing the summary belongs to `ahok stats`.

use std::collections;

use serde;
use serde_json;

/// Call count and latency percentiles for one hook event.
#[derive(Debug)]
pub struct Event {
  /// Event name, such as `PreToolUse`.
  pub name: String,
  /// Number of logged calls.
  pub calls: usize,
  /// Median time in ahok, in microseconds.
  pub p50_micros: u64,
  /// 99th-percentile time in ahok, in microseconds.
  pub p99_micros: u64,
}

/// Everything `ahok stats` reports.
#[derive(Debug)]
pub struct Summary {
  /// One row per event name, sorted by name.
  pub events: Vec<Event>,
  /// One row per rule that decided a call, most hits first.
  pub rules: Vec<RuleHits>,
  /// Lines that weren't valid log entries.
  pub skipped: usize,
}

/// How often one rule decided a call.
#[derive(Debug)]
pub struct RuleHits {
  /// The rule's id.
  pub id: String,
  /// Number of calls it decided.
  pub hits: usize,
}

/// The fields of a log line the summary reads; others are ignored, so old
/// logs keep working when fields are added.
#[derive(serde::Deserialize)]
struct Entry {
  /// Hook event name.
  event: String,
  /// Id of the deciding rule, if any.
  rule: Option<String>,
  /// Time spent in ahok, in microseconds.
  micros: u64,
}

/// Summarizes the text of a `decisions.jsonl` file.
#[must_use]
pub fn summarize(log: &str) -> Summary {
  let mut latencies = collections::BTreeMap::<String, Vec<u64>>::new();
  let mut hits = collections::HashMap::<String, usize>::new();
  let mut skipped = 0;
  for line in log.lines().filter(|line| !line.trim().is_empty()) {
    let Ok(entry) = serde_json::from_str::<Entry>(line) else {
      skipped += 1;
      continue;
    };
    latencies.entry(entry.event).or_default().push(entry.micros);
    if let Some(rule) = entry.rule {
      *hits.entry(rule).or_default() += 1;
    }
  }
  let events = latencies
    .into_iter()
    .map(|(name, mut micros)| {
      micros.sort_unstable();
      Event {
        name,
        calls: micros.len(),
        p50_micros: percentile(&micros, 50),
        p99_micros: percentile(&micros, 99),
      }
    })
    .collect();
  let mut rules: Vec<_> = hits
    .into_iter()
    .map(|(id, hits)| RuleHits { id, hits })
    .collect();
  rules.sort_by(|a, b| b.hits.cmp(&a.hits).then_with(|| a.id.cmp(&b.id)));
  Summary {
    events,
    rules,
    skipped,
  }
}

/// The nearest-rank `pct`th percentile of non-empty, sorted `values`.
fn percentile(sorted: &[u64], pct: usize) -> u64 {
  let rank = (sorted.len() * pct).div_ceil(100).max(1);
  sorted[rank - 1]
}

#[cfg(test)]
mod tests {
  use std::fmt::Write;

  use super::*;

  #[test]
  fn summarizes_latency_and_hits() {
    let mut log = String::new();
    for micros in 1..=100 {
      writeln!(
        log,
        r#"{{"event":"PreToolUse","rule":null,"micros":{micros}}}"#
      )
      .unwrap();
    }
    log.push_str(r#"{"event":"PreToolUse","rule":"a","micros":5}"#);
    log.push_str("\nnot json\n");
    log.push_str(r#"{"event":"Stop","rule":"b","micros":7,"extra":1}"#);
    let summary = summarize(&log);
    assert_eq!(summary.skipped, 1);
    let pre = &summary.events[0];
    assert_eq!((pre.name.as_str(), pre.calls), ("PreToolUse", 101));
    assert_eq!((pre.p50_micros, pre.p99_micros), (50, 99));
    assert_eq!(summary.events[1].p99_micros, 7);
    let rules: Vec<_> = summary
      .rules
      .iter()
      .map(|rule| (rule.id.as_str(), rule.hits))
      .collect();
    assert_eq!(rules, [("a", 1), ("b", 1)]);
  }
}
