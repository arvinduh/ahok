//! Git hook adapter: the hooks `ahok setup` installs, and where.
//!
//! This module owns the shim scripts git runs and pointing git at them.
//! What the hooks check belongs to `rules` and `check`; answering a call
//! belongs to the binary's `hook git`.

use std::path;
use std::process;

use crate::link;

/// Where the shims live; `core.hooksPath` points here.
pub const HOOKS: &str = "~/.config/ahok/git-hooks";

/// The user-level git config ahok sets `core.hooksPath` in. Git reads it
/// after `~/.config/git/config`, so it wins over a linked dotfile without
/// writing into it.
const CONFIG: &str = "~/.gitconfig";

/// Each hook ahok answers, and whether git writes to its stdin.
const NAMES: [(&str, bool); 2] = [("commit-msg", false), ("pre-push", true)];

/// The shim scripts that run `ahok hook -C <workspace> git <hook>`.
///
/// Each passes only when ahok's rules and checks pass, then runs the
/// repository's own hook, which `core.hooksPath` hides. A missing ahok, or
/// any exit but 1, never blocks git.
#[must_use]
pub fn shims(workspace: &path::Path) -> Vec<link::Script> {
  let workspace = workspace.display().to_string().replace('\'', r"'\''");
  NAMES
    .into_iter()
    .map(|(name, stdin)| {
      let (read, pipe) = if stdin {
        ("refs=$(cat)\n", r#"printf '%s\n' "$refs" | "#)
      } else {
        ("", "")
      };
      let text = format!(
        "#!/bin/sh\n\
         # Written by `ahok setup`; rerun it instead of editing.\n\
         {read}status=0\n\
         if command -v ahok >/dev/null 2>&1; then\n  \
         {pipe}ahok hook -C '{workspace}' git {name} \"$@\" || status=$?\n\
         fi\n\
         [ \"$status\" -eq 1 ] && exit 1\n\
         hook=\"$(git rev-parse --git-common-dir)/hooks/{name}\"\n\
         if [ -x \"$hook\" ]; then\n  \
         {pipe}\"$hook\" \"$@\" || exit $?\n\
         fi\n"
      );
      link::Script {
        to: format!("{HOOKS}/{name}"),
        text,
      }
    })
    .collect()
}

/// Points git's `core.hooksPath` at the shims, in `~/.gitconfig`.
///
/// # Side Effects
///
/// Runs `git config`; unless `dry_run`, it may write `~/.gitconfig`.
#[must_use]
pub fn hooks_path(dry_run: bool) -> link::Row {
  let row = |outcome, note: &str| link::Row {
    outcome,
    target: format!("{CONFIG} core.hooksPath"),
    note: note.to_owned(),
  };
  let file = match link::expand(CONFIG) {
    Ok(file) => file,
    Err(note) => return row(link::Outcome::Failed, &note),
  };
  let config = |args: &[&str]| {
    process::Command::new("git")
      .arg("config")
      .arg("--file")
      .arg(&file)
      .args(args)
      .output()
  };
  let current = config(&["--get", "core.hooksPath"]).ok();
  let current = current
    .as_ref()
    .map(|out| String::from_utf8_lossy(&out.stdout));
  if current.is_some_and(|value| value.trim() == HOOKS) {
    return row(link::Outcome::Ok, "");
  }
  if dry_run {
    return row(link::Outcome::Would("write"), "");
  }
  match config(&["core.hooksPath", HOOKS]) {
    Ok(out) if out.status.success() => row(link::Outcome::Written, ""),
    Ok(out) => row(
      link::Outcome::Failed,
      String::from_utf8_lossy(&out.stderr).trim(),
    ),
    Err(err) => row(link::Outcome::Failed, &err.to_string()),
  }
}
