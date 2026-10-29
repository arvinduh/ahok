//! Rules from `rules/*.toml`: their schema and loading.
//!
//! This module owns the on-disk rule format, which the rules files in the
//! agents repo share with ahok. Turning events into decisions builds on it;
//! finding the rules directory belongs to the caller.

use std::fs;
use std::io;
use std::path;

use globset;
use regex;
use serde;
use thiserror;
use toml;

mod eval;

pub use eval::Env;
pub use eval::Match;
pub use eval::evaluate;

/// Why the rules directory could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum Error {
  /// The directory or a file in it could not be read.
  #[error(transparent)]
  Io(#[from] io::Error),
  /// A rules file is not valid TOML or does not match the schema.
  #[error("{}: {source}", path.display())]
  Parse {
    /// The offending file.
    path: path::PathBuf,
    /// What the parser rejected.
    source: toml::de::Error,
  },
}

/// One `[[rule]]` table, exactly as written in a rules file.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
  dead_code,
  reason = "git-hook fields are parsed so bad rules fail to load; `ahok git` \
            will read them"
)]
pub struct Rule {
  /// Unique name, shown to the agent and logged.
  id: String,
  /// Where the rule is checked.
  event: Event,
  /// Tools the rule applies to; any tool when unset.
  tool: Option<Tools>,
  /// Programs plus words, matched against each simple command's argv.
  #[serde(default)]
  match_commands: Vec<String>,
  /// Regex matched against the whole shell command line.
  #[serde(default, deserialize_with = "compile_regex")]
  match_pattern: Option<regex::Regex>,
  /// Globs matched against the paths an edit writes.
  #[serde(default, deserialize_with = "compile_globs")]
  match_files: Option<globset::GlobSet>,
  /// What to do with a matching call.
  decision: Option<Decision>,
  /// Why, shown to the agent.
  reason: Option<String>,
  /// Only on this OS, as in `std::env::consts::OS`.
  os: Option<String>,
  /// Deny until the session loads this skill.
  require_skill: Option<String>,
  /// Stop matching after firing once in a session.
  #[serde(default)]
  once_per_session: bool,
  /// The git hook a `GitHook` rule runs in.
  hook: Option<GitHook>,
  /// Regex the commit subject must match.
  pattern: Option<String>,
  /// Commit-message tag that exempts the commit.
  allow_tag: Option<String>,
  /// Most net lines a staged diff may change.
  max_net_lines: Option<u32>,
  /// Paths left out of the diff count.
  #[serde(default)]
  exclude_globs: Vec<String>,
  /// Environment variable that exempts the commit.
  allow_env: Option<String>,
  /// Skip the rule while this variable is set.
  skip_if_env: Option<String>,
  /// Match only when an argument names a larger file.
  max_file_bytes: Option<u64>,
  /// Branches a push may not target.
  #[serde(default)]
  protected_branches: Vec<String>,
  /// Behavior when ahok fails while checking the rule.
  fail: Option<Fail>,
}

/// The lifecycle point a rule applies to.
#[derive(Debug, serde::Deserialize)]
enum Event {
  /// Checked before a tool call, from `ahok hook`.
  PreToolUse,
  /// Checked from a git hook, by `ahok git`.
  GitHook,
}

/// The tools a `PreToolUse` rule applies to: `"shell"`, `"edit"`, or a raw
/// harness tool name, given as one string or a list.
#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum Tools {
  /// A single tool.
  One(String),
  /// Any of several tools.
  Many(Vec<String>),
}

/// What happens to a call a rule matches; earlier variants win.
#[derive(
  Clone,
  Copy,
  Debug,
  PartialEq,
  Eq,
  PartialOrd,
  Ord,
  serde::Deserialize,
  serde::Serialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
  /// Block the call and tell the agent why.
  Deny,
  /// Ask the user to approve the call.
  Ask,
}

/// The git hook a `GitHook` rule runs in.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum GitHook {
  /// Checks the commit message and the staged diff.
  CommitMsg,
  /// Checks the refs being pushed.
  PrePush,
}

/// How a rule behaves when ahok itself fails while checking it.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum Fail {
  /// Block the call instead of the default fail-open allow.
  Closed,
}

