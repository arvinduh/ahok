//! Process-level contract of `ahok setup`: it links what `ahok.toml` names,
//! never loses a real file, and fails loudly on what it cannot link.

use std::env;
use std::fs;
use std::path;
use std::process;

/// A fresh repo and target directory under the temp dir.
fn setup(name: &str, links: &str) -> (path::PathBuf, path::PathBuf) {
  let root = env::temp_dir().join(format!("ahok-link-{name}"));
  drop(fs::remove_dir_all(&root));
  let repo = root.join("repo");
  let out = root.join("out");
  fs::create_dir_all(repo.join("skills").join("one")).expect("temp dir");
  fs::create_dir_all(repo.join("skills").join("two")).expect("temp dir");
  fs::create_dir_all(&out).expect("temp dir");
  fs::write(repo.join("a.txt"), "a").expect("temp file");
  fs::write(repo.join("ahok.toml"), links).expect("temp file");
  (repo, out)
}

/// Runs `ahok setup <repo> <args>` with `%OUT%` and the home directory set
/// to `out`; returns success and stdout.
fn link(repo: &path::Path, out: &path::Path, args: &[&str]) -> (bool, String) {
  let output = process::Command::new(env!("CARGO_BIN_EXE_ahok"))
    .arg("setup")
    .arg(repo)
    .args(args)
    .env("OUT", out)
    .env("HOME", out)
    .env("USERPROFILE", out)
    .env_remove("CLAUDE_CODE_REMOTE")
    .output()
    .expect("the ahok binary runs");
  let stdout = String::from_utf8(output.stdout).expect("UTF-8 report");
  (output.status.success(), stdout)
}

/// Whether `link` is a symlink to `source`.
fn points_to(link: &path::Path, source: &path::Path) -> bool {
  fs::read_link(link).is_ok_and(|target| target == source)
}

const ONE: &str = r#"
[[link]]
from = "a.txt"
to = "%OUT%/nested/a.txt"
"#;

#[test]
fn links_then_reports_ok_on_rerun() {
  let (repo, out) = setup("rerun", ONE);
  let (ok, stdout) = link(&repo, &out, &[]);
  assert!(ok, "{stdout}");
  assert!(stdout.starts_with("linked"), "{stdout}");
  assert!(points_to(
    &out.join("nested").join("a.txt"),
    &repo.join("a.txt")
  ));
  let (ok, stdout) = link(&repo, &out, &[]);
  assert!(ok);
  assert!(stdout.starts_with("ok"), "{stdout}");
}

#[test]
fn backs_up_a_real_file() {
  let (repo, out) = setup("backup", ONE);
  fs::create_dir_all(out.join("nested")).unwrap();
  fs::write(out.join("nested").join("a.txt"), "mine").unwrap();
  let (ok, stdout) = link(&repo, &out, &[]);
  assert!(ok, "{stdout}");
  let backup = fs::read_dir(out.join("nested"))
    .unwrap()
    .map(|e| e.unwrap().path())
    .find(|p| p.to_string_lossy().contains(".bak-"))
    .expect("backup file");
  assert_eq!(fs::read_to_string(backup).unwrap(), "mine");
}

#[test]
fn check_changes_nothing_and_fails_when_stale() {
  let (repo, out) = setup("check", ONE);
  let (ok, stdout) = link(&repo, &out, &["--check"]);
  assert!(!ok, "a missing link is out of date");
  assert!(stdout.starts_with("would create"), "{stdout}");
  assert!(!out.join("nested").exists());
  assert!(link(&repo, &out, &[]).0);
  assert!(link(&repo, &out, &["--check"]).0, "up to date after setup");
}

