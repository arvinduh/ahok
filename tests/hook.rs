//! Process-level contract of `ahok hook`: JSON on stdout and exit 0, even
//! when the call fails open.

use std::env;
use std::fs;
use std::io;
use std::path;
use std::process;

/// Runs `ahok hook claude <event>` on `payload` with fixture rules and a
/// fresh state directory; returns stdout and ahok's state directory.
fn hook(name: &str, event: &str, payload: &str) -> (String, path::PathBuf) {
  hook_harness("claude", name, event, payload)
}

/// Runs `ahok hook <harness> <event>` on `payload` with fixture rules and a
/// fresh state directory; returns stdout and ahok's state directory.
fn hook_harness(
  harness: &str,
  name: &str,
  event: &str,
  payload: &str,
) -> (String, path::PathBuf) {
  let state = env::temp_dir().join(format!("ahok-hook-{name}"));
  drop(fs::remove_dir_all(&state));
  (
    run_harness(harness, &state, event, payload),
    state.join("ahok"),
  )
}

/// Runs `ahok hook claude <event>` on `payload` with state under `state`.
fn run(state: &path::Path, event: &str, payload: &str) -> String {
  run_harness("claude", state, event, payload)
}

/// Runs `ahok hook <harness> <event>` on `payload` with state under `state`.
fn run_harness(
  harness: &str,
  state: &path::Path,
  event: &str,
  payload: &str,
) -> String {
  let root = path::Path::new(env!("CARGO_MANIFEST_DIR"));
  let mut child = process::Command::new(env!("CARGO_BIN_EXE_ahok"))
    .args(["hook", harness, event])
    .env("AHOK_CONFIG_DIR", root.join("crates/rules/tests/fixtures"))
    .env("XDG_STATE_HOME", state)
    .env("LOCALAPPDATA", state)
    .env_remove("CLAUDE_CODE_REMOTE")
    .stdin(process::Stdio::piped())
    .stdout(process::Stdio::piped())
    .spawn()
    .expect("the ahok binary runs");
  // A temporary, so stdin closes before the wait and the hook sees EOF.
  io::Write::write_all(
    &mut child.stdin.take().expect("stdin is piped"),
    payload.as_bytes(),
  )
  .expect("the hook reads all of stdin");
  let output = child.wait_with_output().expect("the hook exits");
  assert!(output.status.success(), "hook must always exit 0");
  String::from_utf8(output.stdout).expect("the reply is UTF-8 JSON")
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

#[test]
fn skill_reminder_fires_once_per_session() {
  let edit = r#"{"session_id":"once","tool_name":"Edit",
    "tool_input":{"file_path":"src/lib.rs"}}"#;
  let (first, state) = hook("once", "PreToolUse", edit);
  assert!(first.contains("rust-guide"), "{first}");
  let second = run(state.parent().unwrap(), "PreToolUse", edit);
  assert_eq!(second.trim(), "{}");
}

#[test]
fn antigravity_denied_command_replies_and_logs() {
  let payload = r#"{"conversationId":"agy-1","toolCall":{"name":"run_command",
    "args":{"CommandLine":"rustfmt src/main.rs"}}}"#;
  let (stdout, state) =
    hook_harness("antigravity", "agy-deny", "PreToolUse", payload);
  assert!(stdout.contains(r#""decision":"deny""#), "{stdout}");
  assert!(stdout.contains("[ahok format-via-fml]"), "{stdout}");
  let log = fs::read_to_string(state.join("decisions.jsonl")).unwrap();
  assert!(log.contains(r#""rule":"format-via-fml""#), "{log}");
  assert!(log.contains(r#""harness":"antigravity""#), "{log}");
}

#[test]
fn agy_alias_works_identically() {
  let payload = r#"{"conversationId":"agy-2","toolCall":{"name":"run_command",
    "args":{"CommandLine":"rustfmt src/main.rs"}}}"#;
  let (stdout, state) = hook_harness("agy", "agy-alias", "PreToolUse", payload);
  assert!(stdout.contains(r#""decision":"deny""#), "{stdout}");
  let log = fs::read_to_string(state.join("decisions.jsonl")).unwrap();
  assert!(log.contains(r#""harness":"agy""#), "{log}");
}

#[test]
fn antigravity_allowed_command_replies_allow() {
  let payload = r#"{"conversationId":"agy-3","toolCall":{"name":"run_command",
    "args":{"CommandLine":"git status"}}}"#;
  let (stdout, _) =
    hook_harness("antigravity", "agy-allow", "PreToolUse", payload);
  assert_eq!(stdout.trim(), r#"{"decision":"allow"}"#);
}

#[test]
fn antigravity_malformed_payload_fails_open_with_allow() {
  let (stdout, _) =
    hook_harness("antigravity", "agy-malformed", "PreToolUse", "not json");
  assert_eq!(stdout.trim(), r#"{"decision":"allow"}"#);
}
