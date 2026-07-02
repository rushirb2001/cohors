//! The bounded command runner (ADR-020): run a shell command in each target
//! repo over a bounded thread pool, capping captured output per stream.
//!
//! One implementation, shared by every surface. Each surface decides how to
//! present a [`RunResult`] (the MCP serializes it to JSON, the CLI prints it);
//! the runner itself stays presentation-free.

use std::time::Duration;

use cohors_core::RepoSnapshot;
use serde::Serialize;

/// Maximum captured output per stream per repo. A chatty command can't flood the
/// caller; anything past this is dropped and flagged with `truncated`.
pub const RUN_OUTPUT_CAP: usize = 64 * 1024;
/// Per-repo wall-clock bound for `run` when the caller doesn't set one, so one
/// hung command can't stall the whole fan-out.
pub const DEFAULT_RUN_TIMEOUT_SECS: u64 = 120;
/// How many repos any bulk action executes concurrently (ADR-020). Bounded on
/// purpose: 500 simultaneous `git fetch`es would hammer remotes and the disk;
/// a pool of 8 keeps a large fleet fast while staying polite.
const ACTION_POOL_SIZE: usize = 8;

/// The one process-wide bounded pool every bulk-action path shares (the `run`
/// fan-out, the per-repo git actions, and the TUI's background action spawns).
/// `None` if the pool couldn't be built — callers then degrade to sequential.
fn action_pool() -> Option<&'static rayon::ThreadPool> {
    use std::sync::OnceLock;
    static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(ACTION_POOL_SIZE)
            .thread_name(|i| format!("cohors-action-{i}"))
            .build()
            .ok()
    })
    .as_ref()
}

/// Run `f` on the shared bounded action pool (or inline if the pool couldn't be
/// built). Lets a front-end stream per-repo work through the same 8-wide pool
/// instead of spawning one OS thread per repo.
pub fn spawn_bounded(f: impl FnOnce() + Send + 'static) {
    match action_pool() {
        Some(pool) => pool.spawn(f),
        None => f(),
    }
}

/// Run `op` over `items` on the shared pool, preserving order. Sequential
/// fallback when the pool is unavailable.
pub(crate) fn map_bounded<T: Sync, R: Send>(items: &[T], op: impl Fn(&T) -> R + Sync) -> Vec<R> {
    use rayon::prelude::*;
    match action_pool() {
        Some(pool) => pool.install(|| items.par_iter().map(&op).collect()),
        None => items.iter().map(op).collect(),
    }
}

/// One repo's result from a [`run_each`] fan-out. Field names match the MCP wire
/// shape, so a surface can `serde_json::to_value` it directly.
#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    /// The repo's stable id.
    pub repo: String,
    /// Exited 0 and wasn't killed by the timeout.
    pub ok: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    /// Either stream was clipped at [`RUN_OUTPUT_CAP`].
    pub truncated: bool,
    pub timed_out: bool,
}

/// Truncate a captured stream to [`RUN_OUTPUT_CAP`] bytes on a char boundary.
pub fn cap_output(mut s: String) -> (String, bool) {
    if s.len() <= RUN_OUTPUT_CAP {
        return (s, false);
    }
    let mut end = RUN_OUTPUT_CAP;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    (s, true)
}

/// Monotonic, process-global run id (ADR-020).
pub fn next_run_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Run `command` in each target's directory over a bounded thread pool, returning
/// per-repo results in target order. Falls back to sequential if the pool can't
/// be built. Targets must have a path (callers filter path-less repos out).
pub fn run_each(targets: &[RepoSnapshot], command: &str, timeout: Duration) -> Vec<RunResult> {
    let exec = |s: &RepoSnapshot| {
        let path = s.path.as_ref().expect("action targets have a path");
        let out = crate::git::run_command_timeout(path, command, timeout);
        let (stdout, t1) = cap_output(out.stdout);
        let (stderr, t2) = cap_output(out.stderr);
        RunResult {
            repo: s.id.0.clone(),
            ok: out.code == 0 && !out.timed_out,
            exit_code: out.code,
            stdout,
            stderr,
            truncated: t1 || t2,
            timed_out: out.timed_out,
        }
    };

    map_bounded(targets, exec)
}
