//! Build a [`RepoSnapshot`] for one repo.
//!
//! Split per ADR-004: `gix` (pure Rust, primary) reads what the repo *is* —
//! HEAD/branch and the last commit — while `git2` (the `git2-fallback` feature,
//! on by default) fills the gaps gix 0.84 doesn't cover ergonomically yet:
//! worktree status counts, ahead/behind, and stash count. Everything is
//! best-effort: a failure to read one field degrades that field, and a failure
//! to open the repo yields an error snapshot — never a panic.

use camino::Utf8Path;
use cohors_core::{Branch, CommitMeta, RepoRef, RepoSnapshot, Upstream, WorktreeStatus};
use gix::bstr::{BStr, ByteSlice};

/// How many trailing weeks of commit activity to record for the sparkline.
const ACTIVITY_WEEKS: usize = 12;

/// Snapshot a single repo. Always returns a snapshot; read failures land in
/// [`RepoSnapshot::error`]. `now` (Unix seconds) buckets the commit-activity
/// sparkline deterministically. This runs once per repo, in parallel across the
/// fleet (the provider walks repos with `rayon`), so the per-repo git walks fan
/// out across cores rather than running serially.
pub fn snapshot_repo(repo_ref: &RepoRef, now: i64) -> RepoSnapshot {
    let Some(path) = repo_ref.path.clone() else {
        return RepoSnapshot::errored(
            repo_ref.id.clone(),
            "(no path)",
            None,
            "repository reference has no local path",
        );
    };

    let name = path
        .file_name()
        .unwrap_or_else(|| path.as_str())
        .to_string();

    let repo = match gix::open(path.as_std_path()) {
        Ok(repo) => repo,
        Err(err) => {
            return RepoSnapshot::errored(
                repo_ref.id.clone(),
                name,
                Some(path),
                format!("could not open repo: {err}"),
            );
        }
    };

    let last_commit = repo.head_commit().ok().and_then(|c| commit_meta(&c));
    let branch = branch_of(&repo, last_commit.as_ref());
    let extras = git2_extras(&path, now);

    RepoSnapshot {
        id: repo_ref.id.clone(),
        name,
        path: Some(path),
        branch,
        upstream: extras.upstream,
        worktree: extras.worktree,
        stash_count: extras.stash_count,
        stash_latest: extras.stash_latest,
        remote_url: extras.remote_url,
        remote: None,
        last_commit,
        error: None,
        activity: extras.activity,
        // Stamped after the scan by the Scanner (it holds the config `[groups]`).
        groups: Vec::new(),
    }
}

/// Classify HEAD into a [`Branch`]. Reuses the last commit's short id for the
/// detached label when available.
fn branch_of(repo: &gix::Repository, last_commit: Option<&CommitMeta>) -> Branch {
    match repo.head() {
        Ok(head) => match head.kind {
            gix::head::Kind::Symbolic(reference) => {
                Branch::Named(bstr_to_string(reference.name.shorten()))
            }
            gix::head::Kind::Unborn(_) => Branch::Unborn,
            gix::head::Kind::Detached { target, peeled } => {
                let short = last_commit
                    .map(|c| c.short_id.clone())
                    .unwrap_or_else(|| short_hex(peeled.unwrap_or(target)));
                Branch::Detached(short)
            }
        },
        Err(_) => Branch::Unborn,
    }
}

/// Extract commit metadata, tolerating partial decode failures.
fn commit_meta(commit: &gix::Commit<'_>) -> Option<CommitMeta> {
    let short_id = commit.short_id().ok()?.to_string();
    let author = commit
        .author()
        .ok()
        .map(|sig| bstr_to_string(sig.name).trim().to_string())
        .unwrap_or_default();
    let timestamp = commit.time().map(|t| t.seconds).unwrap_or(0);
    let summary = commit
        .message()
        .ok()
        .map(|msg| bstr_to_string(msg.summary().as_ref()))
        .unwrap_or_default();
    Some(CommitMeta {
        short_id,
        author,
        timestamp,
        summary,
    })
}

/// Lossy UTF-8 of a git byte string (branch names, authors, messages).
fn bstr_to_string(bytes: &BStr) -> String {
    bytes.to_str_lossy().into_owned()
}

/// First 7 hex chars of an object id (hex is ASCII, so slicing is safe).
fn short_hex(id: gix::ObjectId) -> String {
    let hex = id.to_string();
    hex[..hex.len().min(7)].to_string()
}

/// The fields filled by the libgit2 fallback.
#[derive(Default)]
struct Extras {
    worktree: WorktreeStatus,
    upstream: Option<Upstream>,
    stash_count: u32,
    stash_latest: Option<i64>,
    remote_url: Option<String>,
    activity: Vec<u8>,
}

