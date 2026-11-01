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
    .args(["hook", "-C"])
    .arg(root.join("crates/core/tests/fixtures"))
    .args([harness, event])
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
fn denied_command_replies() {
  let payload = r#"{"session_id":"s","tool_name":"Bash",
    "tool_input":{"command":"rustfmt src/main.rs"}}"#;
  let (stdout, _) = hook("deny", "PreToolUse", payload);
  assert!(
    stdout.contains(r#""permissionDecision":"deny""#),
    "{stdout}"
  );
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
fn antigravity_denied_command_replies() {
  let payload = r#"{"conversationId":"agy-1","toolCall":{"name":"run_command",
    "args":{"CommandLine":"rustfmt src/main.rs"}}}"#;
  let (stdout, _) = hook_harness("agy", "agy-deny", "PreToolUse", payload);
  assert!(stdout.contains(r#""decision":"deny""#), "{stdout}");
  assert!(stdout.contains("[ahok format-via-fml]"), "{stdout}");
}

#[test]
fn antigravity_allowed_command_replies_allow() {
  let payload = r#"{"conversationId":"agy-3","toolCall":{"name":"run_command",
    "args":{"CommandLine":"git status"}}}"#;
  let (stdout, _) = hook_harness("agy", "agy-allow", "PreToolUse", payload);
  assert_eq!(stdout.trim(), r#"{"decision":"allow"}"#);
}

#[test]
fn antigravity_malformed_payload_fails_open_with_allow() {
  let (stdout, _) =
    hook_harness("agy", "agy-malformed", "PreToolUse", "not json");
  assert_eq!(stdout.trim(), r#"{"decision":"allow"}"#);
}

#[test]
fn codex_ask_rule_denies() {
  let payload = r#"{"session_id":"cx-1","cwd":"/repo","tool_name":"Bash",
    "tool_input":{"command":"git commit -m x"}}"#;
  let (stdout, _) = hook_harness("codex", "codex-ask", "PreToolUse", payload);
  assert!(
    stdout.contains(r#""permissionDecision":"deny""#),
    "{stdout}"
  );
}

#[test]
fn codex_patch_needing_a_skill_adds_context() {
  let payload = r#"{"session_id":"cx-2","cwd":"/repo","tool_name":"apply_patch",
    "tool_input":{"command":"*** Begin Patch\n*** Update File: a.rs\n*** End Patch"}}"#;
  let (stdout, _) = hook_harness("codex", "codex-skill", "PreToolUse", payload);
  assert!(stdout.contains(r#""additionalContext":"#), "{stdout}");
  assert!(!stdout.contains("permissionDecision"), "{stdout}");
}

/// Runs `ahok hook -C <workspace> claude <event>` with `manifest` as the
/// workspace's `ahok.toml`; returns stdout.
fn hook_in(name: &str, manifest: &str, event: &str, payload: &str) -> String {
  let workspace = env::temp_dir().join(format!("ahok-hook-ws-{name}"));
  drop(fs::remove_dir_all(&workspace));
  fs::create_dir_all(&workspace).expect("create the workspace");
  fs::write(workspace.join("ahok.toml"), manifest).expect("write manifest");
  let mut child = process::Command::new(env!("CARGO_BIN_EXE_ahok"))
    .args(["hook", "-C"])
    .arg(&workspace)
    .args(["claude", event])
    .env("XDG_STATE_HOME", &workspace)
    .env("LOCALAPPDATA", &workspace)
    .stdin(process::Stdio::piped())
    .stdout(process::Stdio::piped())
    .spawn()
    .expect("the ahok binary runs");
  io::Write::write_all(
    &mut child.stdin.take().expect("stdin is piped"),
    payload.as_bytes(),
  )
  .expect("the hook reads all of stdin");
  let output = child.wait_with_output().expect("the hook exits");
  assert!(output.status.success(), "hook must always exit 0");
  String::from_utf8(output.stdout).expect("the reply is UTF-8 JSON")
}

#[cfg(unix)]
#[test]
fn failed_edit_check_blocks_with_its_command() {
  let manifest = "[[check]]\non = [\"edit\"]\nrun = \"false {files}\"\n";
  let payload = r#"{"session_id":"s","cwd":"/","tool_name":"Edit",
    "tool_input":{"file_path":"/tmp/a.rs"}}"#;
  let stdout = hook_in("edit", manifest, "PostToolUse", payload);
  assert!(stdout.contains(r#""decision":"block""#), "{stdout}");
  assert!(stdout.contains("$ false /tmp/a.rs"), "{stdout}");
}

#[test]
fn stop_outside_a_repo_runs_nothing() {
  let manifest = "[[check]]\non = [\"stop\"]\nrun = \"false\"\n";
  let cwd = env::temp_dir().join("ahok-hook-no-repo");
  fs::create_dir_all(&cwd).expect("create the dir");
  let payload = format!(
    r#"{{"session_id":"s","cwd":{},"stop_hook_active":false}}"#,
    json_string(&cwd.display().to_string())
  );
  let stdout = hook_in("stop", manifest, "Stop", &payload);
  assert_eq!(stdout.trim(), "{}");
}

/// `text` as a JSON string literal, escaping backslashes and quotes.
fn json_string(text: &str) -> String {
  format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}
