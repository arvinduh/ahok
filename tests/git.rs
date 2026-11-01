//! Process-level contract of `ahok hook git`: exit 1 with a reason on
//! stderr when a rule is broken, exit 0 otherwise.

use std::env;
use std::fs;
use std::io;
use std::path;
use std::process;

/// Runs `ahok hook git <args>` with fixture rules and `stdin` in an empty git
/// repo named after `name`, so this checkout's staged changes can't trip the diff budget;
/// returns whether it passed and its stderr.
fn git_hook(name: &str, args: &[&str], stdin: &str) -> (bool, String) {
  let root = path::Path::new(env!("CARGO_MANIFEST_DIR"));
  let repo = env::temp_dir().join(format!("ahok-git-repo-{name}"));
  drop(fs::remove_dir_all(&repo));
  fs::create_dir_all(&repo).expect("create the temp repo");
  let init = process::Command::new("git")
    .args(["init", "-q"])
    .current_dir(&repo)
    .status()
    .expect("git runs");
  assert!(init.success(), "git init must succeed");
  let mut child = process::Command::new(env!("CARGO_BIN_EXE_ahok"))
    .args(["hook", "-C"])
    .arg(root.join("crates/core/tests/fixtures"))
    .arg("git")
    .args(args)
    .current_dir(&repo)
    .stdin(process::Stdio::piped())
    .stderr(process::Stdio::piped())
    .spawn()
    .expect("the ahok binary runs");
  io::Write::write_all(
    &mut child.stdin.take().expect("stdin is piped"),
    stdin.as_bytes(),
  )
  .expect("the hook reads stdin");
  let output = child.wait_with_output().expect("the hook exits");
  let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
  (output.status.success(), stderr)
}

/// Writes `message` to a temp file named after `name`; returns its path.
fn message_file(name: &str, message: &str) -> String {
  let path = env::temp_dir().join(format!("ahok-git-{name}"));
  fs::write(&path, message).expect("write the message file");
  path.display().to_string()
}

#[test]
fn bad_commit_message_is_rejected() {
  let file = message_file("bad", "Fixed stuff.\n");
  let (passed, stderr) = git_hook("bad", &["commit-msg", &file], "");
  assert!(!passed, "a bad subject must stop the commit");
  assert!(
    stderr.contains("[ahok conventional-commit-msg]"),
    "{stderr}"
  );
}

#[test]
fn good_commit_message_passes() {
  let file = message_file("good", "feat(git): add hooks\n");
  let (passed, stderr) = git_hook("good", &["commit-msg", &file], "");
  assert!(passed, "{stderr}");
}

#[test]
fn push_to_main_is_rejected() {
  let refs = "refs/heads/x abc refs/heads/main def\n";
  let (passed, stderr) = git_hook("push", &["pre-push", "origin", "url"], refs);
  assert!(!passed, "pushing to main must stop the push");
  assert!(stderr.contains("[ahok no-push-default-branch]"), "{stderr}");
  let refs = "refs/heads/x abc refs/heads/x def\n";
  assert!(git_hook("push", &["pre-push", "origin", "url"], refs).0);
}
