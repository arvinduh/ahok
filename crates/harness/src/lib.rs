//! Harness adapters: each harness's hook JSON in, `event::Event` out.
//!
//! Each submodule owns one harness's wire format. This crate owns what they
//! share: event-name lookup and the error type. Reading stdin and writing
//! stdout belong to the binary.

#![deny(clippy::print_stdout, clippy::print_stderr)]

pub mod antigravity;
pub mod claude;

use event;
use serde_json;
use thiserror;

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
    "PostToolUseFailure" => event::Kind::PostToolUseFailure(tool()),
    "SessionStart" => event::Kind::SessionStart,
    "Stop" => event::Kind::Stop { already_blocked },
    "PreInvocation" => event::Kind::PreInvocation,
    _ => {
      return Err(Error::UnknownEvent {
        name: name.to_owned(),
      });
    }
  })
}
