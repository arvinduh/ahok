//! Matching events against rules.
//!
//! This module decides which rules apply to an event and which one wins.
//! The schema and loading live in `rules`; turning the winner into a
//! harness response belongs to the harness adapters.

use std::fs;
use std::path;
use std::slice;

use event;
use state::session;

use crate::shell;

/// The rule that decided an event.
#[derive(Debug)]
pub struct Match<'a> {
  /// The rule's `id`, for the decision log.
  pub id: &'a str,
  /// What to do with the call.
  pub decision: event::Decision,
  /// Why, for the agent.
  pub reason: Option<&'a str>,
  /// The skill the rule wants loaded, for a default reason.
  pub require_skill: Option<&'a str>,
  /// Whether the rule fires only once per session, so the caller records it.
  pub once: bool,
}

/// Facts outside the event that rules test.
pub struct Env<'a> {
  /// The running OS, as in `std::env::consts::OS`.
  pub os: &'a str,
  /// Whether an environment variable is set.
  pub var_set: &'a dyn Fn(&str) -> bool,
  /// What this harness session has done so far.
  pub session: &'a session::State,
}

/// Returns the strongest rule that matches `event`.
///
/// Deny beats ask; among equals, the first rule in load order wins. A
/// `require_skill` rule denies until the session loads that skill; a
/// `once_per_session` rule stops matching after it fired once. Other rules
/// without a `decision` never win.
#[must_use]
pub fn evaluate<'a>(
  rules: &'a [crate::Rule],
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
      if rule.once_per_session && env.session.has_fired(&rule.id) {
        return None;
      }
      let decision = match rule.require_skill.as_deref() {
        Some(skill) if env.session.has_skill(skill) => return None,
        Some(_) => rule.decision.unwrap_or(event::Decision::Deny),
        None => rule.decision?,
      };
      Some(Match {
        id: &rule.id,
        decision,
        reason: rule.reason.as_deref(),
        require_skill: rule.require_skill.as_deref(),
        once: rule.once_per_session,
      })
    })
    .min_by_key(|found| found.decision)
}

