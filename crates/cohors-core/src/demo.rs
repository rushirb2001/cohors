//! A privacy-safe, deterministic sample fleet for the `cohors demo` command and
//! the future web playground.
//!
//! This lives in the pure core (no I/O, no clock) so every front-end can show
//! the same instant, zero-config dashboard: the caller injects `now` and gets a
//! varied set of [`RepoSnapshot`]s exercising every state the UI renders —
//! ahead/behind, dirty, stashed, CI pass/fail/pending, open PRs, a detached
//! HEAD, an off-remote repo, and one unreadable repo.

use camino::Utf8PathBuf;

use crate::detail::{ChangedFile, RepoDetail};
use crate::model::{
    Branch, CiStatus, CommitMeta, RemoteInfo, RepoId, RepoSnapshot, Upstream, WorktreeStatus,
};
use crate::standup::StandupCommit;

const HOUR: i64 = 3_600;
const DAY: i64 = 24 * HOUR;

/// One row in the demo fleet, kept terse so the table below reads like data.
struct Spec {
    name: &'static str,
    branch: Branch,
    /// `(ahead, behind)` vs upstream; `None` means no upstream is configured.
    upstream: Option<(u32, u32)>,
    /// `(staged, modified, untracked)` working-tree counts.
    worktree: (u32, u32, u32),
    stash: u32,
    /// `(open_prs, ci)` when the repo is on a remote; `None` when it isn't.
    remote: Option<(u32, CiStatus)>,
    /// Age of the last commit, in seconds before `now`.
    age: i64,
    summary: &'static str,
    /// Set for the one repo that fails to read, to exercise the error row.
    error: Option<&'static str>,
}

/// Build the demo fleet, with commit ages relative to `now` (Unix seconds).
pub fn fleet(now: i64) -> Vec<RepoSnapshot> {
    // Import only the variants we use; a glob would pull in `CiStatus::None`,
    // which shadows `Option::None` and breaks every `error: None` below.
    use CiStatus::{Failing, Passing, Pending};
    let specs = [
        Spec {
            name: "payments",
            branch: Branch::Named("main".into()),
            upstream: Some((2, 0)),
            worktree: (1, 2, 0),
            stash: 0,
            remote: Some((2, Passing)),
            age: 2 * HOUR,
            summary: "fix: retry charge on 5xx from processor",
            error: None,
        },
        Spec {
            name: "web-app",
            branch: Branch::Named("feat/checkout".into()),
            upstream: Some((0, 5)),
            worktree: (0, 6, 1),
            stash: 0,
            remote: Some((1, Pending)),
            age: 20 * 60,
            summary: "wip: cart drawer animation",
            error: None,
        },
        Spec {
            name: "auth-service",
            branch: Branch::Named("main".into()),
            upstream: Some((0, 0)),
            worktree: (0, 0, 0),
            stash: 0,
            remote: Some((0, Passing)),
            age: 3 * DAY,
            summary: "chore: bump jsonwebtoken to 9.3",
            error: None,
        },
        Spec {
            name: "design-system",
            branch: Branch::Named("main".into()),
            upstream: Some((1, 0)),
            worktree: (0, 0, 0),
            stash: 1,
            remote: Some((1, Passing)),
            age: 6 * HOUR,
            summary: "feat: dark-mode tokens",
            error: None,
        },
        Spec {
            name: "data-pipeline",
            branch: Branch::Named("main".into()),
            upstream: Some((0, 3)),
            worktree: (0, 0, 0),
            stash: 0,
            remote: Some((0, Failing)),
            age: 5 * HOUR,
            summary: "fix: backfill job off-by-one window",
            error: None,
        },
        Spec {
            name: "mobile-app",
            branch: Branch::Named("release/2.0".into()),
            upstream: Some((4, 1)),
            worktree: (3, 1, 0),
            stash: 0,
            remote: Some((3, Pending)),
            age: 9 * HOUR,
            summary: "feat: biometric unlock",
            error: None,
        },
        Spec {
            name: "infra",
            branch: Branch::Detached("a1b2c3d".into()),
            upstream: None,
            worktree: (0, 2, 2),
            stash: 0,
            remote: None,
            age: 8 * DAY,
            summary: "build: pin terraform provider versions",
            error: None,
        },
        Spec {
            name: "analytics",
            branch: Branch::Named("main".into()),
            upstream: Some((1, 0)),
            worktree: (0, 0, 0),
            stash: 2,
            remote: Some((0, Passing)),
            age: 4 * DAY,
            summary: "perf: precompute funnel rollups",
            error: None,
        },
        Spec {
            name: "cli-tools",
            branch: Branch::Named("main".into()),
            upstream: Some((0, 0)),
            worktree: (0, 0, 1),
            stash: 0,
            remote: Some((0, Passing)),
            age: 12 * HOUR,
            summary: "docs: add shell-completion install notes",
            error: None,
        },
        Spec {
            name: "marketing-site",
            branch: Branch::Named("main".into()),
            upstream: Some((0, 0)),
            worktree: (0, 0, 0),
            stash: 0,
            remote: None,
            age: 30 * DAY,
            summary: "content: Q2 launch landing page",
            error: None,
        },
        Spec {
            name: "experiments",
            branch: Branch::Detached("9f8e7d6".into()),
            upstream: None,
            worktree: (0, 4, 3),
            stash: 0,
            remote: None,
            age: 400 * DAY,
            summary: "spike: try wgpu for the renderer",
            error: None,
        },
        Spec {
            name: "legacy-billing",
            branch: Branch::Unborn,
            upstream: None,
            worktree: (0, 0, 0),
            stash: 0,
            remote: None,
            age: 0,
            summary: "",
            error: Some("could not read .git (permission denied)"),
        },
    ];

    specs.into_iter().map(|s| build(s, now)).collect()
}