#[test]
fn expands_children_and_filters_by_platform() {
  let links = r#"
[[link]]
from = "skills/*"
to = "%OUT%/skills"

[[link]]
from = "a.txt"
to = "%OUT%/a.txt"
on = ["linux", "windows"]
"#;
  let (repo, out) = setup("cloud", links);
  let (ok, stdout) = link(&repo, &out, &["--platform", "cloud"]);
  assert!(ok, "{stdout}");
  for name in ["one", "two"] {
    let source = repo.join("skills").join(name);
    assert!(points_to(&out.join("skills").join(name), &source));
  }
  assert!(!out.join("a.txt").exists(), "a.txt leaves out cloud");
}

#[test]
fn fails_on_missing_source_and_unset_variable() {
  let links = r#"
[[link]]
from = "gone.txt"
to = "%OUT%/gone.txt"

[[link]]
from = "a.txt"
to = "%AHOK_TEST_UNSET%/a.txt"
"#;
  let (repo, out) = setup("missing", links);
  let (ok, stdout) = link(&repo, &out, &[]);
  assert!(!ok);
  assert!(stdout.contains("MISSING"), "{stdout}");
  assert!(stdout.contains("%AHOK_TEST_UNSET% is not set"), "{stdout}");
}

#[test]
fn rejects_unknown_fields() {
  let links = "[[link]]\nfrom = \"a.txt\"\nt = \"%OUT%/a.txt\"\n";
  let (repo, out) = setup("schema", links);
  let (ok, _) = link(&repo, &out, &[]);
  assert!(!ok);
}

#[test]
fn rejects_unknown_os_keys() {
  let links = "[[link]]\nfrom = \"a.txt\"\nto = { linx = \"%OUT%/a\" }\n";
  let (repo, out) = setup("os-schema", links);
  let (ok, _) = link(&repo, &out, &[]);
  assert!(!ok);
}

#[test]
fn links_a_per_os_target_only_on_that_os() {
  let links = r#"
[[link]]
from = "a.txt"
to = { windows = "%OUT%/a.txt" }
"#;
  let (repo, out) = setup("per-os", links);
  let (ok, stdout) = link(&repo, &out, &["--platform", "linux"]);
  assert!(ok, "{stdout}");
  assert!(!out.join("a.txt").exists(), "no linux target, so no link");
}

#[test]
fn applies_every_repo_given() {
  let (first, out) = setup("multi-a", ONE);
  let other = r#"
[[link]]
from = "a.txt"
to = "%OUT%/b.txt"
"#;
  let (second, _) = setup("multi-b", other);
  let output = process::Command::new(env!("CARGO_BIN_EXE_ahok"))
    .arg("setup")
    .args([&first, &second])
    .env("OUT", &out)
    .env_remove("CLAUDE_CODE_REMOTE")
    .output()
    .expect("the ahok binary runs");
  assert!(output.status.success(), "both repos link");
  assert!(points_to(
    &out.join("nested").join("a.txt"),
    &first.join("a.txt")
  ));
  assert!(points_to(&out.join("b.txt"), &second.join("a.txt")));
}

#[test]
fn places_instructions_and_skills_for_each_harness() {
  let agents = r#"
[agents]
harnesses = ["claude", "codex"]
instructions = "a.txt"
skills = "skills"
"#;
  let (repo, out) = setup("agents", agents);
  let (ok, stdout) = link(&repo, &out, &[]);
  assert!(ok, "{stdout}");
  let instructions = repo.join("a.txt");
  assert!(points_to(&out.join(".claude/CLAUDE.md"), &instructions));
  assert!(points_to(&out.join(".codex/AGENTS.md"), &instructions));
  let skill = repo.join("skills").join("two");
  assert!(points_to(&out.join(".claude/skills/two"), &skill));
  assert!(points_to(&out.join(".codex/skills/two"), &skill));
  assert!(!out.join(".gemini").exists(), "agy is not listed");
}

