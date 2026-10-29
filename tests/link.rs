//! Process-level contract of `ahok link`: it links what `links.toml` names,
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
  fs::write(repo.join("links.toml"), links).expect("temp file");
  (repo, out)
}

/// Runs `ahok link <repo> <args>` with `%OUT%` set; returns success and
/// stdout.
fn link(repo: &path::Path, out: &path::Path, args: &[&str]) -> (bool, String) {
  let output = process::Command::new(env!("CARGO_BIN_EXE_ahok"))
    .arg("link")
    .arg(repo)
    .args(args)
    .env("OUT", out)
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
source = "a.txt"
linux = "%OUT%/nested/a.txt"
windows = "%OUT%/nested/a.txt"
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
fn dry_run_changes_nothing() {
  let (repo, out) = setup("dry", ONE);
  let (ok, stdout) = link(&repo, &out, &["--dry-run"]);
  assert!(ok);
  assert!(stdout.starts_with("would create"), "{stdout}");
  assert!(!out.join("nested").exists());
}

#[test]
fn expands_children_and_filters_cloud() {
  let links = r#"
[[link]]
source = "skills/*"
home = "%OUT%/skills"
cloud = true

[[link]]
source = "a.txt"
home = "%OUT%/a.txt"
"#;
  let (repo, out) = setup("cloud", links);
  let (ok, stdout) = link(&repo, &out, &["--platform", "cloud"]);
  assert!(ok, "{stdout}");
  for name in ["one", "two"] {
    let source = repo.join("skills").join(name);
    assert!(points_to(&out.join("skills").join(name), &source));
  }
  assert!(!out.join("a.txt").exists(), "a.txt is not marked cloud");
}

#[test]
fn fails_on_missing_source_and_unset_variable() {
  let links = r#"
[[link]]
source = "gone.txt"
home = "%OUT%/gone.txt"

[[link]]
source = "a.txt"
home = "%AHOK_TEST_UNSET%/a.txt"
"#;
  let (repo, out) = setup("missing", links);
  let (ok, stdout) = link(&repo, &out, &[]);
  assert!(!ok);
  assert!(stdout.contains("MISSING"), "{stdout}");
  assert!(stdout.contains("%AHOK_TEST_UNSET% is not set"), "{stdout}");
}

#[test]
fn rejects_unknown_fields() {
  let links = "[[link]]\nsource = \"a.txt\"\nhom = \"%OUT%/a.txt\"\n";
  let (repo, out) = setup("schema", links);
  let (ok, _) = link(&repo, &out, &[]);
  assert!(!ok);
}
