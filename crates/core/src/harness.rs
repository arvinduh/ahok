//! Harness adapters: each harness's hook JSON in, `event::Event` out.
//!
//! Each submodule owns one harness's wire format. This module owns what they
//! share: event-name lookup and the error type. Reading stdin and writing
//! stdout belong to the binary.

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod git;

use std::path;

use serde;
use serde_json;
use thiserror;

use crate::event;

/// A coding-agent harness ahok can wire up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
  /// Claude Code.
  Claude,
  /// The Codex CLI.
  Codex,
  /// Google Antigravity.
  Agy,
  /// Git, through its `commit-msg` and `pre-push` hooks.
  Git,
}

impl Harness {
  /// Every harness, in the order they are listed to the user.
  const ALL: [Self; 4] = [Self::Claude, Self::Codex, Self::Agy, Self::Git];

  /// The harness's name in `ahok.toml` and on the command line.
  #[must_use]
  pub const fn name(self) -> &'static str {
    match self {
      Self::Claude => "claude",
      Self::Codex => "codex",
      Self::Agy => "agy",
      Self::Git => "git",
    }
  }
}

impl Harness {
  /// Where the harness reads the user's instructions; git reads none.
  pub(crate) const fn instructions(self) -> Option<&'static str> {
    match self {
      Self::Claude => Some(claude::INSTRUCTIONS),
      Self::Codex => Some(codex::INSTRUCTIONS),
      Self::Agy => Some(antigravity::INSTRUCTIONS),
      Self::Git => None,
    }
  }

  /// The directory the harness reads skills from, one per child; git reads
  /// none.
  pub(crate) const fn skills(self) -> Option<&'static str> {
    match self {
      Self::Claude => Some(claude::SKILLS),
      Self::Codex => Some(codex::SKILLS),
      Self::Agy => Some(antigravity::SKILLS),
      Self::Git => None,
    }
  }
}

impl std::str::FromStr for Harness {
  type Err = String;

  fn from_str(name: &str) -> Result<Self, Self::Err> {
    Self::ALL
      .into_iter()
      .find(|harness| harness.name() == name)
      .ok_or_else(|| format!("expected claude, codex, agy or git, got {name}"))
  }
}

/// The hook events a workspace needs answered, and where it lives.
pub struct Wiring<'a> {
  /// The workspace, baked into each hook command as `-C`.
  pub workspace: &'a path::Path,
  /// Register `PreToolUse`: the workspace has rules.
  pub rules: bool,
  /// Register `PostToolUse` on edits: the workspace has edit checks.
  pub edit: bool,
  /// Register `Stop`: the workspace has stop checks.
  pub stop: bool,
}

impl Wiring<'_> {
  /// The shell command a harness runs for `event`: `ahok hook`, or `{}`
  /// (allow) when ahok is not installed.
  pub(crate) fn command(&self, harness: Harness, event: &str) -> String {
    let workspace = self.workspace.display().to_string().replace('\'', r"'\''");
    format!(
      "command -v ahok >/dev/null 2>&1 && ahok hook -C '{workspace}' {} \
       {event} || echo '{{}}'",
      harness.name()
    )
  }
}

/// Why a hook payload could not become an `event::Event`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
  /// The payload is not the JSON shape the harness documents.
  #[error(transparent)]
  Json(#[from] serde_json::Error),
  /// The event name is not one ahok handles, e.g. after a harness update.
  #[error("unknown hook event `{name}`")]
  UnknownEvent {
    /// Event name as passed on the command line.
    name: String,
  },
}

/// Builds the `event::Kind` for `name`, calling `tool` only for tool events.
///
/// `already_blocked` is only used for `Stop`.
fn kind(
  name: &str,
  tool: impl FnOnce() -> event::Tool,
  already_blocked: bool,
) -> Result<event::Kind, Error> {
  Ok(match name {
    "PreToolUse" => event::Kind::PreToolUse(tool()),
    "PostToolUse" => event::Kind::PostToolUse(tool()),
    "Stop" => event::Kind::Stop { already_blocked },
    _ => {
      return Err(Error::UnknownEvent {
        name: name.to_owned(),
      });
    }
  })
}
