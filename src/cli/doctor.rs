//! `ahok doctor`: report whether the install works.
//!
//! This module prints one line per check from the library's `doctor`
//! module and turns any failure into exit code 1.

use std::process;

use ahok::doctor;

/// Prints every check; exits 1 if any check failed.
///
/// A free function rather than `Args::run`, since the command takes no
/// arguments.
pub fn run() -> process::ExitCode {
  let checks = doctor::run();
  for check in &checks {
    let label = match check.status {
      doctor::Status::Ok => "ok  ",
      doctor::Status::Warn => "warn",
      doctor::Status::Fail => "FAIL",
    };
    println!("{label} {:<8} {}", check.name, check.detail);
  }
  if checks
    .iter()
    .any(|check| check.status == doctor::Status::Fail)
  {
    process::ExitCode::FAILURE
  } else {
    process::ExitCode::SUCCESS
  }
}
