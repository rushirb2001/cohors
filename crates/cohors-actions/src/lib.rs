//! cohors-actions — the write-side adapter.
//!
//! Peer to `cohors-git` (which reads): this crate owns the *what* of every
//! mutating, side-effecting operation on the fleet — the git primitives
//! (fetch/pull/push/commit/stash), the bounded command runner, and the action
//! registry that drives every front-end (TUI, MCP, web). It depends only on
//! `cohors-core`; it is **native** (shells out to `git`, spawns threads) and is
//! deliberately kept out of the WASM-safe core's dependency tree.

#![forbid(unsafe_code)]

mod git;
mod runner;

pub use git::{
    RunOutcome, commit, fetch, pull_ff, push, run_command, run_command_timeout, stash_push,
};
pub use runner::{
    DEFAULT_RUN_TIMEOUT_SECS, RUN_OUTPUT_CAP, RunResult, cap_output, next_run_id, run_each,
};