#[cfg(feature = "git2-fallback")]
fn git2_extras(path: &Utf8Path, now: i64) -> Extras {
    let mut extras = Extras::default();
    let mut repo = match git2::Repository::open(path.as_std_path()) {
        Ok(repo) => repo,
        Err(err) => {
            tracing::debug!(%path, error = %err, "git2 open failed; status/ahead-behind/stash unavailable");
            return extras;
        }
    };

    extras.worktree = worktree_status(&repo);
    extras.upstream = upstream_info(&repo);
    extras.activity = commit_activity(&repo, now);
    // The `origin` URL is local git data; cohors-github resolves it to a repo.
    extras.remote_url = repo
        .find_remote("origin")
        .ok()
        .and_then(|r| r.url().ok().map(str::to_string));

    // `stash_foreach` needs `&mut`, so do it last after the shared borrows end.
    // Index 0 is the most recent stash; capture its oid to time it afterward.
    let mut stash_count = 0u32;
    let mut newest_stash: Option<git2::Oid> = None;
    let _ = repo.stash_foreach(|index, _, oid| {
        stash_count += 1;
        if index == 0 {
            newest_stash = Some(*oid);
        }
        true
    });
    extras.stash_count = stash_count;
    extras.stash_latest = newest_stash
        .and_then(|oid| repo.find_commit(oid).ok())
        .map(|commit| commit.time().seconds());

    extras
}

#[cfg(feature = "git2-fallback")]
fn worktree_status(repo: &git2::Repository) -> WorktreeStatus {
    use git2::Status;
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);

    let Ok(statuses) = repo.statuses(Some(&mut opts)) else {
        return WorktreeStatus::default();
    };

    let staged_flags = Status::INDEX_NEW
        | Status::INDEX_MODIFIED
        | Status::INDEX_DELETED
        | Status::INDEX_RENAMED
        | Status::INDEX_TYPECHANGE;
    let modified_flags =
        Status::WT_MODIFIED | Status::WT_DELETED | Status::WT_RENAMED | Status::WT_TYPECHANGE;

    let mut status = WorktreeStatus::default();
    for entry in statuses.iter() {
        let s = entry.status();
        // An unmerged path is *only* counted as conflicted — it carries WT_/INDEX_
        // flags too, but folding it into modified/staged would hide the conflict.
        if s.contains(Status::CONFLICTED) {
            status.conflicted += 1;
            continue;
        }
        if s.intersects(staged_flags) {
            status.staged += 1;
        }
        if s.contains(Status::WT_NEW) {
            status.untracked += 1;
        } else if s.intersects(modified_flags) {
            status.modified += 1;
        }
    }
    status
}

#[cfg(feature = "git2-fallback")]
fn upstream_info(repo: &git2::Repository) -> Option<Upstream> {
    let head = repo.head().ok()?;
    if !head.is_branch() {
        return None;
    }
    let local_oid = head.target()?;

    let branch = git2::Branch::wrap(head);
    let upstream = branch.upstream().ok()?; // Err when no upstream is configured
    let upstream_ref = upstream.get();
    let upstream_oid = upstream_ref.target()?;
    let name = upstream_ref.shorthand().unwrap_or_default().to_string();

    let (ahead, behind) = repo.graph_ahead_behind(local_oid, upstream_oid).ok()?;
    Some(Upstream {
        name,
        ahead: ahead as u32,
        behind: behind as u32,
    })
}

/// Recent commit activity as weekly counts (oldest first, ending with the current
/// week) for the sparkline. Walks newest-first by commit time and stops once it
/// leaves the window, so an inactive repo costs ~one lookup; a hard cap bounds
/// huge or merge-tangled histories. Best-effort: any failure yields empty.
#[cfg(feature = "git2-fallback")]
fn commit_activity(repo: &git2::Repository, now: i64) -> Vec<u8> {
    const WEEK: i64 = 7 * 24 * 60 * 60;
    const CAP: usize = 2000;
    let window_start = now - (ACTIVITY_WEEKS as i64) * WEEK;
    let mut buckets = vec![0u8; ACTIVITY_WEEKS];

    let Ok(mut walk) = repo.revwalk() else {
        return buckets;
    };
    if walk.push_head().is_err() {
        return buckets; // unborn HEAD / empty repo
    }
    let _ = walk.set_sorting(git2::Sort::TIME); // newest commit first

    for (i, oid) in walk.enumerate() {
        if i >= CAP {
            break;
        }
        let Ok(oid) = oid else { continue };
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        let ts = commit.time().seconds();
        if ts < window_start {
            break; // time-sorted newest-first ⇒ everything remaining is older
        }
        let weeks_ago = ((now - ts) / WEEK).max(0) as usize;
        if weeks_ago < ACTIVITY_WEEKS {
            let idx = ACTIVITY_WEEKS - 1 - weeks_ago; // oldest first, current week last
            buckets[idx] = buckets[idx].saturating_add(1);
        }
    }
    buckets
}

#[cfg(not(feature = "git2-fallback"))]
fn git2_extras(_path: &Utf8Path, _now: i64) -> Extras {
    // Without libgit2 these fields are unavailable; degrade to empty rather than
    // failing. (Migration target: implement via gix — see ADR-004.)
    Extras::default()
}
