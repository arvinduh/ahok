//! Codex hook adapter.
//!
//! Owns Codex's tool names and the parts of its reply that differ from
//! Claude Code. Codex copies Claude Code's payload and reply shapes, so the
//! shared renderers stay in `claude`; event-name lookup is in `harness`.

use std::path;

use event;
use rules;
use serde;
use serde_json;

use crate::claude;

/// The fields of a Codex hook payload that rules read.
#[derive(serde::Deserialize)]
struct Payload {
  /// Codex's session id.
  session_id: String,
  /// Directory the session works in; patch paths are relative to it.
  #[serde(default)]
  cwd: path::PathBuf,
  /// Tool name: `Bash` for shell commands, `apply_patch` for edits.
  #[serde(default)]
  tool_name: String,
  /// The tool's arguments.
  #[serde(default)]
  tool_input: ToolInput,
  /// Set when a Stop hook already blocked this stop.
  #[serde(default)]
  stop_hook_active: bool,
}

/// The `tool_input` field shared by `Bash` and `apply_patch`.
#[derive(Default, serde::Deserialize)]
struct ToolInput {
  /// The shell command, or the whole patch for `apply_patch`.
  command: Option<String>,
}

/// Parses the payload Codex sends on stdin for hook `event_name`.
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
  let kind = crate::kind(
    event_name,
    || tool(payload.tool_name, payload.tool_input.command),
    payload.stop_hook_active,
  )?;
  Ok(event::Event {
    session_id: payload.session_id,
    cwd: payload.cwd,
    kind,
  })
}

/// Renders the stdout reply to a Codex hook call that `found` decided.
///
/// Codex has no Skill tool, so a `require_skill` rule adds context naming
/// the skill instead of blocking. Codex rejects `permissionDecision: ask` as
/// invalid and runs the call, so an `ask` rule denies and tells the agent to
/// get the user's go-ahead. No match renders `{}`, which lets the call
/// proceed.
#[must_use]
pub fn respond(found: Option<&rules::Match>) -> String {
  match found {
    Some(found) if let Some(skill) = found.require_skill => {
      let reason = found.reason.map_or_else(
        || format!("Follow the `{skill}` skill; read its SKILL.md first."),
        str::to_owned,
      );
      serde_json::json!({
        "hookSpecificOutput": {
          "hookEventName": "PreToolUse",
          "additionalContext": format!("[ahok {}] {reason}", found.id),
        }
      })
      .to_string()
    }
    Some(found) if found.decision == event::Decision::Ask => {
      let reason = format!(
        "{} Codex hooks can't ask the user, so ask them in chat; they can \
         run it themselves.",
        claude::reason(found)
      );
      claude::permission("deny", found.id, &reason)
    }
    _ => claude::respond(found),
  }
}

/// Maps a Codex tool call onto the harness-neutral tool.
fn tool(name: String, command: Option<String>) -> event::Tool {
  match (name.as_str(), command) {
    ("Bash", Some(command)) => event::Tool::Shell { command },
    ("apply_patch", Some(patch)) => event::Tool::Edit {
      paths: patch_paths(&patch),
    },
    _ => event::Tool::Other { name },
  }
}

/// Lists the files an `apply_patch` patch creates or changes.
///
/// Deleted files are left out, and a moved file is listed at its new path:
/// only files that exist afterwards can be formatted or linted.
fn patch_paths(patch: &str) -> Vec<path::PathBuf> {
  let mut paths = Vec::new();
  for line in patch.lines() {
    if let Some(file) = line
      .strip_prefix("*** Add File: ")
      .or_else(|| line.strip_prefix("*** Update File: "))
    {
      paths.push(path::PathBuf::from(file.trim()));
    } else if let Some(file) = line.strip_prefix("*** Move to: ")
      && let Some(last) = paths.last_mut()
    {
      *last = path::PathBuf::from(file.trim());
    }
  }
  paths
}

#[cfg(test)]
mod tests {
  use super::*;

  fn pre_tool(tool: &str, command: &str) -> event::Kind {
    let json = serde_json::json!({
      "session_id": "s1",
      "cwd": "/repo",
      "hook_event_name": "PreToolUse",
      "tool_name": tool,
      "tool_input": { "command": command },
    });
    parse("PreToolUse", &json.to_string()).unwrap().kind
  }

  #[test]
  fn bash_becomes_shell() {
    assert!(matches!(
      pre_tool("Bash", "git status"),
      event::Kind::PreToolUse(event::Tool::Shell { command })
        if command == "git status"
    ));
  }

  #[test]
  fn apply_patch_becomes_edit_of_surviving_files() {
    let patch = "*** Begin Patch\n*** Add File: new.rs\n+fn a() {}\n\
      *** Update File: src/old.rs\n*** Move to: src/moved.rs\n@@\n-a\n+b\n\
      *** Delete File: gone.rs\n*** End Patch";
    let event::Kind::PreToolUse(event::Tool::Edit { paths }) =
      pre_tool("apply_patch", patch)
    else {
      panic!("expected an edit");
    };
    assert_eq!(paths, [path::Path::new("new.rs"), "src/moved.rs".as_ref()]);
  }

  #[test]
  fn other_tool_keeps_its_name() {
    assert!(matches!(
      pre_tool("spawn_agent", ""),
      event::Kind::PreToolUse(event::Tool::Other { name })
        if name == "spawn_agent"
    ));
  }

  #[test]
  fn stop_reads_stop_hook_active() {
    let json = r#"{"session_id":"s1","cwd":"/repo","stop_hook_active":true}"#;
    let event = parse("Stop", json).unwrap();
    assert_eq!(event.cwd, path::Path::new("/repo"));
    assert!(matches!(
      event.kind,
      event::Kind::Stop {
        already_blocked: true
      }
    ));
  }

  fn decided(decision: event::Decision) -> serde_json::Value {
    let found = rules::Match {
      id: "commit-needs-approval",
      decision,
      reason: Some("Ask first."),
      require_skill: None,
      once: false,
    };
    let reply: serde_json::Value =
      serde_json::from_str(&respond(Some(&found))).unwrap();
    reply["hookSpecificOutput"].clone()
  }

  #[test]
  fn require_skill_adds_context_without_blocking() {
    let found = rules::Match {
      id: "skill-dispatch-rust",
      decision: event::Decision::Deny,
      reason: None,
      require_skill: Some("rust-guide"),
      once: true,
    };
    let reply: serde_json::Value =
      serde_json::from_str(&respond(Some(&found))).unwrap();
    let output = &reply["hookSpecificOutput"];
    assert!(output.get("permissionDecision").is_none(), "{output}");
    assert_eq!(
      output["additionalContext"],
      "[ahok skill-dispatch-rust] Follow the `rust-guide` skill; read its \
       SKILL.md first."
    );
  }

  #[test]
  fn deny_renders_like_claude() {
    let output = decided(event::Decision::Deny);
    assert_eq!(output["permissionDecision"], "deny");
    assert_eq!(
      output["permissionDecisionReason"],
      "[ahok commit-needs-approval] Ask first."
    );
    assert_eq!(respond(None), "{}");
  }

  #[test]
  fn ask_becomes_deny() {
    let output = decided(event::Decision::Ask);
    assert_eq!(output["permissionDecision"], "deny");
    let reason = output["permissionDecisionReason"].as_str().unwrap();
    assert!(reason.contains("ask them in chat"), "{reason}");
  }
}
