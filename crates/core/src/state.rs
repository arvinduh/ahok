//! Files ahok keeps between hook calls, and where they live.
//!
//! This module owns the config and state directories, the per-session file,
//! and the error log.
//! What to record and when belongs to the `ahok` binary.

pub mod dirs;
pub mod journal;
pub mod session;
