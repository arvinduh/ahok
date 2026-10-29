//! Process-level contract of `ahok hook`: JSON on stdout and exit 0, even
//! when the call fails open.

use std::env;
use std::fs;
use std::io::Write;
use std::path;
use std::process;

/// Runs `ahok hook claude <event>` on `payload` with fixture rules and a
/// fresh state directory; returns stdout and the state directory.
fn hook(name: &str, event: &str, payload: &str) -> (String, path::PathBuf) {
  let root = path::Path::new(env!("CARGO_MANIFEST_DIR"));
  let state = env::temp_dir().join(format!("ahok-hook-{name}"));
  drop(fs::remove_dir_all(&state));
  let mut child = process::Command::new(env!("CARGO_BIN_EXE_ahok"))
    .args(["hook", "claude", event])
    .env("AHOK_CONFIG_DIR", root.join("tests/fixtures"))
    .env("XDG_STATE_HOME", &state)
    .env("LOCALAPPDATA", &state)
    .env_remove("CLAUDE_CODE_REMOTE")
    .stdin(process::Stdio::piped())
    .stdout(process::Stdio::piped())
    .spawn()
    .unwrap();
  child
    .stdin
    .take()
    .unwrap()
    .write_all(payload.as_bytes())
    .unwrap();
  let output = child.wait_with_output().unwrap();
  assert!(output.status.success(), "hook must always exit 0");
  (
    String::from_utf8(output.stdout).unwrap(),
    state.join("ahok"),
  )
}

#[test]
fn denied_command_replies_and_logs() {
  let payload = r#"{"session_id":"s","tool_name":"Bash",
    "tool_input":{"command":"rustfmt src/main.rs"}}"#;
  let (stdout, state) = hook("deny", "PreToolUse", payload);
  assert!(
    stdout.contains(r#""permissionDecision":"deny""#),
    "{stdout}"
  );
  let log = fs::read_to_string(state.join("decisions.jsonl")).unwrap();
  assert!(log.contains(r#""rule":"format-via-fml""#), "{log}");
}

#[test]
fn malformed_payload_fails_open() {
  let (stdout, state) = hook("malformed", "PreToolUse", "not json");
  assert_eq!(stdout.trim(), "{}");
  let errors = fs::read_to_string(state.join("errors.log")).unwrap();
  assert!(errors.contains("hook claude PreToolUse"), "{errors}");
}

#[test]
fn unknown_event_fails_open() {
  let (stdout, _) = hook("unknown", "Notification", r#"{"session_id":"s"}"#);
  assert_eq!(stdout.trim(), "{}");
}
