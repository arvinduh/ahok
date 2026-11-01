//! Files ahok keeps between hook calls, and where they live.
//!
//! This crate owns the config and state directories, the per-session file,
//! and the error log.
//! What to record and when belongs to the `ahok` binary.

#![deny(clippy::print_stdout, clippy::print_stderr)]

pub mod dirs;
pub mod journal;
pub mod session;
