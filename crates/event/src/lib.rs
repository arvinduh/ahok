//! Harness-neutral hook events.
//!
//! Harness adapters translate each harness's stdin JSON into these types, and
//! the rule engine evaluates them. This crate parses nothing and knows no
//! harness; tool names are normalized before they reach it.

#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::path;

use serde;

/// One hook invocation, translated from a harness's JSON.
#[derive(Debug)]
pub struct Event {
  /// Harness session id; keys once-per-session state.
  pub session_id: String,
  /// Directory the agent works in; relative paths in a tool call resolve
  /// against it. Empty when the harness didn't say.
  pub cwd: path::PathBuf,
  /// The lifecycle point the hook fired at.
  pub kind: Kind,
}

/// The lifecycle point a hook fired at, with the payload it carries.
#[derive(Debug)]
pub enum Kind {
  /// Before a tool runs; the only point where a call can be blocked.
  PreToolUse(Tool),
  /// After a tool succeeded.
  PostToolUse(Tool),
  /// After a tool failed.
  PostToolUseFailure(Tool),
  /// A session started or resumed.
  SessionStart,
  /// The agent finished its turn.
  Stop {
    /// The agent is already continuing because a Stop hook blocked it;
    /// blocking again could loop forever.
    already_blocked: bool,
  },
  /// Before a model call (Antigravity only).
  PreInvocation,
}

/// The tool call a tool event is about.
#[derive(Debug)]
pub enum Tool {
  /// A shell command, as the agent wrote it.
  Shell {
    /// Full command line, unparsed.
    command: String,
  },
  /// A file write or edit.
  Edit {
    /// Files the call creates or changes.
    paths: Vec<path::PathBuf>,
  },
  /// A skill being loaded into the agent's context.
  Skill {
    /// Skill name, such as `rust-guide`.
    name: String,
  },
  /// Any other tool, kept by harness name so rules can still match it.
  Other {
    /// Tool name as the harness reports it.
    name: String,
  },
}

/// What happens to a call a rule matched; earlier variants win.
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