/// Whether every condition `rule` sets holds for this tool call.
fn applies(
  rule: &crate::Rule,
  tool: &event::Tool,
  argvs: &[Vec<String>],
  cwd: &path::Path,
  env: &Env,
) -> bool {
  matches!(rule.event, crate::Event::PreToolUse)
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
      matches!(tool, event::Tool::Shell { command }
        if pattern.is_match(command))
    })
    && rule.match_files.as_ref().is_none_or(|globs| {
      matches!(tool, event::Tool::Edit { paths }
        if paths.iter().any(|path| globs.is_match(path)))
    })
    && (rule.match_commands.is_empty()
      || argvs.iter().any(|argv| {
        rule
          .match_commands
          .iter()
          .any(|pattern| has_words(argv, pattern))
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
fn names(tools: &crate::Tools) -> &[String] {
  match tools {
    crate::Tools::One(name) => slice::from_ref(name),
    crate::Tools::Many(names) => names,
  }
}

/// Whether `tool` is the one a rule calls `name`.
fn tool_is(tool: &event::Tool, name: &str) -> bool {
  match tool {
    event::Tool::Shell { .. } => name == "shell",
    event::Tool::Edit { .. } => name == "edit",
    event::Tool::Skill { .. } => name == "skill",
    event::Tool::Other { name: other } => other == name,
  }
}

/// Whether `argv` runs `pattern`'s program with its other words, in order.
///
/// `"git push --force"` matches `git push origin main --force` but not
/// `git push --force-with-lease`. Words after a bare `--` belong to another
/// program or are operands, so `cargo run -- fmt` does not run `cargo fmt`.
fn has_words(argv: &[String], pattern: &str) -> bool {
  let mut words = pattern.split_whitespace();
  let (Some(program), Some((first, rest))) = (words.next(), argv.split_first())
  else {
    return false;
  };
  let mut rest = rest.iter().take_while(|arg| *arg != "--");
  first == program && words.all(|word| rest.any(|arg| arg == word))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn fixture_rules() -> Vec<crate::Rule> {
    let dir = path::Path::new(env!("CARGO_MANIFEST_DIR"));
    crate::load(&dir.join("tests/fixtures/rules")).unwrap()
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

  fn decide(command: &str, cloud: bool) -> Option<(String, event::Decision)> {
    let rules = fixture_rules();
    let var_set = |var: &str| cloud && var == "CLAUDE_CODE_REMOTE";
    let env = Env {
      os: "linux",
      var_set: &var_set,
      session: &session::State::default(),
    };
    evaluate(&rules, &shell(command), &env)
      .map(|found| (found.id.to_owned(), found.decision))
  }

  #[test]
  fn commit_asks_locally_and_is_skipped_in_cloud() {
    let local = decide("cd x && git commit -m 'feat(a): b'", false);
    assert_eq!(
      local,
      Some(("no-unapproved-commit".to_owned(), event::Decision::Ask))
    );
    assert_eq!(decide("git commit -m x", true), None);
  }

  #[test]
  fn deny_beats_ask() {
    let found = decide("git commit -m x && rustfmt a.rs", false);
    assert_eq!(found.unwrap().1, event::Decision::Deny);
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
      session: &session::State::default(),
    };
    let found = evaluate(&rules, &shell("python3 <<'EOF'\nx\nEOF"), &env);
    assert_eq!(found.unwrap().id, "no-shell-heredoc-scripts");
  }

  #[test]
  fn bad_regex_fails_to_parse() {
    let text =
      "[[rule]]\nid = \"x\"\nevent = \"PreToolUse\"\nmatch_pattern = '('\n";
    assert!(crate::parse(text).is_err());
  }

  #[test]
  fn file_globs_match_edited_paths() {
    let text = "[[rule]]\nid = \"x\"\nevent = \"PreToolUse\"\n\
      match_files = [\"*.rs\"]\ndecision = \"deny\"\n";
    let rules = crate::parse(text).unwrap();
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
      session: &session::State::default(),
    };
    assert!(evaluate(&rules, &edit("/src/a.rs"), &env).is_some());
    assert!(evaluate(&rules, &edit("/src/a.py"), &env).is_none());
  }

  #[test]
  fn bounded_read_checks_file_size() {
    // The workspace root, which holds a large Cargo.lock.
    let dir = path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rules = fixture_rules();
    let env = Env {
      os: "linux",
      var_set: &|_| false,
      session: &session::State::default(),
    };
    let mut event = shell("cat Cargo.lock");
    event.cwd = dir.clone();
    let found = evaluate(&rules, &event, &env);
    assert_eq!(found.unwrap().id, "bounded-file-read");
    let mut event = shell("cat -n .rustfmt.toml");
    event.cwd = dir;
    assert!(evaluate(&rules, &event, &env).is_none());
  }

  #[test]
  fn require_skill_denies_once_until_loaded() {
    let rules = fixture_rules();
    let mut event = shell("");
    event.kind = event::Kind::PreToolUse(event::Tool::Edit {
      paths: vec![path::PathBuf::from("src/main.rs")],
    });
    let mut state = session::State::default();
    let decide = |state: &session::State| {
      let env = Env {
        os: "linux",
        var_set: &|_| false,
        session: state,
      };
      evaluate(&rules, &event, &env).map(|found| (found.id, found.once))
    };
    assert_eq!(decide(&state), Some(("skill-dispatch-rust", true)));
    state.add_fired("skill-dispatch-rust");
    assert_eq!(decide(&state), None);
    let mut state = session::State::default();
    state.add_skill("rust-guide");
    assert_eq!(decide(&state), None);
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
    assert!(
      !has_words(&argv("cargo run -q -- fmt --check"), "cargo fmt"),
      "words after `--` are not cargo's"
    );
  }
}
