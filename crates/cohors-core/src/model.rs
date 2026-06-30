//! Data models shared by every front-end.
//!
//! These are plain, serializable values with no behavior that touches the
//! outside world. A [`RepoSnapshot`] is the unit the dashboard renders, sorts,
//! and filters; the git/GitHub adapters produce them and `cohors-core` arranges
//! them.

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

/// Stable identity for a repo.
///
/// For local repos this is the canonical filesystem path; for remote repos
/// (v0.2) it will be the remote URL. Used as the cache key and to keep the
/// selection pinned to the same repo across refreshes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RepoId(pub String);

impl RepoId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RepoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A lightweight handle to a repo, produced by [`crate::RepoProvider::list`]
/// before any expensive status work. [`crate::RepoProvider::snapshot`] turns one
/// into a full [`RepoSnapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    pub id: RepoId,
    /// Local filesystem path, if this repo lives on disk. `None` for
    /// purely-remote repos (the web front-end).
    pub path: Option<Utf8PathBuf>,
}

/// Which branch (if any) `HEAD` points at.
//
// Serialized as an adjacently-tagged object (`{"kind":"named","value":"main"}`)
// so the `cohors scan` JSON is uniform and easy to consume from scripts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum Branch {
    /// A normal named branch, e.g. `main`.
    Named(String),
    /// Detached `HEAD` at the given short commit id, e.g. `a1b2c3d`.
    Detached(String),
    /// A brand-new repo with no commits yet.
    Unborn,
}

impl Branch {
    /// Human-facing label for the branch column.
    ///
    /// Presentation-neutral (no color or glyphs) so the TUI and web render it
    /// identically: `main`, `@a1b2c3d (detached)`, or `(no commits)`.
    pub fn label(&self) -> String {
        match self {
            Branch::Named(name) => name.clone(),
            Branch::Detached(short) => format!("@{short} (detached)"),
            Branch::Unborn => "(no commits)".to_string(),
        }
    }
}

/// Upstream tracking branch and how far the local branch has drifted from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upstream {
    /// e.g. `origin/main`.
    pub name: String,
    /// Local commits not yet on the upstream.
    pub ahead: u32,
    /// Upstream commits not yet pulled.
    pub behind: u32,
}

/// Working-tree status counts. `is_dirty` is *derived*, never stored, so it can
/// never drift from the counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeStatus {
    /// Staged changes (in the index).
    pub staged: u32,
    /// Unstaged changes to tracked files.
    pub modified: u32,
    /// Untracked files.
    pub untracked: u32,
    /// Unmerged (conflicted) files. Surfaced separately from `modified` because
    /// "you have conflicts" is a distinct, more urgent state than a dirty tree —
    /// it usually means a merge/rebase/cherry-pick is mid-flight. `#[serde(default)]`
    /// so snapshots cached before this field existed still deserialize as 0.
    #[serde(default)]
    pub conflicted: u32,
}

impl WorktreeStatus {
    /// Any uncommitted change at all?
    pub fn is_dirty(&self) -> bool {
        self.staged > 0 || self.modified > 0 || self.untracked > 0 || self.conflicted > 0
    }

    /// Total number of changed entries.
    pub fn total(&self) -> u32 {
        self.staged + self.modified + self.untracked + self.conflicted
    }

    /// Whether any file is in an unmerged/conflicted state.
    pub fn has_conflicts(&self) -> bool {
        self.conflicted > 0
    }
}

/// Metadata about the most recent commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitMeta {
    /// Short commit hash, e.g. `a1b2c3d`.
    pub short_id: String,
    /// Author name.
    pub author: String,
    /// Commit time in seconds since the Unix epoch (UTC). Kept as a raw
    /// timestamp so sorting needs no clock and [`crate::time::relative`] can
    /// render it against an injected "now".
    pub timestamp: i64,
    /// First line of the commit message.
    pub summary: String,
}

/// CI / checks status for a repo's default branch (v0.2). `None` means no checks
/// configured (or not fetched yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiStatus {
    Passing,
    Failing,
    Pending,
    None,
}

/// A multi-step git operation that is paused mid-flight in the working tree — the
/// repo is in a special state until you finish or abort it. Surfaced because it's
/// urgent context (often paired with conflicts) that a plain dirty/ahead view hides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoOperation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Bisect,
    /// `git am` — applying a mailbox of patches.
    Mailbox,
}

impl RepoOperation {
    /// A short, stable label for status lines (e.g. "rebase in progress").
    pub fn label(self) -> &'static str {
        match self {
            RepoOperation::Merge => "merge",
            RepoOperation::Rebase => "rebase",
            RepoOperation::CherryPick => "cherry-pick",
            RepoOperation::Revert => "revert",
            RepoOperation::Bisect => "bisect",
            RepoOperation::Mailbox => "am",
        }
    }
}

