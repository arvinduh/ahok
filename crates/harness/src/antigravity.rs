//! Antigravity hook adapter.
//!
//! Owns Antigravity's hook payload shape and its tool names. Event-name
//! lookup is shared in `harness`; rules are evaluated elsewhere.

use std::path;

use dispatch;
use event;
use rules;
use serde;
use serde_json;

/// The fields of an Antigravity hook payload that rules read.
#[derive(serde::Deserialize)]
struct Payload {
  /// Antigravity's conversation/session id.
  #[serde(
    alias = "conversationId",
    alias = "sessionId",
    alias = "session_id",
    default
  )]
  conversation_id: String,
  /// Workspace directories the session operates in.
  #[serde(alias = "workspacePaths", default)]
  workspace_paths: Vec<path::PathBuf>,
  /// Tool call details for tool events.
  #[serde(alias = "toolCall", default)]
  tool_call: Option<ToolCall>,
  /// Turn execution index, used to detect already-blocked stops.
  #[serde(alias = "executionNum", default)]
  execution_num: Option<u32>,
  /// Explicit stop hook active indicator.
  #[serde(alias = "stopHookActive", default)]
  stop_hook_active: bool,
}

/// The tool call details sent in Antigravity tool events.
#[derive(Default, serde::Deserialize)]
struct ToolCall {
  /// Tool name, such as `run_command` or `edit_file`.
  #[serde(default)]
  name: String,
  /// Tool arguments map.
  #[serde(default)]
  args: ToolArgs,
}

/// Tool arguments extracted for shell, edit, and skill inspection.
#[derive(Default, serde::Deserialize)]
struct ToolArgs {
  /// Shell command line for `run_command`.
  #[serde(alias = "CommandLine", alias = "commandLine", alias = "command")]
  command: Option<String>,
  /// Target file path for `edit_file` or `write_file`.
  #[serde(
    alias = "Path",
    alias = "path",
    alias = "file_path",
    alias = "filePath"
  )]
  path: Option<path::PathBuf>,
  /// Skill name when loading skills.
  #[serde(
    alias = "Skill",
    alias = "skill",
    alias = "skill_name",
    alias = "skillName"
  )]
  skill: Option<String>,
}

/// Parses the payload Antigravity sends on stdin for hook `event_name`.
///
/// # Errors
///
/// `crate::Error::Json` for a malformed payload, and
/// `crate::Error::UnknownEvent` for an unsupported `event_name`.
pub fn parse(
  event_name: &str,
  json: &str,
) -> Result<event::Event, crate::Error> {
  let payload: Payload = serde_json::from_str(json)?;
  let already_blocked = payload.stop_hook_active
    || payload.execution_num.is_some_and(|num| num > 1);
  let kind =
    crate::kind(event_name, || tool(payload.tool_call), already_blocked)?;
  let cwd = payload
    .workspace_paths
    .into_iter()
    .next()
    .unwrap_or_default();
  Ok(event::Event {
    session_id: payload.conversation_id,
    cwd,
    kind,
  })
}

/// Renders the stdout reply to an Antigravity hook call that `found` decided.
///
/// No match renders `{}`, letting the tool run.
#[must_use]
pub fn respond(found: Option<&rules::Match>) -> String {
  let Some(found) = found else {
    return "{}".to_owned();
  };
  let decision = match found.decision {
    event::Decision::Deny => "deny",
    event::Decision::Ask => "ask",
  };
  let reason = match (found.reason, found.require_skill) {
    (Some(reason), _) => reason.to_owned(),
    (None, Some(skill)) => {
      format!("Load the `{skill}` skill before continuing.")
    }
    (None, None) => found.id.to_owned(),
  };
  serde_json::json!({
    "decision": decision,
    "reason": format!("[ahok {}] {reason}", found.id),
  })
  .to_string()
}

/// Renders the stdout reply to an Antigravity `Stop` call with `failures`.
///
/// `decision: continue` re-enters the agent loop to fix checks.
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
  serde_json::json!({
    "decision": "continue",
    "reason": reason,
  })
  .to_string()
}

