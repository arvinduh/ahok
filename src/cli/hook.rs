//! `ahok hook <harness> <event>`: answer one harness hook call.
//!
//! This module owns the hook process contract: read the payload from stdin,
//! always write a JSON reply to stdout, and end `Clean` so the process exits
//! 0. Any failure is logged to `errors.log` and the call is allowed (fail
//! open). Parsing, rules, and replies come from the crates.

use std::env;
use std::io;
use std::path;
use std::time;

use clap;
use dispatch;
use event;
use harness;
use rules;
use state::dirs;
use state::journal;
use state::session;
use thiserror;

use crate::cli;

/// Arguments of `ahok hook`.
#[derive(clap::Args)]
pub struct Args {
  /// Harness that fired the hook: claude, codex, antigravity or agy.
  harness: String,
  /// Hook event name, such as `PreToolUse`.
  event: String,
}

/// Why a hook call failed open.
#[derive(Debug, thiserror::Error)]
enum Error {
  /// The payload could not be read from stdin.
  #[error("reading stdin: {0}")]
  Stdin(#[from] io::Error),
  /// The payload is not a valid hook call.
  #[error(transparent)]
  Harness(#[from] harness::Error),
  /// The rules directory failed to load.
  #[error(transparent)]
  Rules(#[from] rules::Error),
  /// The harness argument names no adapter.
  #[error("unsupported harness `{0}`")]
  UnknownHarness(String),
  /// The session file could not be read.
  #[error("session state: {0}")]
  Session(io::Error),
  /// No config directory could be located.
  #[error("no config directory: set AHOK_CONFIG_DIR or HOME")]
  NoConfig,
}

impl Args {
  /// Answers the hook call on stdout. Always `Clean`: hooks fail open.
  pub fn run(self) -> cli::Status {
    let start = time::Instant::now();
    let unix_ms = time::SystemTime::now()
      .duration_since(time::UNIX_EPOCH)
      .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(0));
    let state = dirs::state();
    let reply = match self.decide(start, unix_ms, state.as_deref()) {
      Ok(reply) => reply,
      Err(err) => {
        if let Some(dir) = &state {
          let message = format!("hook {} {}: {err}", self.harness, self.event);
          drop(journal::error(dir, unix_ms, &message));
        }
        match self.harness.as_str() {
          "antigravity" | "agy" if self.event == "PreToolUse" => {
            r#"{"decision":"allow"}"#.to_owned()
          }
          _ => "{}".to_owned(),
        }
      }
    };
    println!("{reply}");
    cli::Status::Clean
  }

  /// Parses the payload, runs what the event calls for, logs the call, and
  /// returns the reply.
  ///
  /// `PreToolUse` evaluates the rules; `PostToolUse` on an edit formats the
  /// files; `Stop` runs the gate unless a Stop hook already blocked once.
  fn decide(
    &self,
    start: time::Instant,
    unix_ms: u64,
    state: Option<&path::Path>,
  ) -> Result<String, Error> {
    let mut input = String::new();
    io::Read::read_to_string(&mut io::stdin(), &mut input)?;
    let (reply, found) = match self.harness.as_str() {
      // Codex shares Claude Code's reply shape for everything but rules.
      "claude" | "codex" => {
        let (parse, respond): (Parse, Respond) = match self.harness.as_str() {
          "claude" => (harness::claude::parse, harness::claude::respond),
          _ => (harness::codex::parse, harness::codex::respond),
        };
        let event = parse(&self.event, &input)?;
        match &event.kind {
          event::Kind::PreToolUse(tool) => {
            pre_tool_use(&event, tool, state, unix_ms, respond)?
          }
          event::Kind::PostToolUse(event::Tool::Edit { paths }) => {
            let failure = dispatch::format(&event.cwd, paths);
            (harness::claude::feedback(failure.as_slice()), None)
          }
          event::Kind::Stop {
            already_blocked: false,
          } => (harness::claude::feedback(&dispatch::gate(&event.cwd)), None),
          _ => ("{}".to_owned(), None),
        }
      }
      "antigravity" | "agy" => {
        let event = harness::antigravity::parse(&self.event, &input)?;
        match &event.kind {
          event::Kind::PreToolUse(tool) => pre_tool_use(
            &event,
            tool,
            state,
            unix_ms,
            harness::antigravity::respond,
          )?,
          event::Kind::PostToolUse(event::Tool::Edit { paths }) => {
            drop(dispatch::format(&event.cwd, paths));
            ("{}".to_owned(), None)
          }
          event::Kind::Stop {
            already_blocked: false,
          } => (
            harness::antigravity::feedback(&dispatch::gate(&event.cwd)),
            None,
          ),
          _ => ("{}".to_owned(), None),
        }
      }
      _ => return Err(Error::UnknownHarness(self.harness.clone())),
    };
    if let Some(dir) = state {
      let entry = journal::Decision {
        unix_ms,
        harness: &self.harness,
        event: &self.event,
        rule: found.as_ref().map(|(id, _)| id.as_str()),
        decision: found.as_ref().map(|&(_, decision)| decision),
        micros: u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX),
      };
      if let Err(err) = journal::decision(dir, &entry) {
        drop(journal::error(
          dir,
          unix_ms,
          &format!("decision log: {err}"),
        ));
      }
    }
    Ok(reply)
  }
}

/// An adapter's payload parser, such as `harness::claude::parse`.
type Parse = fn(&str, &str) -> Result<event::Event, harness::Error>;

/// An adapter's `PreToolUse` reply renderer.
type Respond = fn(Option<&rules::Match>) -> String;

/// Evaluates the rules for a `PreToolUse` call and updates the session.
///
/// Returns the reply and the deciding rule's id and decision.
///
/// # Side Effects
///
/// Records skill loads and once-per-session fires in the session file;
/// a failed save is logged, not returned, so the decision still stands.
fn pre_tool_use(
  event: &event::Event,
  tool: &event::Tool,
  state: Option<&path::Path>,
  unix_ms: u64,
  respond: impl FnOnce(Option<&rules::Match>) -> String,
) -> Result<(String, Option<(String, event::Decision)>), Error> {
  let config = dirs::config().ok_or(Error::NoConfig)?;
  let rules = rules::load(&config.join("rules"))?;
  let id = &event.session_id;
  let mut session = match state {
    Some(dir) => session::State::load(dir, id).map_err(Error::Session)?,
    None => session::State::default(),
  };
  let var_set = |name: &str| env::var_os(name).is_some();
  let env = rules::Env {
    os: env::consts::OS,
    var_set: &var_set,
    session: &session,
  };
  let found = rules::evaluate(&rules, event, &env);
  let reply = respond(found.as_ref());
  let fired = found
    .as_ref()
    .filter(|found| found.once)
    .map(|found| found.id);
  let decided = found.map(|found| (found.id.to_owned(), found.decision));
  let loaded = match tool {
    event::Tool::Skill { name } => Some(name.as_str()),
    _ => None,
  };
  if (fired.is_some() || loaded.is_some())
    && let Some(dir) = state
  {
    if let Some(rule) = fired {
      session.add_fired(rule);
    }
    if let Some(skill) = loaded {
      session.add_skill(skill);
    }
    if let Err(err) = session.save(dir, id) {
      drop(journal::error(
        dir,
        unix_ms,
        &format!("session save: {err}"),
      ));
    }
  }
  Ok((reply, decided))
}