/// Remote (GitHub) info for a repo, populated by `cohors-github` (v0.2). `None`
/// on a snapshot means "not a GitHub repo, or not fetched yet".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteInfo {
    /// e.g. `github.com`.
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub default_branch: String,
    /// Open pull requests on the repo.
    pub open_prs: u32,
    /// PRs requesting the current user's review.
    pub prs_awaiting_review: u32,
    /// CI/checks status of the default branch's latest commit.
    pub ci: CiStatus,
}

/// An open pull request, for the detail pane (populated by `cohors-github`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    pub number: u32,
    pub title: String,
    pub author: String,
    pub draft: bool,
    pub branch: String,
    pub url: String,
}

/// A repository contributor, by commit count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contributor {
    pub login: String,
    pub contributions: u32,
}

/// Remote (GitHub) drill-in detail for one repo: open PRs + top contributors.
/// Fetched on demand when the detail pane opens.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDetail {
    pub prs: Vec<PullRequest>,
    pub contributors: Vec<Contributor>,
    /// Open issues (excluding PRs).
    pub open_issues: u32,
    /// Latest release tag, if the repo has one.
    pub latest_release: Option<String>,
}

/// A full point-in-time view of one repo — the unit the dashboard renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoSnapshot {
    pub id: RepoId,
    /// Alias (if configured) or the repo directory name.
    pub name: String,
    /// Canonical absolute path. `None` for purely-remote repos.
    pub path: Option<Utf8PathBuf>,
    pub branch: Branch,
    /// Tracking branch + ahead/behind, if the current branch has an upstream.
    pub upstream: Option<Upstream>,
    #[serde(default)]
    pub worktree: WorktreeStatus,
    #[serde(default)]
    pub stash_count: u32,
    /// Commit time (Unix seconds) of the newest stash, if any — lets the
    /// attention layer flag a *stale* stash you've forgotten about.
    #[serde(default)]
    pub stash_latest: Option<i64>,
    pub last_commit: Option<CommitMeta>,
    /// The `origin` remote URL (local git data), if any — feeds GitHub
    /// resolution in `cohors-github`.
    #[serde(default)]
    pub remote_url: Option<String>,
    /// A multi-step git operation paused in the working tree (merge/rebase/…), if
    /// any. `None` is the normal "clean state". Urgent context the dashboard flags.
    #[serde(default)]
    pub operation: Option<RepoOperation>,
    /// The repo's default branch (e.g. `main`), detected locally from
    /// `refs/remotes/origin/HEAD`. `None` when there's no origin or it isn't set.
    /// Lets a surface flag "you're on a non-default branch" without a network call.
    #[serde(default)]
    pub default_branch: Option<String>,
    /// When the repo last fetched from its remote (Unix seconds), from the mtime
    /// of `FETCH_HEAD`. `None` if it has never fetched. Feeds a "stale remote"
    /// (haven't fetched in a while) signal without touching the network.
    #[serde(default)]
    pub last_fetch: Option<i64>,
    /// GitHub-derived info (v0.2), filled asynchronously after the local scan;
    /// `None` until fetched (or for non-GitHub repos).
    #[serde(default)]
    pub remote: Option<RemoteInfo>,
    /// Set when the repo couldn't be read; the other fields then hold
    /// best-effort defaults. One bad repo must never crash the dashboard, so
    /// adapters record the reason here instead of failing the whole scan.
    #[serde(default)]
    pub error: Option<String>,
    /// Recent commit activity: one count per week, oldest first, ending with the
    /// current week — for a sparkline. Empty when unknown (old caches, repos read
    /// without the git2 walk). Counts are capped at `u8::MAX`.
    #[serde(default)]
    pub activity: Vec<u8>,
    /// Config-defined groups this repo belongs to (e.g. `["payments"]`), stamped
    /// after the scan from the `[groups]` table. Empty when ungrouped. Lets a
    /// surface reason about a repo *cluster* as a fact rather than guessing from
    /// names.
    #[serde(default)]
    pub groups: Vec<String>,
}

