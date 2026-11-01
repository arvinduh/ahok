//! `ahok hook -C <workspace> <harness> <event>`: answer one hook call.
//!
//! This module owns the agent hook process contract: read the payload from
//! stdin, always write a JSON reply to stdout, and end `Clean` so the
//! process exits 0. Any failure is logged to `errors.log` and the call is
//! allowed (fail open). Git's hooks follow git's contract instead, in `git`.
//! Parsing, rules, and replies come from `ahok_core`.

mod git;

use std::env;
use std::io;
use std::path;
use std::time;

use ahok_core::check;
use ahok_core::event;
use ahok_core::harness;
use ahok_core::manifest;
use ahok_core::rules;
use ahok_core::state::dirs;
use ahok_core::state::journal;
use ahok_core::state::session;
use clap;
use thiserror;

use crate::cli;

/// Arguments of `ahok hook`.
#[derive(clap::Args)]
pub struct Args {
  /// Workspace whose `ahok.toml` holds the rules and checks.
  #[arg(short = 'C', long)]
  workspace: path::PathBuf,
  /// Harness that fired the hook: claude, codex, agy or git.
  harness: harness::Harness,
  /// Hook event name, such as `PreToolUse` or `commit-msg`.
  event: String,
  /// What git passes its hook, such as the message file.
  #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
  passed: Vec<String>,
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
  /// The workspace's `ahok.toml` failed to load.
  #[error(transparent)]
  Manifest(#[from] manifest::Error),
  /// The rules directory failed to load.
  #[error(transparent)]
  Rules(#[from] rules::Error),
  /// The session file could not be read.
  #[error("session state: {0}")]
  Session(io::Error),
}

impl Args {
  /// Answers the hook call on stdout. Always `Clean` for agents, whose
  /// hooks fail open; `Failed` for git when a rule is broken.
  pub fn run(self) -> cli::Status {
    if self.harness == harness::Harness::Git {
      return git::run(&self.workspace, &self.event, &self.passed);
    }
    let unix_ms = time::SystemTime::now()
      .duration_since(time::UNIX_EPOCH)
      .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(0));
    let state = dirs::state();
    let reply = match self.decide(unix_ms, state.as_deref()) {
      Ok(reply) => reply,
      Err(err) => {
        if let Some(dir) = &state {
          let message =
            format!("hook {} {}: {err}", self.harness.name(), self.event);
          drop(journal::error(dir, unix_ms, &message));
        }
        match self.harness {
          harness::Harness::Agy if self.event == "PreToolUse" => {
            r#"{"decision":"allow"}"#.to_owned()
          }
          _ => "{}".to_owned(),
        }
      }
    };
    println!("{reply}");
    cli::Status::Clean
  }

  /// Parses the payload, runs what the event calls for, and returns the
  /// reply.
  ///
  /// `PreToolUse` evaluates the rules; `PostToolUse` on an edit runs the
  /// edit checks on the edited files; `Stop` runs the stop checks on every
  /// file changed since the branch's base, unless a Stop hook already
  /// blocked once.
  fn decide(
    &self,
    unix_ms: u64,
    state: Option<&path::Path>,
  ) -> Result<String, Error> {
    let mut input = String::new();
    io::Read::read_to_string(&mut io::stdin(), &mut input)?;
    // Codex shares Claude Code's reply shape for everything but rules.
    let (parse, respond, feedback): (Parse, Respond, Feedback) =
      match self.harness {
        harness::Harness::Claude => (
          harness::claude::parse,
          harness::claude::respond,
          harness::claude::feedback,
        ),
        harness::Harness::Codex => (
          harness::codex::parse,
          harness::codex::respond,
          harness::claude::feedback,
        ),
        harness::Harness::Agy => (
          harness::antigravity::parse,
          harness::antigravity::respond,
          harness::antigravity::feedback,
        ),
        // `run` hands git's hooks to `git::run`.
        harness::Harness::Git => return Ok("{}".to_owned()),
      };
    let event = parse(&self.event, &input)?;
    let manifest = manifest::load(&self.workspace)?;
    Ok(match &event.kind {
      event::Kind::PreToolUse(tool) => {
        let dir = manifest.agents.and_then(|a| a.rules(&self.workspace));
        let rules = match dir {
          Some(dir) => rules::load(&dir)?,
          None => Vec::new(),
        };
        pre_tool_use(&event, tool, &rules, state, unix_ms, respond)?
      }
      event::Kind::PostToolUse(event::Tool::Edit { paths }) => {
        let on = check::On::Edit;
        let failures = check::run(&manifest.check, on, &event.cwd, paths);
        // Antigravity can't feed a failed edit check back to the agent.
        match self.harness {
          harness::Harness::Agy => "{}".to_owned(),
          _ => feedback(&failures),
        }
      }
      event::Kind::Stop {
        already_blocked: false,
      } => match check::changed_since_base(&event.cwd) {
        Some((root, files)) => {
          feedback(&check::run(&manifest.check, check::On::Stop, &root, &files))
        }
        None => "{}".to_owned(),
      },
      _ => "{}".to_owned(),
    })
  }
}

/// An adapter's payload parser, such as `harness::claude::parse`.
type Parse = fn(&str, &str) -> Result<event::Event, harness::Error>;

/// An adapter's `PreToolUse` reply renderer.
type Respond = fn(Option<&rules::Match>) -> String;

/// An adapter's renderer for failed checks.
type Feedback = fn(&[check::Failure]) -> String;

/// Evaluates the rules for a `PreToolUse` call and updates the session.
///
/// # Side Effects
///
/// Records skill loads and once-per-session fires in the session file;
/// a failed save is logged, not returned, so the decision still stands.
fn pre_tool_use(
  event: &event::Event,
  tool: &event::Tool,
  rules: &[rules::Rule],
  state: Option<&path::Path>,
  unix_ms: u64,
  respond: impl FnOnce(Option<&rules::Match>) -> String,
) -> Result<String, Error> {
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
  let found = rules::evaluate(rules, event, &env);
  let reply = respond(found.as_ref());
  let fired = found
    .as_ref()
    .filter(|found| found.once)
    .map(|found| found.id);
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
  Ok(reply)
}
