//! Claude Code hook adapter.
//!
//! Owns Claude Code's hook payload shape and its tool names. Event-name
//! lookup is shared in `harness`; rules are evaluated elsewhere.

use std::path;

use serde_json;

use crate::event;
use crate::harness;

/// The fields of a Claude Code hook payload that rules read.
#[derive(serde::Deserialize)]
struct Payload {
  session_id: String,
  #[serde(default)]
  tool_name: String,
  #[serde(default)]
  tool_input: ToolInput,
}

/// The `tool_input` fields used by `Bash` and the file-editing tools.
#[derive(Default, serde::Deserialize)]
struct ToolInput {
  command: Option<String>,
  file_path: Option<path::PathBuf>,
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
  let kind =
    harness::kind(event_name, || tool(payload.tool_name, payload.tool_input))?;
  Ok(event::Event {
    session_id: payload.session_id,
    kind,
  })
}

/// Maps a Claude Code tool call onto the harness-neutral tool.
fn tool(name: String, input: ToolInput) -> event::Tool {
  match (name.as_str(), input.command, input.file_path) {
    ("Bash", Some(command), _) => event::Tool::Shell { command },
    ("Edit" | "Write" | "MultiEdit", _, Some(path)) => {
      event::Tool::Edit { paths: vec![path] }
    }
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
  fn other_tool_keeps_its_name() {
    let event = pre_tool("Read", r#"{"file_path":"/a.rs"}"#);
    assert!(matches!(
      event.kind,
      event::Kind::PreToolUse(event::Tool::Other { name }) if name == "Read"
    ));
  }

  #[test]
  fn stop_has_no_tool() {
    let event = parse("Stop", r#"{"session_id":"s1"}"#).unwrap();
    assert!(matches!(event.kind, event::Kind::Stop));
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
