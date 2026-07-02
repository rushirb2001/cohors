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

// The public contract, deliberately small (everything else is internal):
// - the registry: what actions exist + their safety metadata (drives catalogs,
//   keymaps, and the parity tests);
// - dispatch: the one verb→primitive entry point for machine surfaces;
// - the git primitives: for the TUI/CLI, whose targets come from interactive
//   selection rather than a JSON selector;
// - run_each: the bounded fan-out runner the CLI prints from.
// The orchestration layer (git_action, resolve/cap/dry-run/audit helpers) is
// crate-internal — only `dispatch` composes it, so it can change freely.
pub use dispatch::{Caps, dispatch};
pub use git::{commit, fetch, pull_ff, push, run_command, stash_push};
pub use registry::{ActionDef, Tier, find as find_action, registry};
pub use runner::{RunResult, run_each};
