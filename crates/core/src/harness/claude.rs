//! Claude Code hook adapter.
//!
//! Owns Claude Code's hook payload and settings shapes, its tool names, and
//! where it reads its files. Event-name lookup is shared in `harness`; rules
//! are evaluated elsewhere.

use std::path;

use serde;
use serde_json;

use crate::check;
use crate::event;
use crate::harness;
use crate::rules;

/// Where Claude Code reads the user's instructions.
pub const INSTRUCTIONS: &str = "~/.claude/CLAUDE.md";

/// The directory Claude Code reads the user's skills from, one per child.
pub const SKILLS: &str = "~/.claude/skills";

/// Claude Code's user settings, which hold its hooks.
pub const SETTINGS: &str = "~/.claude/settings.json";

/// The tool that runs shell commands.
const SHELL: &str = "Bash";

/// The tools that write files.
const EDITS: [&str; 3] = ["Edit", "Write", "MultiEdit"];

/// The tool that loads a skill.
const SKILL: &str = "Skill";

/// Hook timeouts in seconds, per event: rules are fast, checks are not.
const TIMEOUTS: [(&str, u32); 3] =
  [("PreToolUse", 5), ("PostToolUse", 60), ("Stop", 300)];

/// The fields of a Claude Code hook payload that rules read.
#[derive(serde::Deserialize)]
struct Payload {
  /// Claude Code's session id.
  session_id: String,
  /// Directory the session works in.
  #[serde(default)]
  cwd: path::PathBuf,
  /// Tool name, such as `Bash`; empty for non-tool events.
  #[serde(default)]
  tool_name: String,
  /// The tool's arguments.
  #[serde(default)]
  tool_input: ToolInput,
  /// Set when a Stop hook already blocked this stop.
  #[serde(default)]
  stop_hook_active: bool,
}

/// The `tool_input` fields used by `Bash` and the file-editing tools.
#[derive(Default, serde::Deserialize)]
struct ToolInput {
  /// `Bash` command line.
  command: Option<String>,
  /// File an editing tool writes.
  file_path: Option<path::PathBuf>,
  /// Skill the `Skill` tool loads.
  skill: Option<String>,
}

/// Parses the payload Claude Code sends on stdin for hook `event_name`.
///
/// # Errors
///
/// `harness::Error::Json` for a malformed payload, and
/// `harness::Error::UnknownEvent` for an unsupported `event_name`.
pub fn parse(
  event_name: &str,
  json: &str,
) -> Result<event::Event, harness::Error> {
  let payload: Payload = serde_json::from_str(json)?;
  let kind = harness::kind(
    event_name,
    || tool(payload.tool_name, payload.tool_input),
    payload.stop_hook_active,
  )?;
  Ok(event::Event {
    session_id: payload.session_id,
    cwd: payload.cwd,
    kind,
  })
}

/// Renders the stdout reply to a hook call that `found` decided.
///
/// No match renders `{}`, which lets the call proceed.
#[must_use]
pub fn respond(found: Option<&rules::Match>) -> String {
  let Some(found) = found else {
    return "{}".to_owned();
  };
  let decision = match found.decision {
    event::Decision::Deny => "deny",
    event::Decision::Ask => "ask",
  };
  permission(decision, found.id, &reason(found))
}

/// Explains `found` to the agent: its own reason, else what it requires.
pub(crate) fn reason(found: &rules::Match) -> String {
  match (found.reason, found.require_skill) {
    (Some(reason), _) => reason.to_owned(),
    (None, Some(skill)) => {
      format!("Load the `{skill}` skill with the Skill tool, then retry.")
    }
    (None, None) => found.id.to_owned(),
  }
}

/// Renders a `PreToolUse` permission reply in the shape Claude Code and
/// Codex share; `decision` is `deny` or `ask`.
pub(crate) fn permission(decision: &str, rule: &str, reason: &str) -> String {
  serde_json::json!({
    "hookSpecificOutput": {
      "hookEventName": "PreToolUse",
      "permissionDecision": decision,
      "permissionDecisionReason": format!("[ahok {rule}] {reason}"),
    }
  })
  .to_string()
}

