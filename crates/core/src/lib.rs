//! ahok's library: hook events, harness adapters, rules, checks, and links.
//!
//! Everything here returns data and never prints or exits. Parsing the
//! command line, writing replies, and choosing exit codes belong to the
//! `ahok` binary, which is this crate's only caller.

#![deny(clippy::print_stdout, clippy::print_stderr)]

pub mod agents;
pub mod check;
mod de;
pub mod event;
pub mod harness;
pub mod link;
pub mod manifest;
pub mod rules;
pub mod state;
