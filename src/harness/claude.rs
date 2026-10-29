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
