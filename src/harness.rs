//! Harness adapters: each harness's hook JSON in, `event::Event` out.
//!
//! Each submodule owns one harness's wire format. This module owns what they
//! share: event-name lookup and the error type. Reading stdin and writing
//! stdout belong to the binary.

use serde_json;

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