/// Renders the stdout reply to a `PostToolUse` or `Stop` call whose
/// checks produced `failures`.
///
/// `decision: block` shows the reason to the agent: after an edit it is
/// feedback, and at `Stop` it keeps the agent working. Codex reads the same
/// shape. No failures renders `{}`.
#[must_use]
pub fn feedback(failures: &[check::Failure]) -> String {
  if failures.is_empty() {
    return "{}".to_owned();
  }
  let mut reason = format!("[ahok] {} check(s) failed.", failures.len());
  for failure in failures {
    reason.push_str("\n\n$ ");
    reason.push_str(&failure.command);
    reason.push('\n');
    reason.push_str(&failure.output);
  }
  serde_json::json!({ "decision": "block", "reason": reason }).to_string()
}

/// Renders the `hooks` setting that wires Claude Code to ahok.
#[must_use]
pub fn hooks(wiring: &harness::Wiring) -> serde_json::Value {
  let edits = EDITS.join("|");
  let pre = format!("{SHELL}|{edits}|{SKILL}");
  render(harness::Harness::Claude, &pre, &edits, wiring)
}

/// Renders a `hooks` table in the shape Claude Code and Codex share.
///
/// `pre` and `post` are the tool matchers for `PreToolUse` and
/// `PostToolUse`; an event the workspace doesn't need is left out.
pub(crate) fn render(
  harness: harness::Harness,
  pre: &str,
  post: &str,
  wiring: &harness::Wiring,
) -> serde_json::Value {
  let wanted = [
    (wiring.rules, Some(pre)),
    (wiring.edit, Some(post)),
    (wiring.stop, None),
  ];
  let mut hooks = serde_json::Map::new();
  for ((event, timeout), (wanted, matcher)) in TIMEOUTS.into_iter().zip(wanted)
  {
    if !wanted {
      continue;
    }
    let mut entry = serde_json::json!({
      "hooks": [{
        "type": "command",
        "command": wiring.command(harness, event),
        "timeout": timeout,
      }],
    });
    if let Some(matcher) = matcher {
      entry["matcher"] = matcher.into();
    }
    hooks.insert(event.to_owned(), serde_json::json!([entry]));
  }
  serde_json::Value::Object(hooks)
}