impl RepoSnapshot {
    /// Convenience constructor for a repo that couldn't be read. The branch is
    /// a placeholder (`Unborn`) since the real state is unknown — the TUI shows
    /// the [`error`](Self::error) instead of the normal columns.
    pub fn errored(
        id: RepoId,
        name: impl Into<String>,
        path: Option<Utf8PathBuf>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            path,
            branch: Branch::Unborn,
            upstream: None,
            worktree: WorktreeStatus::default(),
            stash_count: 0,
            stash_latest: None,
            last_commit: None,
            remote_url: None,
            operation: None,
            default_branch: None,
            last_fetch: None,
            remote: None,
            error: Some(error.into()),
            activity: Vec::new(),
            groups: Vec::new(),
        }
    }

    /// Whether the working tree has uncommitted changes.
    pub fn is_dirty(&self) -> bool {
        self.worktree.is_dirty()
    }

    /// Commits ahead of upstream (0 if no upstream).
    pub fn ahead(&self) -> u32 {
        self.upstream.as_ref().map_or(0, |u| u.ahead)
    }

    /// Commits behind upstream (0 if no upstream).
    pub fn behind(&self) -> u32 {
        self.upstream.as_ref().map_or(0, |u| u.behind)
    }

    /// Total divergence from upstream — the key for the ahead/behind sort.
    pub fn divergence(&self) -> u32 {
        self.ahead() + self.behind()
    }

    /// Recency key for sorting: the last commit's timestamp, or `None` if
    /// unknown (empty or unreadable repo).
    pub fn last_commit_time(&self) -> Option<i64> {
        self.last_commit.as_ref().map(|c| c.timestamp)
    }

    /// Does this repo want the user's attention?
    ///
    /// Matches the dirty-only toggle exactly (MVP-SPEC §5): a dirty working
    /// tree, commits to push, or a stash. Also the primary key for the
    /// dirty-first sort.
    pub fn needs_attention(&self) -> bool {
        self.is_dirty() || self.ahead() > 0 || self.stash_count > 0
    }

    /// Whether this snapshot represents a repo that failed to read.
    pub fn has_error(&self) -> bool {
        self.error.is_some()
    }

    /// Whether HEAD is on the repo's known default branch. `None` when the current
    /// branch or the default branch is unknown (detached, unborn, or no origin).
    pub fn on_default_branch(&self) -> Option<bool> {
        match (&self.branch, &self.default_branch) {
            (Branch::Named(b), Some(default)) => Some(b == default),
            _ => None,
        }
    }
}

/// Build a minimal snapshot for tests in this crate. Kept here (compiled only
/// under `cfg(test)`) so the sort/fuzzy/view tests can share it.
#[cfg(test)]
pub(crate) fn sample(
    id: &str,
    name: &str,
    dirty: bool,
    ahead: u32,
    stash: u32,
    commit_ts: Option<i64>,
) -> RepoSnapshot {
    RepoSnapshot {
        id: RepoId(id.to_string()),
        name: name.to_string(),
        path: Some(Utf8PathBuf::from(format!("/repos/{id}"))),
        branch: Branch::Named("main".to_string()),
        upstream: (ahead > 0).then(|| Upstream {
            name: "origin/main".to_string(),
            ahead,
            behind: 0,
        }),
        worktree: if dirty {
            WorktreeStatus {
                staged: 0,
                modified: 1,
                untracked: 0,
                conflicted: 0,
            }
        } else {
            WorktreeStatus::default()
        },
        stash_count: stash,
        stash_latest: None,
        remote_url: None,
        operation: None,
        default_branch: Some("main".to_string()),
        last_fetch: None,
        remote: None,
        last_commit: commit_ts.map(|t| CommitMeta {
            short_id: "abc1234".to_string(),
            author: "Dev".to_string(),
            timestamp: t,
            summary: "msg".to_string(),
        }),
        error: None,
        activity: Vec::new(),
        groups: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_is_dirty_only_when_counts_present() {
        assert!(!WorktreeStatus::default().is_dirty());
        assert!(
            WorktreeStatus {
                modified: 1,
                ..Default::default()
            }
            .is_dirty()
        );
        assert_eq!(
            WorktreeStatus {
                staged: 2,
                modified: 3,
                untracked: 1,
                conflicted: 2,
            }
            .total(),
            8
        );
        assert!(
            WorktreeStatus {
                conflicted: 1,
                ..Default::default()
            }
            .has_conflicts()
        );
    }

    #[test]
    fn needs_attention_covers_dirty_ahead_and_stash() {
        assert!(!sample("a", "a", false, 0, 0, Some(1)).needs_attention());
        assert!(sample("a", "a", true, 0, 0, Some(1)).needs_attention()); // dirty
        assert!(sample("a", "a", false, 2, 0, Some(1)).needs_attention()); // ahead
        assert!(sample("a", "a", false, 0, 1, Some(1)).needs_attention()); // stash
    }

    #[test]
    fn ahead_behind_default_to_zero_without_upstream() {
        let s = sample("a", "a", false, 0, 0, Some(1));
        assert_eq!(s.ahead(), 0);
        assert_eq!(s.behind(), 0);
        assert_eq!(s.divergence(), 0);
    }

    #[test]
    fn branch_labels_render_each_variant() {
        assert_eq!(Branch::Named("main".into()).label(), "main");
        assert_eq!(
            Branch::Detached("a1b2c3d".into()).label(),
            "@a1b2c3d (detached)"
        );
        assert_eq!(Branch::Unborn.label(), "(no commits)");
    }

    #[test]
    fn errored_snapshot_carries_reason() {
        let s = RepoSnapshot::errored(RepoId("x".into()), "x", None, "permission denied");
        assert!(s.has_error());
        assert_eq!(s.error.as_deref(), Some("permission denied"));
        assert!(!s.needs_attention());
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let s = sample("payments", "payments", true, 2, 1, Some(1_700_000_000));
        let json = serde_json::to_string(&s).expect("serialize");
        let back: RepoSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(s, back);
    }
}
