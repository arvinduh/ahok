//! Matching events against rules.
//!
//! This module decides which rules apply to an event and which one wins.
//! The schema and loading live in `rules`; turning the winner into a
//! harness response belongs to the harness adapters.

use std::slice;

use crate::event;
use crate::rules;
use crate::shell;

/// The rule that decided an event.
#[derive(Debug)]
pub struct Match<'a> {
  /// The rule's `id`, for the decision log.
  pub id: &'a str,
  /// What to do with the call.
  pub decision: rules::Decision,
  /// Why, for the agent.
  pub reason: Option<&'a str>,
}

/// Facts outside the event that rules test.
pub struct Env<'a> {
  /// The running OS, as in `std::env::consts::OS`.
  pub os: &'a str,
  /// Whether an environment variable is set.
  pub var_set: &'a dyn Fn(&str) -> bool,
}

/// Returns the strongest rule that matches `event`.
///
/// Deny beats ask; among equals, the first rule in load order wins. Rules
/// without a `decision` never win here.
#[must_use]
pub fn evaluate<'a>(
  rules: &'a [rules::Rule],
  event: &event::Event,
  env: &Env,
) -> Option<Match<'a>> {
  let event::Kind::PreToolUse(tool) = &event.kind else {
    return None;
  };
  let argvs = match tool {
    event::Tool::Shell { command } => shell::commands(command),
    _ => Vec::new(),
  };
  rules
    .iter()
    .filter(|rule| applies(rule, tool, &argvs, env))
    .filter_map(|rule| {
      Some(Match {
        id: &rule.id,
        decision: rule.decision?,
        reason: rule.reason.as_deref(),
      })
    })
    .min_by_key(|found| found.decision)
}

/// Whether every condition `rule` sets holds for this tool call.
fn applies(
  rule: &rules::Rule,
  tool: &event::Tool,
  argvs: &[Vec<String>],
  env: &Env,
) -> bool {
  matches!(rule.event, rules::Event::PreToolUse)
    && rule.os.as_deref().is_none_or(|os| os == env.os)
    && rule
      .skip_if_env
      .as_deref()
      .is_none_or(|var| !(env.var_set)(var))
    && rule
      .tool
      .as_ref()
      .is_none_or(|tools| names(tools).iter().any(|name| tool_is(tool, name)))
    && (rule.match_commands.is_empty()
      || rule
        .match_commands
        .iter()
        .any(|pattern| argvs.iter().any(|argv| has_words(argv, pattern))))
}

/// The tool names a rule lists.
fn names(tools: &rules::Tools) -> &[String] {
  match tools {
    rules::Tools::One(name) => slice::from_ref(name),
    rules::Tools::Many(names) => names,
  }
}

/// Whether `tool` is the one a rule calls `name`.
fn tool_is(tool: &event::Tool, name: &str) -> bool {
  match tool {
    event::Tool::Shell { .. } => name == "shell",
    event::Tool::Edit { .. } => name == "edit",
    event::Tool::Other { name: other } => other == name,
  }
}

/// Whether `argv` runs `pattern`'s program with its other words, in order.
///
/// `"git push --force"` matches `git push origin main --force` but not
/// `git push --force-with-lease`.
fn has_words(argv: &[String], pattern: &str) -> bool {
  let mut words = pattern.split_whitespace();
  let (Some(program), Some((first, rest))) = (words.next(), argv.split_first())
  else {
    return false;
  };
  let mut rest = rest.iter();
  first == program && words.all(|word| rest.any(|arg| arg == word))
}
