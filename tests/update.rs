//! Integration tests for the `ahok update` command routing.

use std::process;

#[test]
fn update_help_reports_check_flag() {
  let output = process::Command::new(env!("CARGO_BIN_EXE_ahok"))
    .args(["update", "--help"])
    .output()
    .expect("the ahok binary runs");
  assert!(output.status.success(), "ahok update --help must succeed");
  let stdout = String::from_utf8(output.stdout).expect("stdout is valid UTF-8");
  assert!(
    stdout.contains("--check"),
    "help output must mention --check flag: {stdout}"
  );
  assert!(
    stdout.contains("Update the ahok binary"),
    "help output must mention update description: {stdout}"
  );
}

#[test]
fn update_check_runs_cleanly() {
  let output = process::Command::new(env!("CARGO_BIN_EXE_ahok"))
    .args(["update", "--check"])
    .output()
    .expect("the ahok binary runs");
  assert!(output.status.success(), "ahok update --check must succeed");
  let stdout = String::from_utf8(output.stdout).expect("stdout is valid UTF-8");
  assert!(
    stdout.contains("ahok: "),
    "update check output must report status: {stdout}"
  );
}
