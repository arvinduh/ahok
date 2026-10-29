//! Matching events against rules.
//!
//! This module decides which rules apply to an event and which one wins.
//! The schema and loading live in `rules`; turning the winner into a
//! harness response belongs to the harness adapters.

use std::fs;
use std::path;
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
  let cwd = &event.cwd;
  let argvs = match tool {
    event::Tool::Shell { command } => shell::commands(command),
    _ => Vec::new(),
  };
  rules
    .iter()
    .filter(|rule| applies(rule, tool, &argvs, cwd, env))
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
  cwd: &path::Path,
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
    && rule.match_pattern.as_ref().is_none_or(|pattern| {
      matches!(tool, event::Tool::Shell { command } if pattern.is_match(command))
    })
    && rule.match_files.as_ref().is_none_or(|globs| {
      matches!(tool, event::Tool::Edit { paths }
        if paths.iter().any(|path| globs.is_match(path)))
    })
    && (rule.match_commands.is_empty()
      || argvs.iter().any(|argv| {
        rule.match_commands.iter().any(|pattern| has_words(argv, pattern))
          && rule
            .max_file_bytes
            .is_none_or(|max| reads_large_file(argv, cwd, max))
      }))
}

/// Whether any non-flag argument in `argv` names a file over `max` bytes.
///
/// # Side Effects
///
/// Reads file metadata, never contents.
fn reads_large_file(argv: &[String], cwd: &path::Path, max: u64) -> bool {
  argv
    .iter()
    .skip(1)
    .filter(|arg| !arg.starts_with('-'))
    .any(|arg| {
      fs::metadata(cwd.join(arg))
        .is_ok_and(|meta| meta.is_file() && meta.len() > max)
    })
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

#[cfg(test)]
mod tests {
  use super::*;

  fn fixture_rules() -> Vec<rules::Rule> {
    let dir = path::Path::new(env!("CARGO_MANIFEST_DIR"));
    rules::load(&dir.join("tests/fixtures/rules")).unwrap()
  }

  fn shell(command: &str) -> event::Event {
    event::Event {
      session_id: "s1".to_owned(),
      cwd: path::PathBuf::new(),
      kind: event::Kind::PreToolUse(event::Tool::Shell {
        command: command.to_owned(),
      }),
    }
  }

  fn decide(command: &str, cloud: bool) -> Option<(String, rules::Decision)> {
    let rules = fixture_rules();
    let var_set = |var: &str| cloud && var == "CLAUDE_CODE_REMOTE";
    let env = Env {
      os: "linux",
      var_set: &var_set,
    };
    evaluate(&rules, &shell(command), &env)
      .map(|found| (found.id.to_owned(), found.decision))
  }

  #[test]
  fn commit_asks_locally_and_is_skipped_in_cloud() {
    let local = decide("cd x && git commit -m 'feat(a): b'", false);
    assert_eq!(
      local,
      Some(("no-unapproved-commit".to_owned(), rules::Decision::Ask))
    );
    assert_eq!(decide("git commit -m x", true), None);
  }

  #[test]
  fn deny_beats_ask() {
    let found = decide("git commit -m x && rustfmt a.rs", false);
    assert_eq!(found.unwrap().1, rules::Decision::Deny);
  }

  #[test]
  fn words_inside_quotes_are_not_commands() {
    assert_eq!(decide("echo 'git commit'", false), None);
  }

  #[test]
  fn os_restricted_rule_skips_other_os() {
    assert_eq!(decide("python3 <<EOF\nx\nEOF", false), None);
  }

  #[test]
  fn pattern_matches_the_raw_command() {
    let rules = fixture_rules();
    let env = Env {
      os: "windows",
      var_set: &|_| false,
    };
    let found = evaluate(&rules, &shell("python3 <<'EOF'\nx\nEOF"), &env);
    assert_eq!(found.unwrap().id, "no-shell-heredoc-scripts");
  }

  #[test]
  fn bad_regex_fails_to_parse() {
    let text =
      "[[rule]]\nid = \"x\"\nevent = \"PreToolUse\"\nmatch_pattern = '('\n";
    assert!(rules::parse(text).is_err());
  }

  #[test]
  fn file_globs_match_edited_paths() {
    let text = "[[rule]]\nid = \"x\"\nevent = \"PreToolUse\"\n\
      match_files = [\"*.rs\"]\ndecision = \"deny\"\n";
    let rules = rules::parse(text).unwrap();
    let edit = |path: &str| event::Event {
      session_id: "s1".to_owned(),
      cwd: path::PathBuf::new(),
      kind: event::Kind::PreToolUse(event::Tool::Edit {
        paths: vec![path::PathBuf::from(path)],
      }),
    };
    let env = Env {
      os: "linux",
      var_set: &|_| false,
    };
    assert!(evaluate(&rules, &edit("/src/a.rs"), &env).is_some());
    assert!(evaluate(&rules, &edit("/src/a.py"), &env).is_none());
  }

  #[test]
  fn bounded_read_checks_file_size() {
    let dir = path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let rules = fixture_rules();
    let env = Env {
      os: "linux",
      var_set: &|_| false,
    };
    let mut event = shell("cat Cargo.lock");
    event.cwd = dir.to_owned();
    let found = evaluate(&rules, &event, &env);
    assert_eq!(found.unwrap().id, "bounded-file-read");
    let mut event = shell("cat -n .rustfmt.toml");
    event.cwd = dir.to_owned();
    assert!(evaluate(&rules, &event, &env).is_none());
  }

  #[test]
  fn has_words_needs_program_and_ordered_words() {
    let argv = |line: &str| -> Vec<String> {
      line.split(' ').map(str::to_owned).collect()
    };
    assert!(has_words(
      &argv("git push origin --force"),
      "git push --force"
    ));
    assert!(!has_words(
      &argv("git push --force-with-lease"),
      "git push --force"
    ));
    assert!(!has_words(&argv("jj push --force"), "git push --force"));
  }
}
