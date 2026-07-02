//! cohors-actions — the write-side adapter.
//!
//! Peer to `cohors-git` (which reads): this crate owns the *what* of every
//! mutating, side-effecting operation on the fleet — the git primitives
//! (fetch/pull/push/commit/stash), the bounded command runner, and the action
//! registry that drives every front-end (TUI, MCP, web). It depends only on
//! `cohors-core`; it is **native** (shells out to `git`, spawns threads) and is
//! deliberately kept out of the WASM-safe core's dependency tree.

#![forbid(unsafe_code)]

mod dispatch;
mod git;
mod orchestrate;
mod registry;
mod runner;

pub use dispatch::{Caps, dispatch};
pub use git::{
    RunOutcome, commit, fetch, pull_ff, push, run_command, run_command_timeout, stash_push,
};
pub use orchestrate::{
    action_selector, audit, command_allowed, command_matches, dry_run_preview, git_action,
    is_dry_run, no_targets, resolve_targets, within_cap,
};
pub use registry::{ActionDef, Tier, find as find_action, registry};
pub use runner::{
    DEFAULT_RUN_TIMEOUT_SECS, RUN_OUTPUT_CAP, RunResult, cap_output, next_run_id, run_each,
};