/// Maps an Antigravity tool call onto the harness-neutral tool.
fn tool(call: Option<ToolCall>) -> event::Tool {
  let Some(call) = call else {
    return event::Tool::Other {
      name: String::new(),
    };
  };
  let name = call.name;
  let args = call.args;
  match (name.as_str(), args.command, args.path, args.skill) {
    ("run_command", Some(command), _, _) => event::Tool::Shell { command },
    ("write_file" | "edit_file", _, Some(path), _) => {
      event::Tool::Edit { paths: vec![path] }
    }
    ("Skill" | "skill" | "use_skill", _, _, Some(name)) => {
      event::Tool::Skill { name }
    }
    _ => event::Tool::Other { name },
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn pre_tool(tool: &str, args: &str) -> event::Event {
    let json = format!(
      r#"{{"conversationId":"c1","workspacePaths":["/app"],
      "toolCall":{{"name":"{tool}","args":{args}}}}}"#
    );
    parse("PreToolUse", &json).expect("valid payload")
  }

  #[test]
  fn run_command_becomes_shell() {
    let event = pre_tool("run_command", r#"{"CommandLine":"git status"}"#);
    assert_eq!(event.session_id, "c1");
    assert_eq!(event.cwd, path::PathBuf::from("/app"));
    assert!(matches!(
      event.kind,
      event::Kind::PreToolUse(event::Tool::Shell { command })
        if command == "git status"
    ));
  }

  #[test]
  fn edit_file_becomes_edit() {
    let event = pre_tool("edit_file", r#"{"Path":"src/main.rs"}"#);
    assert!(matches!(
      event.kind,
      event::Kind::PreToolUse(event::Tool::Edit { paths })
        if paths == [path::PathBuf::from("src/main.rs")]
    ));
  }

  #[test]
  fn write_file_becomes_edit() {
    let event = pre_tool("write_file", r#"{"Path":"src/lib.rs"}"#);
    assert!(matches!(
      event.kind,
      event::Kind::PreToolUse(event::Tool::Edit { paths })
        if paths == [path::PathBuf::from("src/lib.rs")]
    ));
  }

  #[test]
  fn skill_tool_becomes_skill() {
    let event = pre_tool("Skill", r#"{"Skill":"rust-guide"}"#);
    let event::Kind::PreToolUse(event::Tool::Skill { name }) = event.kind
    else {
      panic!("expected a skill load, got {:?}", event.kind);
    };
    assert_eq!(name, "rust-guide");
  }

  #[test]
  fn other_tool_keeps_its_name() {
    let event = pre_tool("read_file", r#"{"Path":"src/lib.rs"}"#);
    assert!(matches!(
      event.kind,
      event::Kind::PreToolUse(event::Tool::Other { name })
        if name == "read_file"
    ));
  }

  #[test]
  fn deny_renders_decision_and_reason() {
    let found = rules::Match {
      id: "format-via-fml",
      decision: event::Decision::Deny,
      reason: Some("Use fml."),
      require_skill: None,
      once: false,
    };
    let reply: serde_json::Value =
      serde_json::from_str(&respond(Some(&found))).expect("valid JSON");
    assert_eq!(reply["decision"], "deny");
    assert_eq!(reply["reason"], "[ahok format-via-fml] Use fml.");
    assert_eq!(respond(None), "{}");
  }

  #[test]
  fn stop_feedback_renders_continue() {
    let failures = [dispatch::Failure {
      command: "cargo check".to_owned(),
      output: "error: broken".to_owned(),
    }];
    let reply: serde_json::Value =
      serde_json::from_str(&feedback(&failures)).expect("valid JSON");
    assert_eq!(reply["decision"], "continue");
    assert_eq!(
      reply["reason"],
      "[ahok] 1 check(s) failed.\n\n$ cargo check\nerror: broken"
    );
    assert_eq!(feedback(&[]), "{}");
  }

  #[test]
  fn stop_with_execution_num_detects_already_blocked() {
    let json = r#"{"conversationId":"c1","executionNum":2}"#;
    let event = parse("Stop", json).expect("valid payload");
    assert!(matches!(
      event.kind,
      event::Kind::Stop {
        already_blocked: true
      }
    ));

    let json_first = r#"{"conversationId":"c1","executionNum":1}"#;
    let event_first = parse("Stop", json_first).expect("valid payload");
    assert!(matches!(
      event_first.kind,
      event::Kind::Stop {
        already_blocked: false
      }
    ));
  }
}