/// Compiles a regex field, so a bad pattern fails loading with its file.
fn compile_regex<'de, D: serde::Deserializer<'de>>(
  deserializer: D,
) -> Result<Option<regex::Regex>, D::Error> {
  let pattern = <String as serde::Deserialize>::deserialize(deserializer)?;
  regex::Regex::new(&pattern)
    .map(Some)
    .map_err(<D::Error as serde::de::Error>::custom)
}

/// Compiles a glob list into one matcher, failing loading on a bad glob.
fn compile_globs<'de, D: serde::Deserializer<'de>>(
  deserializer: D,
) -> Result<Option<globset::GlobSet>, D::Error> {
  let globs = <Vec<String> as serde::Deserialize>::deserialize(deserializer)?;
  let mut set = globset::GlobSetBuilder::new();
  for glob in &globs {
    set.add(globset::Glob::new(glob).map_err(serde::de::Error::custom)?);
  }
  set.build().map(Some).map_err(serde::de::Error::custom)
}

/// The top level of a rules file.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
  /// Every `[[rule]]` table, in file order.
  rule: Vec<Rule>,
}

/// Parses the text of one rules file.
fn parse(text: &str) -> Result<Vec<Rule>, toml::de::Error> {
  toml::from_str::<File>(text).map(|file| file.rule)
}

/// Loads every `*.toml` file in `dir`, in file-name order.
///
/// # Errors
///
/// `Error::Io` if `dir` or a file in it can't be read, and `Error::Parse`
/// naming the first file that fails to parse.
pub fn load(dir: &path::Path) -> Result<Vec<Rule>, Error> {
  let mut paths = fs::read_dir(dir)?
    .map(|entry| entry.map(|entry| entry.path()))
    .collect::<io::Result<Vec<_>>>()?;
  paths.retain(|path| path.extension().is_some_and(|ext| ext == "toml"));
  paths.sort();
  let mut rules = Vec::new();
  for path in paths {
    let text = fs::read_to_string(&path)?;
    match parse(&text) {
      Ok(file_rules) => rules.extend(file_rules),
      Err(source) => return Err(Error::Parse { path, source }),
    }
  }
  Ok(rules)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn fixtures() -> path::PathBuf {
    path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rules")
  }

  #[test]
  fn parses_shell_and_edit_rules() {
    let text = include_str!("../tests/fixtures/rules/style.toml");
    let rules = parse(text).unwrap();
    assert_eq!(rules.len(), 4);
    assert!(matches!(rules[0].tool, Some(Tools::One(_))));
    assert!(matches!(rules[1].tool, Some(Tools::Many(_))));
  }

  #[test]
  fn parses_git_hook_rules() {
    let text = include_str!("../tests/fixtures/rules/git.toml");
    let rules = parse(text).unwrap();
    assert_eq!(rules.len(), 4);
    assert!(matches!(rules[1].hook, Some(GitHook::CommitMsg)));
    assert!(matches!(rules[3].hook, Some(GitHook::PrePush)));
  }

  #[test]
  fn double_quoted_regex_escape_is_rejected() {
    let text = "[[rule]]\nid = \"x\"\nevent = \"GitHook\"\npattern = \"\\(\"\n";
    assert!(parse(text).is_err());
  }

  #[test]
  fn unknown_field_is_rejected() {
    let text = "[[rule]]\nid = \"x\"\nevent = \"PreToolUse\"\ntypo = 1\n";
    assert!(parse(text).is_err());
  }

  #[test]
  fn loads_files_in_name_order() {
    let rules = load(&fixtures()).unwrap();
    assert_eq!(rules.len(), 8);
    assert_eq!(rules[0].id, "no-unapproved-commit");
    assert_eq!(rules[4].id, "format-via-fml");
  }

  #[test]
  fn parse_error_names_the_file() {
    let dir = std::env::temp_dir().join("ahok-rules-bad");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
      dir.join("bad.toml"),
      "[[rule]]
",
    )
    .unwrap();
    let err = load(&dir).unwrap_err().to_string();
    assert!(err.starts_with(&dir.join("bad.toml").display().to_string()));
  }
}