/// Maps a Claude Code tool call onto the harness-neutral tool.
fn tool(name: String, input: ToolInput) -> event::Tool {
  match (name.as_str(), input.command, input.file_path, input.skill) {
    (SHELL, Some(command), _, _) => event::Tool::Shell { command },
    (edit, _, Some(path), _) if EDITS.contains(&edit) => {
      event::Tool::Edit { paths: vec![path] }
    }
    (SKILL, _, _, Some(name)) => event::Tool::Skill { name },
    _ => event::Tool::Other { name },
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hooks_register_only_the_events_the_workspace_needs() {
    let workspace = path::Path::new("/home/me/agents");
    let all = harness::Wiring {
      workspace,
      rules: true,
      edit: true,
      stop: true,
    };
    let hooks = hooks(&all);
    assert_eq!(
      hooks["PreToolUse"][0]["matcher"],
      "Bash|Edit|Write|MultiEdit|Skill"
    );
    assert_eq!(hooks["PostToolUse"][0]["matcher"], "Edit|Write|MultiEdit");
    assert_eq!(hooks["Stop"][0]["hooks"][0]["timeout"], 300);
    assert_eq!(
      hooks["PreToolUse"][0]["hooks"][0]["command"],
      "command -v ahok >/dev/null 2>&1 && ahok hook -C '/home/me/agents' \
       claude PreToolUse || echo '{}'"
    );
    let stop_only = harness::Wiring {
      rules: false,
      edit: false,
      ..all
    };
    let stop_only = super::hooks(&stop_only);
    assert_eq!(hooks_keys(&stop_only), ["Stop"]);
  }

  fn hooks_keys(hooks: &serde_json::Value) -> Vec<&str> {
    hooks
      .as_object()
      .expect("hooks is an object")
      .keys()
      .map(String::as_str)
      .collect()
  }

  fn pre_tool(tool: &str, input: &str) -> event::Event {
    let json = format!(
      r#"{{"session_id":"s1","hook_event_name":"PreToolUse",
      "tool_name":"{tool}","tool_input":{input}}}"#
    );
    parse("PreToolUse", &json).unwrap()
  }

  #[test]
  fn bash_becomes_shell() {
    let event = pre_tool("Bash", r#"{"command":"git status"}"#);
    assert_eq!(event.session_id, "s1");
    assert!(matches!(
      event.kind,
      event::Kind::PreToolUse(event::Tool::Shell { command })
        if command == "git status"
    ));
  }

  #[test]
  fn write_becomes_edit() {
    let event = pre_tool("Write", r#"{"file_path":"/a.rs","content":""}"#);
    assert!(matches!(
      event.kind,
      event::Kind::PreToolUse(event::Tool::Edit { paths })
        if paths == [path::PathBuf::from("/a.rs")]
    ));
  }

  #[test]
  fn skill_tool_becomes_skill() {
    let event = pre_tool("Skill", r#"{"skill":"rust-guide"}"#);
    let event::Kind::PreToolUse(event::Tool::Skill { name }) = event.kind
    else {
      panic!("expected a skill load, got {:?}", event.kind);
    };
    assert_eq!(name, "rust-guide");
  }

  #[test]
  fn other_tool_keeps_its_name() {
    let event = pre_tool("Read", r#"{"file_path":"/a.rs"}"#);
    assert!(matches!(
      event.kind,
      event::Kind::PreToolUse(event::Tool::Other { name }) if name == "Read"
    ));
  }

  #[test]
  fn deny_renders_permission_decision() {
    let found = rules::Match {
      id: "format-via-fml",
      decision: event::Decision::Deny,
      reason: Some("Use fml."),
      require_skill: None,
      once: false,
    };
    let reply: serde_json::Value =
      serde_json::from_str(&respond(Some(&found))).unwrap();
    let output = &reply["hookSpecificOutput"];
    assert_eq!(output["permissionDecision"], "deny");
    assert_eq!(
      output["permissionDecisionReason"],
      "[ahok format-via-fml] Use fml."
    );
    assert_eq!(respond(None), "{}");
  }

  #[test]
  fn failures_render_as_block_with_output() {
    let failures = [check::Failure {
      command: "fml lint --changed".to_owned(),
      output: "a.rs:1: bad".to_owned(),
    }];
    let reply: serde_json::Value =
      serde_json::from_str(&feedback(&failures)).unwrap();
    assert_eq!(reply["decision"], "block");
    assert_eq!(
      reply["reason"],
      "[ahok] 1 check(s) failed.\n\n$ fml lint --changed\na.rs:1: bad"
    );
    assert_eq!(feedback(&[]), "{}");
  }

  #[test]
  fn stop_has_no_tool() {
    let event = parse("Stop", r#"{"session_id":"s1"}"#).unwrap();
    assert!(matches!(
      event.kind,
      event::Kind::Stop {
        already_blocked: false
      }
    ));
    let json = r#"{"session_id":"s1","stop_hook_active":true}"#;
    let event = parse("Stop", json).unwrap();
    assert!(matches!(
      event.kind,
      event::Kind::Stop {
        already_blocked: true
      }
    ));
  }

  #[test]
  fn unknown_event_is_an_error() {
    let err = parse("Notification", r#"{"session_id":"s1"}"#).unwrap_err();
    assert!(matches!(err, harness::Error::UnknownEvent { .. }));
  }

  #[test]
  fn malformed_payload_is_an_error() {
    let err = parse("Stop", "{}").unwrap_err();
    assert!(matches!(err, harness::Error::Json(_)));
  }
}
