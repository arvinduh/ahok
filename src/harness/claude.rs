//! Claude Code hook adapter.
//!
//! Owns Claude Code's hook payload shape and its tool names. Event-name
//! lookup is shared in `harness`; rules are evaluated elsewhere.

use std::path;

use serde;
use serde_json;

use crate::dispatch;
use crate::event;
use crate::harness;
use crate::rules;

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
    rules::Decision::Deny => "deny",
    rules::Decision::Ask => "ask",
  };
  let reason = match (found.reason, found.require_skill) {
    (Some(reason), _) => reason.to_owned(),
    (None, Some(skill)) => {
      format!("Load the `{skill}` skill with the Skill tool, then retry.")
    }
    (None, None) => found.id.to_owned(),
  };
  serde_json::json!({
    "hookSpecificOutput": {
      "hookEventName": "PreToolUse",
      "permissionDecision": decision,
      "permissionDecisionReason": format!("[ahok {}] {reason}", found.id),
    }
  })
  .to_string()
}

/// Renders the stdout reply to a `PostToolUse` or `Stop` call whose
/// built-in checks produced `failures`.
///
/// `decision: block` shows the reason to the agent: after an edit it is
/// feedback, and at `Stop` it keeps the agent working. No failures renders
/// `{}`.
#[must_use]
pub fn feedback(failures: &[dispatch::Failure]) -> String {
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

/// Maps a Claude Code tool call onto the harness-neutral tool.
fn tool(name: String, input: ToolInput) -> event::Tool {
  match (name.as_str(), input.command, input.file_path, input.skill) {
    ("Bash", Some(command), _, _) => event::Tool::Shell { command },
    ("Edit" | "Write" | "MultiEdit", _, Some(path), _) => {
      event::Tool::Edit { paths: vec![path] }
    }
    ("Skill", _, _, Some(name)) => event::Tool::Skill { name },
    _ => event::Tool::Other { name },
  }
}

#[cfg(test)]
mod tests {
  use super::*;

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
      decision: rules::Decision::Deny,
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
    let failures = [dispatch::Failure {
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
