//! Standup commit collection (v0.2): walk a repo's history and return the
//! commits authored by a given user within a time window, tagged with the repo
//! name. Implemented in the standup git-collection step.
//!
//! Like the snapshot path, the real walk runs on `git2` (libgit2) behind the
//! `git2-fallback` feature (on by default); without it the collector degrades to
//! an empty list rather than failing to compile. See ADR-004.

use camino::Utf8Path;
use cohors_core::StandupCommit;

/// Collect commits authored by `author_email` reachable from `HEAD` whose time
/// falls in `[since, until)` (Unix seconds), tagged with the repo's directory
/// name. Best-effort: an unreadable repo yields an empty list.
#[cfg(feature = "git2-fallback")]
pub fn collect_commits(
    path: &Utf8Path,
    author_email: &str,
    since: i64,
    until: i64,
) -> Vec<StandupCommit> {
    let repo = match git2::Repository::open(path.as_std_path()) {
        Ok(repo) => repo,
        Err(err) => {
            tracing::debug!(%path, error = %err, "git2 open failed; no standup commits collected");
            return Vec::new();
        }
    };

    // The repo's directory name tags every commit; fall back to the full path
    // when there's no final component (e.g. a root path).
    let repo_name = path
        .file_name()
        .unwrap_or_else(|| path.as_str())
        .to_string();

    // Walk history newest-first from HEAD. Any setup failure degrades to empty.
    let mut revwalk = match repo.revwalk() {
        Ok(rw) => rw,
        Err(_) => return Vec::new(),
    };
    revwalk.set_sorting(git2::Sort::TIME).ok();
    if revwalk.push_head().is_err() {
        // No HEAD (e.g. an unborn branch) → nothing to collect.
        return Vec::new();
    }

    let mut out = Vec::new();
    for oid in revwalk {
        let Ok(oid) = oid else { continue };
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };

        // Commit (committer) time in Unix seconds — used for both the window
        // filter and the recorded timestamp, so the two always agree.
        let timestamp = commit.time().seconds();

        // TIME sorting is newest-first, so once we drop below `since` every
        // remaining commit is older too: stop walking.
        if timestamp < since {
            break;
        }
        // Upper bound is exclusive; future-dated commits are simply skipped
        // (not a reason to stop, since ordering is by committer time).
        if timestamp >= until {
            continue;
        }

        // Author email match, case-insensitive. `email()` is the UTF-8 view and
        // is `Err` for non-UTF-8 emails — which simply can't equal the queried
        // (UTF-8) string, so treating that as "no match" is correct.
        let matches_author = commit
            .author()
            .email()
            .is_ok_and(|email| email.eq_ignore_ascii_case(author_email));
        if !matches_author {
            continue;
        }

        let short_id = short_id_of(&commit);
        // `summary()` is `Result<Option<&str>, _>` (UTF-8 view); fall back to
        // empty on either a decode error or a missing summary.
        let summary = commit
            .summary()
            .ok()
            .flatten()
            .unwrap_or_default()
            .to_string();

        out.push(StandupCommit {
            repo: repo_name.clone(),
            short_id,
            summary,
            timestamp,
        });
    }

    out
}

/// Abbreviated commit id, preferring libgit2's `short_id` (honours the repo's
/// `core.abbrev`); falls back to the first 7 hex chars of the oid.
#[cfg(feature = "git2-fallback")]
fn short_id_of(commit: &git2::Commit<'_>) -> String {
    commit
        .as_object()
        .short_id()
        .ok()
        .and_then(|buf| buf.as_str().map(str::to_string).ok())
        .unwrap_or_else(|| {
            let hex = commit.id().to_string();
            hex[..hex.len().min(7)].to_string()
        })
}

/// Without libgit2 the history walk is unavailable; degrade to an empty list
/// rather than failing. (Migration target: implement via gix — see ADR-004.)
#[cfg(not(feature = "git2-fallback"))]
pub fn collect_commits(
    path: &Utf8Path,
    author_email: &str,
    since: i64,
    until: i64,
) -> Vec<StandupCommit> {
    let _ = (path, author_email, since, until);
    Vec::new()
}
