//! `ahok doctor`: report whether the install works.
//!
//! This module prints one line per check from the library's `doctor`
//! module and reports `Failed` if any check failed.

use ahok::doctor;

use crate::cli;

/// Prints every check; `Failed` if any check failed.
///
/// A free function rather than `Args::run`, since the command takes no
/// arguments.
pub fn run() -> cli::Status {
  let checks = doctor::run();
  for check in &checks {
    let label = match check.status {
      doctor::Status::Ok => "ok  ",
      doctor::Status::Warn => "warn",
      doctor::Status::Fail => "FAIL",
    };
    println!("{label} {:<8} {}", check.name, check.detail);
  }
  let failed = checks
    .iter()
    .any(|check| check.status == doctor::Status::Fail);
  if failed {
    cli::Status::Failed
  } else {
    cli::Status::Clean
  }
}
