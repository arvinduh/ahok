//! `ahok hook <harness> <event>`: answer one harness hook call.
//!
//! This module owns the hook process contract: read the payload from stdin,
//! always write a JSON reply to stdout, and exit 0. Any failure is logged to
//! `errors.log` and the call is allowed (fail open). Parsing, rules, and
//! replies come from the library.

use std::env;
use std::io;
use std::io::Read;
use std::path;
use std::process;
use std::time;

use ahok::dirs;
use ahok::dispatch;
use ahok::event;
use ahok::harness;
use ahok::journal;
use ahok::rules;
use ahok::session;

/// Arguments of `ahok hook`.
#[derive(clap::Args)]
pub struct Args {
  /// Harness that fired the hook: claude.
  harness: String,
  /// Hook event name, such as `PreToolUse`.
  event: String,
}

/// Why a hook call failed open.
#[derive(Debug, thiserror::Error)]
enum Error {
  #[error("reading stdin: {0}")]
  Stdin(#[from] io::Error),
  #[error(transparent)]
  Harness(#[from] harness::Error),
  #[error(transparent)]
  Rules(#[from] rules::Error),
  #[error("unsupported harness `{0}`")]
  UnknownHarness(String),
  #[error("no config directory: set AHOK_CONFIG_DIR or HOME")]
  NoConfig,
}

impl Args {
  /// Answers the hook call on stdout. Always exits 0.
  pub fn run(self) -> process::ExitCode {
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
        "{}".to_owned()
      }
    };
    println!("{reply}");
    process::ExitCode::SUCCESS
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
    if self.harness != "claude" {
      return Err(Error::UnknownHarness(self.harness.clone()));
    }
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let event = harness::claude::parse(&self.event, &input)?;
    let (reply, found) = match &event.kind {
      event::Kind::PreToolUse(_) => {
        let config = dirs::config().ok_or(Error::NoConfig)?;
        let rules = rules::load(&config.join("rules"))?;
        let var_set = |name: &str| env::var_os(name).is_some();
        let session = session::State::default();
        let env = rules::Env {
          os: env::consts::OS,
          var_set: &var_set,
          session: &session,
        };
        let found = rules::evaluate(&rules, &event, &env);
        let reply = harness::claude::respond(found.as_ref());
        (
          reply,
          found.map(|found| (found.id.to_owned(), found.decision)),
        )
      }
      event::Kind::PostToolUse(event::Tool::Edit { paths }) => {
        let failure = dispatch::format(&event.cwd, paths);
        (harness::claude::feedback(failure.as_slice()), None)
      }
      event::Kind::Stop {
        already_blocked: false,
      } => (harness::claude::feedback(&dispatch::gate(&event.cwd)), None),
      _ => ("{}".to_owned(), None),
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
