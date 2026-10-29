//! The `ahok` binary: the process host.
//!
//! This file parses arguments and maps the command's outcome onto an exit
//! code. Commands live in `cli`; policy lives in the
//! crates under `crates/`.

mod cli;

use std::process;

use clap::Parser;

fn main() -> process::ExitCode {
  match cli::Args::parse().run() {
    Ok(cli::Status::Clean) => process::ExitCode::SUCCESS,
    Ok(cli::Status::Failed) => process::ExitCode::FAILURE,
    Err(err) => {
      eprintln!("[ERROR] {err}");
      process::ExitCode::FAILURE
    }
  }
}
