//! Build a [`RepoDetail`] for one repo: the "drill-in" view the TUI's detail
//! pane renders (current branch, recent commits, working-tree changes, local
//! branches, stashes).
//!
//! Like the snapshot and standup paths, this runs on `git2` (libgit2) behind the
//! `git2-fallback` feature (on by default); without it every section degrades to
//! empty rather than failing to compile. See ADR-004. Everything is best-effort:
//! any section that can't be read is left empty and we continue — never a panic.

use camino::Utf8Path;
use cohors_core::{ChangedFile, CommitMeta, RepoDetail};

/// Number of recent commits to surface in the detail view, newest first.
const RECENT_COMMITS_LIMIT: usize = 15;

/// Read the detail view for the repo at `path`. Always returns a [`RepoDetail`];
/// any unreadable section is simply left empty.
#[cfg(feature = "git2-fallback")]
pub fn repo_detail(path: &Utf8Path) -> RepoDetail {
    let mut repo = match git2::Repository::open(path.as_std_path()) {
        Ok(repo) => repo,
        Err(err) => {
            tracing::debug!(%path, error = %err, "git2 open failed; empty repo detail");
            return RepoDetail::default();
        }
    };

    // Order matters for the borrow checker: the stash list needs `&mut repo`, so
    // gather everything that only needs `&repo` first, then take the stashes.
    let current_branch = current_branch(&repo);
    let recent_commits = recent_commits(&repo);
    let changed_files = changed_files(&repo);
    let branches = local_branches(&repo, current_branch.as_deref());
    let stashes = stashes(&mut repo);

    RepoDetail {
        current_branch,
        recent_commits,
        changed_files,
        branches,
        stashes,
    }
}

/// The current local branch name, or `None` for a detached or unborn HEAD.
#[cfg(feature = "git2-fallback")]
fn current_branch(repo: &git2::Repository) -> Option<String> {
    let head = repo.head().ok()?; // Err on an unborn branch → no current branch.
    if !head.is_branch() {
        return None; // Detached HEAD points at a commit, not a branch.
    }
    head.shorthand().map(str::to_string).ok()
}

/// The most recent commits reachable from HEAD, newest first, capped at
/// [`RECENT_COMMITS_LIMIT`]. An unborn/headless repo yields an empty list.
#[cfg(feature = "git2-fallback")]
fn recent_commits(repo: &git2::Repository) -> Vec<CommitMeta> {
    let mut revwalk = match repo.revwalk() {
        Ok(rw) => rw,
        Err(_) => return Vec::new(),
    };
    revwalk.set_sorting(git2::Sort::TIME).ok();
    if revwalk.push_head().is_err() {
        // No HEAD (unborn branch) → nothing to list.
        return Vec::new();
    }

    let mut out = Vec::new();
    for oid in revwalk {
        if out.len() >= RECENT_COMMITS_LIMIT {
            break;
        }
        let Ok(oid) = oid else { continue };
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };

        let short_id = short_id_of(&commit);
        // Author display name; `Some` only when the bytes are valid UTF-8.
        let author = commit.author().name().unwrap_or_default().to_string();
        let timestamp = commit.time().seconds();
        // `summary()` is `Result<Option<&str>, _>` — the first line of the
        // message; empty on a decode error or a missing summary.
        let summary = commit
            .summary()
            .ok()
            .flatten()
            .unwrap_or_default()
            .to_string();

        out.push(CommitMeta {
            short_id,
            author,
            timestamp,
            summary,
        });
    }
    out
}

/// Working-tree changes (staged + modified + untracked), each tagged with a
/// short two-char porcelain-style status. Best-effort: an unreadable status
/// yields an empty list.
#[cfg(feature = "git2-fallback")]
fn changed_files(repo: &git2::Repository) -> Vec<ChangedFile> {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);

    let Ok(statuses) = repo.statuses(Some(&mut opts)) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in statuses.iter() {
        let status = porcelain_status(entry.status());
        // `path()` is the UTF-8 view of the entry's path (relative to the repo
        // root); `None` only for non-UTF-8 paths, which we skip rather than
        // mangle.
        if let Ok(path) = entry.path() {
            out.push(ChangedFile {
                status,
                path: path.to_string(),
            });
        }
    }
    out
}

/// Map a libgit2 status flag set to git's two-character porcelain code: the
/// first char is the index (staged) state, the second the worktree state. We
/// cover the common cases the detail pane shows; anything unrecognised falls
/// back to spaces. Untracked files are `"??"`, matching `git status --porcelain`.
#[cfg(feature = "git2-fallback")]
fn porcelain_status(status: git2::Status) -> String {
    use git2::Status;

    if status.contains(Status::WT_NEW) && !status.intersects(INDEX_FLAGS) {
        return "??".to_string();
    }

    let index = if status.contains(Status::INDEX_NEW) {
        'A'
    } else if status.contains(Status::INDEX_MODIFIED) {
        'M'
    } else if status.contains(Status::INDEX_DELETED) {
        'D'
    } else if status.contains(Status::INDEX_RENAMED) {
        'R'
    } else if status.contains(Status::INDEX_TYPECHANGE) {
        'T'
    } else {
        ' '
    };

    let worktree = if status.contains(Status::WT_NEW) {
        'A'
    } else if status.contains(Status::WT_MODIFIED) {
        'M'
    } else if status.contains(Status::WT_DELETED) {
        'D'
    } else if status.contains(Status::WT_RENAMED) {
        'R'
    } else if status.contains(Status::WT_TYPECHANGE) {
        'T'
    } else {
        ' '
    };

    format!("{index}{worktree}")
}

/// The set of index-side (staged) status flags, used to tell a brand-new
/// untracked file (`"??"`) apart from a freshly-added-but-also-modified one.
#[cfg(feature = "git2-fallback")]
const INDEX_FLAGS: git2::Status = git2::Status::INDEX_NEW
    .union(git2::Status::INDEX_MODIFIED)
    .union(git2::Status::INDEX_DELETED)
    .union(git2::Status::INDEX_RENAMED)
    .union(git2::Status::INDEX_TYPECHANGE);

/// Local branch names, with the current branch (if any) listed first. A repo
/// with no branches yields an empty list.
#[cfg(feature = "git2-fallback")]
fn local_branches(repo: &git2::Repository, current: Option<&str>) -> Vec<String> {
    let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) else {
        return Vec::new();
    };

    let mut names: Vec<String> = branches
        .filter_map(|b| b.ok())
        .filter_map(|(branch, _)| branch.name().ok().flatten().map(str::to_string))
        .collect();

    // Hoist the current branch to the front so the detail pane can show it first.
    if let Some(current) = current
        && let Some(pos) = names.iter().position(|n| n == current)
    {
        names.swap(0, pos);
    }
    names
}

/// Stash messages, newest first. `stash_foreach` requires `&mut`, and index 0 is
/// the most recent entry, so iteration order already matches "newest first".
#[cfg(feature = "git2-fallback")]
fn stashes(repo: &mut git2::Repository) -> Vec<String> {
    let mut out = Vec::new();
    let _ = repo.stash_foreach(|_index, message, _oid| {
        out.push(message.to_string());
        true
    });
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

/// Without libgit2 the detail read is unavailable; degrade to an empty
/// [`RepoDetail`] rather than failing. (Migration target: implement via gix —
/// see ADR-004.)
#[cfg(not(feature = "git2-fallback"))]
pub fn repo_detail(path: &Utf8Path) -> RepoDetail {
    let _ = path;
    RepoDetail::default()
}