fn build(s: Spec, now: i64) -> RepoSnapshot {
    let on_remote = s.remote.is_some();
    let remote = s.remote.map(|(open_prs, ci)| RemoteInfo {
        host: "github.com".into(),
        owner: "acme".into(),
        repo: s.name.into(),
        default_branch: "main".into(),
        open_prs,
        prs_awaiting_review: 0,
        ci,
    });
    RepoSnapshot {
        id: RepoId(format!("demo/{}", s.name)),
        name: s.name.to_string(),
        path: Some(Utf8PathBuf::from(format!("~/code/{}", s.name))),
        branch: s.branch,
        upstream: s.upstream.map(|(ahead, behind)| Upstream {
            name: "origin/main".into(),
            ahead,
            behind,
        }),
        worktree: WorktreeStatus {
            staged: s.worktree.0,
            modified: s.worktree.1,
            untracked: s.worktree.2,
        },
        stash_count: s.stash,
        stash_latest: None,
        remote_url: on_remote.then(|| format!("git@github.com:acme/{}.git", s.name)),
        remote,
        last_commit: (s.error.is_none()).then(|| CommitMeta {
            short_id: "abc1234".into(),
            author: "you".into(),
            timestamp: now - s.age,
            summary: s.summary.to_string(),
        }),
        error: s.error.map(|e| e.to_string()),
    }
}

/// Demo commits for the standup view: a week of work across a few repos.
pub fn standup(now: i64) -> Vec<StandupCommit> {
    let rows: [(&str, &str, i64); 12] = [
        (
            "payments",
            "fix: retry charge on 5xx from processor",
            2 * HOUR,
        ),
        ("payments", "test: cover partial-refund path", 5 * HOUR),
        ("payments", "refactor: extract processor client", 26 * HOUR),
        ("web-app", "wip: cart drawer animation", 20 * 60),
        ("web-app", "feat: empty-cart illustration", 28 * HOUR),
        ("web-app", "fix: checkout button focus ring", 2 * DAY),
        ("design-system", "feat: dark-mode tokens", 6 * HOUR),
        ("design-system", "docs: usage for <Banner/>", 3 * DAY),
        ("mobile-app", "feat: biometric unlock", 9 * HOUR),
        ("mobile-app", "chore: bump kotlin to 2.0", 4 * DAY),
        ("analytics", "perf: precompute funnel rollups", 4 * DAY),
        (
            "data-pipeline",
            "fix: backfill job off-by-one window",
            5 * HOUR,
        ),
    ];
    rows.into_iter()
        .enumerate()
        .map(|(i, (repo, summary, age))| StandupCommit {
            repo: repo.to_string(),
            short_id: format!("c{i:06x}"),
            summary: summary.to_string(),
            timestamp: now - age,
        })
        .collect()
}

/// A sample [`RepoDetail`] for the `cohors demo` drill-in pane.
pub fn detail(now: i64) -> RepoDetail {
    let commit = |short_id: &str, age: i64, summary: &str| CommitMeta {
        short_id: short_id.to_string(),
        author: "you".to_string(),
        timestamp: now - age,
        summary: summary.to_string(),
    };
    RepoDetail {
        current_branch: Some("main".to_string()),
        recent_commits: vec![
            commit(
                "a1b2c3d",
                2 * HOUR,
                "fix: retry charge on 5xx from processor",
            ),
            commit("b2c3d4e", 5 * HOUR, "test: cover partial-refund path"),
            commit("c3d4e5f", 26 * HOUR, "refactor: extract processor client"),
            commit("d4e5f60", 3 * DAY, "feat: idempotency keys on charge"),
            commit("e5f6071", 4 * DAY, "chore: bump stripe sdk to 12.1"),
        ],
        changed_files: vec![
            ChangedFile {
                status: " M".to_string(),
                path: "src/charge.rs".to_string(),
            },
            ChangedFile {
                status: " M".to_string(),
                path: "src/refund.rs".to_string(),
            },
            ChangedFile {
                status: "??".to_string(),
                path: "notes.md".to_string(),
            },
        ],
        branches: vec![
            "main".to_string(),
            "feat/idempotency".to_string(),
            "spike/webhooks".to_string(),
        ],
        stashes: vec!["WIP on main: dashboard tweaks".to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fleet_exercises_every_state() {
        let now = 1_700_000_000;
        let f = fleet(now);
        assert!(f.len() >= 10);
        // At least one of each interesting state the UI renders.
        assert!(f.iter().any(|r| r.error.is_some()));
        assert!(f.iter().any(|r| matches!(r.branch, Branch::Detached(_))));
        assert!(f.iter().any(|r| r.remote.is_none()));
        assert!(f.iter().any(|r| r.stash_count > 0));
        assert!(
            f.iter()
                .any(|r| r.upstream.as_ref().is_some_and(|u| u.ahead > 0))
        );
        assert!(
            f.iter()
                .any(|r| r.upstream.as_ref().is_some_and(|u| u.behind > 0))
        );
        assert!(
            f.iter()
                .any(|r| r.remote.as_ref().is_some_and(|m| m.ci == CiStatus::Failing))
        );
        // Commit ages are relative to the injected now (no clock in core).
        let p = f.iter().find(|r| r.name == "payments").unwrap();
        assert_eq!(p.last_commit.as_ref().unwrap().timestamp, now - 2 * HOUR);
    }

    #[test]
    fn standup_has_multiple_repos() {
        let commits = standup(1_700_000_000);
        let repos: std::collections::HashSet<_> = commits.iter().map(|c| &c.repo).collect();
        assert!(repos.len() >= 4);
    }
}
