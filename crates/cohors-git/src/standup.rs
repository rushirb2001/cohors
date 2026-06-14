//! Standup commit collection (v0.2): walk a repo's history and return the
//! commits authored by a given user within a time window, tagged with the repo
//! name. Implemented in the standup git-collection step.

use camino::Utf8Path;
use cohors_core::StandupCommit;

/// Collect commits authored by `author_email` reachable from `HEAD` whose time
/// falls in `[since, until)` (Unix seconds), tagged with the repo's directory
/// name. Best-effort: an unreadable repo yields an empty list.
pub fn collect_commits(
    path: &Utf8Path,
    author_email: &str,
    since: i64,
    until: i64,
) -> Vec<StandupCommit> {
    // Stub — filled by the standup git-collection agent.
    let _ = (path, author_email, since, until);
    Vec::new()
}
