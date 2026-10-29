//! Health checks for an ahok install.
//!
//! This module checks what the hook depends on: the rules load, the tools
//! it runs are on `PATH`, and each harness config calls `ahok hook`. It
//! returns results as data; printing them belongs to `ahok doctor`.

use std::env;
use std::fs;
use std::path;
use std::process;

use rules;
use state::dirs;

/// How a check came out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
  /// Works.
  Ok,
  /// Works partly, or an optional part is missing.
  Warn,
  /// Broken: hooks will fail open or skip work.
  Fail,
}

/// One check's result.
#[derive(Debug)]
pub struct Check {
  /// What was checked, such as `rules` or `fml`.
  pub name: String,
  /// How it came out.
  pub status: Status,
  /// What was found, or what is wrong.
  pub detail: String,
}

/// Runs every check, in a stable order.
///
/// # Side Effects
///
/// Reads the rules and harness configs, and runs `<tool> --version` for
/// each tool.
#[must_use]
pub fn run() -> Vec<Check> {
  let mut checks = vec![rules_check()];
  checks.push(tool("fml", Status::Fail));
  checks.push(tool("git", Status::Fail));
  checks.push(tool("mise", Status::Warn));
  let home = env::home_dir().unwrap_or_default();
  checks.push(harness(
    "claude",
    &home.join(".claude").join("settings.json"),
  ));
  let agy = home.join(".gemini").join("config").join("hooks.json");
  checks.push(harness("agy", &agy));
  checks
}

/// Whether the rules directory loads.
fn rules_check() -> Check {
  let name = "rules".to_owned();
  let Some(dir) = dirs::config().map(|config| config.join("rules")) else {
    let detail = "no config dir: set AHOK_CONFIG_DIR or HOME".to_owned();
    return Check {
      name,
      status: Status::Fail,
      detail,
    };
  };
  match rules::load(&dir) {
    Ok(rules) => Check {
      name,
      status: Status::Ok,
      detail: format!("{} rules in {}", rules.len(), dir.display()),
    },
    Err(err) => Check {
      name,
      status: Status::Fail,
      detail: format!("{}: {err}", dir.display()),
    },
  }
}

/// Whether `program --version` runs; `missing` is the status if not.
fn tool(program: &str, missing: Status) -> Check {
  let output = process::Command::new(program)
    .arg("--version")
    .stdin(process::Stdio::null())
    .output();
  let (status, detail) = match output {
    Ok(output) if output.status.success() => {
      let text = String::from_utf8_lossy(&output.stdout);
      (
        Status::Ok,
        text.lines().next().unwrap_or_default().to_owned(),
      )
    }
    Ok(output) => (missing, format!("`{program} --version` {}", output.status)),
    Err(err) => (missing, format!("not runnable: {err}")),
  };
  Check {
    name: program.to_owned(),
    status,
    detail,
  }
}

/// Whether the harness config at `path` registers `ahok hook`.
fn harness(name: &str, path: &path::Path) -> Check {
  let (status, detail) = match fs::read_to_string(path) {
    Ok(text) if text.contains("ahok hook") => {
      (Status::Ok, format!("{} calls ahok hook", path.display()))
    }
    Ok(_) => (
      Status::Warn,
      format!("{} doesn't call ahok hook", path.display()),
    ),
    Err(err) => (Status::Warn, format!("{}: {err}", path.display())),
  };
  Check {
    name: name.to_owned(),
    status,
    detail,
  }
}