#[test]
fn writes_hook_configs_and_keeps_other_keys() {
  let agents = r#"
[agents]
harnesses = ["claude", "codex", "agy"]
rules = "rules"
claude_settings = "settings.json"

[[check]]
on = ["stop"]
run = "cargo check"
"#;
  let (repo, out) = setup("hooks", agents);
  fs::write(repo.join("settings.json"), r#"{"tui":"full"}"#).expect("seed");
  fs::create_dir_all(out.join(".claude")).expect("temp dir");
  let settings = out.join(".claude").join("settings.json");
  fs::write(&settings, r#"{"model":"opus"}"#).expect("seed");
  let (ok, stdout) = link(&repo, &out, &[]);
  assert!(ok, "{stdout}");
  let read = |path: &path::Path| -> serde_json::Value {
    let text = fs::read_to_string(path).expect("written");
    serde_json::from_str(&text).expect("JSON")
  };
  let claude = read(&settings);
  assert_eq!(claude["model"], "opus", "keeps the user's keys");
  assert_eq!(claude["tui"], "full", "sets claude_settings keys");
  let hooks = claude["hooks"].as_object().expect("hooks");
  let events: Vec<&str> = hooks.keys().map(String::as_str).collect();
  assert_eq!(events, ["PreToolUse", "Stop"], "no edit checks");
  let codex = read(&out.join(".codex").join("hooks.json"));
  assert!(codex["hooks"]["Stop"].is_array(), "{codex}");
  let (ok, stdout) = link(&repo, &out, &["--check"]);
  assert!(ok, "rerun is clean: {stdout}");
}

#[test]
fn refuses_two_workspaces_with_agents() {
  let (one, out) = setup("agents-one", "[agents]\n");
  let (two, _) = setup("agents-two", "[agents]\n");
  let output = process::Command::new(env!("CARGO_BIN_EXE_ahok"))
    .args(["setup", "--check"])
    .args([&one, &two])
    .env("HOME", &out)
    .env("USERPROFILE", &out)
    .output()
    .expect("the ahok binary runs");
  assert!(!output.status.success(), "two [agents] must fail");
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(stderr.contains("keep it in one"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn git_hooks_run_the_workspace_rules() {
  let rules = path::Path::new(env!("CARGO_MANIFEST_DIR"))
    .join("crates/core/tests/fixtures/rules");
  let manifest = format!(
    "[agents]\nharnesses = [\"git\"]\nrules = \"{}\"\n",
    rules.display()
  );
  let (repo, out) = setup("git", &manifest);
  let (ok, stdout) = link(&repo, &out, &[]);
  assert!(ok, "{stdout}");
  assert!(stdout.contains("core.hooksPath"), "{stdout}");
  let (ok, stdout) = link(&repo, &out, &["--check"]);
  assert!(ok, "rerun is clean: {stdout}");
  let bin = path::Path::new(env!("CARGO_BIN_EXE_ahok"));
  let bin_dir = bin.parent().expect("the binary has a directory");
  let paths = env::var_os("PATH").unwrap_or_default();
  let paths = env::split_paths(&paths);
  let path = env::join_paths(std::iter::once(bin_dir.to_owned()).chain(paths))
    .expect("PATH joins");
  let work = out.join("work");
  fs::create_dir_all(&work).expect("temp dir");
  let git = |args: &[&str]| {
    process::Command::new("git")
      .args(["-c", "user.name=t", "-c", "user.email=t@t"])
      .args(args)
      .current_dir(&work)
      .env("HOME", &out)
      .env("PATH", &path)
      .output()
      .expect("git runs")
  };
  assert!(git(&["init", "-q"]).status.success(), "git init");
  let bad = git(&["commit", "-q", "--allow-empty", "-m", "Fixed stuff."]);
  let stderr = String::from_utf8_lossy(&bad.stderr);
  assert!(!bad.status.success(), "the shim must stop a bad message");
  assert!(
    stderr.contains("[ahok conventional-commit-msg]"),
    "{stderr}"
  );
  let good = git(&[
    "commit",
    "-q",
    "--allow-empty",
    "-m",
    "feat(git): add hooks",
  ]);
  let stderr = String::from_utf8_lossy(&good.stderr);
  assert!(good.status.success(), "{stderr}");
}
