//! Rules from `rules/*.toml`: their schema and loading.
//!
//! This module owns the on-disk rule format, which the rules files in the
//! agents repo share with ahok. Turning events into decisions builds on it;
//! finding the rules directory belongs to the caller.

#![expect(
  dead_code,
  reason = "the evaluator reads rule fields as each rule kind is wired up"
)]

use toml;

/// One `[[rule]]` table, exactly as written in a rules file.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
  id: String,
  event: Event,
  tool: Option<Tools>,
  #[serde(default)]
  match_commands: Vec<String>,
  match_pattern: Option<String>,
  #[serde(default)]
  match_files: Vec<String>,
  decision: Option<Decision>,
  reason: Option<String>,
  os: Option<String>,
  require_skill: Option<String>,
  #[serde(default)]
  once_per_session: bool,
  skip_if_env: Option<String>,
  max_file_bytes: Option<u64>,
  fail: Option<Fail>,
}

/// The lifecycle point a rule applies to.
#[derive(Debug, serde::Deserialize)]
enum Event {
  /// Checked before a tool call, from `ahok hook`.
  PreToolUse,
  /// Checked from a git hook, by `ahok git`.
  GitHook,
}

/// The tools a `PreToolUse` rule applies to: `"shell"`, `"edit"`, or a raw
/// harness tool name, given as one string or a list.
#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum Tools {
  /// A single tool.
  One(String),
  /// Any of several tools.
  Many(Vec<String>),
}

/// What happens to a call a rule matches.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum Decision {
  /// Block the call and tell the agent why.
  Deny,
  /// Ask the user to approve the call.
  Ask,
}

/// How a rule behaves when ahok itself fails while checking it.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum Fail {
  /// Block the call instead of the default fail-open allow.
  Closed,
}

/// The top level of a rules file.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
  rule: Vec<Rule>,
}

/// Parses the text of one rules file.
fn parse(text: &str) -> Result<Vec<Rule>, toml::de::Error> {
  toml::from_str::<File>(text).map(|file| file.rule)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_shell_and_edit_rules() {
    let text = include_str!("../tests/fixtures/rules/style.toml");
    let rules = parse(text).unwrap();
    assert_eq!(rules.len(), 4);
    assert!(matches!(rules[0].tool, Some(Tools::One(_))));
    assert!(matches!(rules[1].tool, Some(Tools::Many(_))));
  }

  #[test]
  fn unknown_field_is_rejected() {
    let text = "[[rule]]\nid = \"x\"\nevent = \"PreToolUse\"\ntypo = 1\n";
    assert!(parse(text).is_err());
  }
}
