//! Attention scoring — turning a snapshot into "what needs you, and why".
//!
//! This is the triage brain: a status table shows *state*, this layer turns it
//! into *judgment*. Pure and clock-injected (like [`crate::time`]): callers pass
//! `now` so "aging"/"stale" judgments are deterministic and WASM-safe.
//!
//! The TUI uses [`assess`] for the per-row reason and [`fleet_summary`] for the
//! header strip; the sort uses the clock-free [`rank`]. See ADR-014.

use serde::Serialize;

use crate::model::{Branch, RepoSnapshot};

/// How urgently a repo wants attention, least to most. Ordered (declaration
/// order = severity order) so `max`/comparison work directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Ok,
    Info,
    Notice,
    Warn,
    Risk,
}

/// Unpushed work older than this (seconds) is "aging" — at risk of being lost.
const UNPUSHED_AGING_SECS: i64 = 2 * 24 * 60 * 60; // 2 days
/// A stash older than this (seconds) is "stale" — probably forgotten.
const STALE_STASH_SECS: i64 = 7 * 24 * 60 * 60; // 1 week

/// A single reason a repo wants attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttentionReason {
    /// The repo couldn't be read.
    Unreadable,
    /// Local commits not pushed; `aging` if the newest is past the threshold.
    Unpushed { commits: u32, aging: bool },
    /// Both ahead of and behind the upstream.
    Diverged { ahead: u32, behind: u32 },
    /// Behind the upstream (a fast-forward pull catches up).
    Behind { commits: u32 },
    /// On a named branch that has a remote available but no upstream — local-only
    /// work that was never pushed, so it isn't backed up anywhere (data-loss risk).
    Unpublished,
    /// Uncommitted changes in the working tree.
    Uncommitted {
        staged: u32,
        modified: u32,
        untracked: u32,
    },
    /// One or more stashes; `stale` if the newest is past the threshold.
    Stash { count: u32, stale: bool },
    /// Detached HEAD.
    Detached,
}

impl AttentionReason {
    /// How urgent this reason is.
    pub fn severity(&self) -> Severity {
        match self {
            AttentionReason::Unreadable => Severity::Risk,
            AttentionReason::Unpushed { aging: true, .. } => Severity::Risk,
            AttentionReason::Unpushed { aging: false, .. } => Severity::Warn,
            AttentionReason::Diverged { .. } => Severity::Warn,
            // Unbacked branch work outranks mere local dirtiness (Notice): if the
            // disk dies, a never-pushed branch is simply gone.
            AttentionReason::Unpublished => Severity::Warn,
            AttentionReason::Stash { stale: true, .. } => Severity::Warn,
            AttentionReason::Behind { .. } => Severity::Notice,
            AttentionReason::Uncommitted { .. } => Severity::Notice,
            AttentionReason::Stash { stale: false, .. } => Severity::Info,
            AttentionReason::Detached => Severity::Info,
        }
    }

    /// Short, presentation-neutral reason for the row (no color/glyph styling).
    pub fn label(&self) -> String {
        match self {
            AttentionReason::Unreadable => "could not read repo".to_string(),
            AttentionReason::Unpushed { commits, aging } => {
                if *aging {
                    format!("↑{commits} unpushed · aging")
                } else {
                    format!("↑{commits} unpushed")
                }
            }
            AttentionReason::Diverged { ahead, behind } => {
                format!("↑{ahead}↓{behind} diverged")
            }
            AttentionReason::Behind { commits } => format!("↓{commits} behind — pull"),
            AttentionReason::Unpublished => "branch never pushed".to_string(),
            AttentionReason::Uncommitted {
                staged,
                modified,
                untracked,
            } => {
                let n = staged + modified + untracked;
                format!("{n} uncommitted change{}", if n == 1 { "" } else { "s" })
            }
            AttentionReason::Stash { count, stale } => {
                if *stale {
                    format!("{count} stash · stale")
                } else {
                    format!("{count} stash")
                }
            }
            AttentionReason::Detached => "detached HEAD".to_string(),
        }
    }
}

/// The attention assessment of one repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    /// All reasons, most urgent first.
    pub reasons: Vec<AttentionReason>,
    /// The most urgent reason, if any.
    pub primary: Option<AttentionReason>,
    /// Overall severity (the primary reason's, or `Ok`).
    pub severity: Severity,
}

impl Assessment {
    /// Whether this repo wants the user's attention (severity ≥ `Notice`, so
    /// purely-informational states like a fresh stash or detached HEAD don't
    /// count toward the "needs you" total).
    pub fn needs_attention(&self) -> bool {
        self.severity >= Severity::Notice
    }
}

/// Assess one repo against `now` (Unix seconds): collect its reasons (most
/// urgent first) and the primary one.
pub fn assess(repo: &RepoSnapshot, now: i64) -> Assessment {
    let mut reasons = Vec::new();

    if repo.has_error() {
        reasons.push(AttentionReason::Unreadable);
    } else {
        let ahead = repo.ahead();
        let behind = repo.behind();
        if ahead > 0 && behind > 0 {
            reasons.push(AttentionReason::Diverged { ahead, behind });
        } else if ahead > 0 {
            let aging = repo
                .last_commit_time()
                .is_some_and(|t| now - t > UNPUSHED_AGING_SECS);
            reasons.push(AttentionReason::Unpushed {
                commits: ahead,
                aging,
            });
        } else if behind > 0 {
            reasons.push(AttentionReason::Behind { commits: behind });
        } else if is_unpublished(repo) {
            reasons.push(AttentionReason::Unpublished);
        }

        let w = &repo.worktree;
        if w.is_dirty() {
            reasons.push(AttentionReason::Uncommitted {
                staged: w.staged,
                modified: w.modified,
                untracked: w.untracked,
            });
        }
        if repo.stash_count > 0 {
            let stale = repo
                .stash_latest
                .is_some_and(|t| now - t > STALE_STASH_SECS);
            reasons.push(AttentionReason::Stash {
                count: repo.stash_count,
                stale,
            });
        }
        if matches!(repo.branch, Branch::Detached(_)) {
            reasons.push(AttentionReason::Detached);
        }
    }

    // Most urgent first (stable sort, so ties keep insertion order).
    reasons.sort_by_key(|r| std::cmp::Reverse(r.severity()));
    let primary = reasons.first().cloned();
    let severity = primary
        .as_ref()
        .map_or(Severity::Ok, AttentionReason::severity);
    Assessment {
        reasons,
        primary,
        severity,
    }
}

/// A named branch on a repo that *has* a remote but isn't tracking one — work
/// that was never pushed, so it lives only on this disk. (A repo with no remote
/// at all is intentionally local and doesn't count.)
fn is_unpublished(repo: &RepoSnapshot) -> bool {
    matches!(repo.branch, Branch::Named(_)) && repo.upstream.is_none() && repo.remote_url.is_some()
}

/// Clock-free urgency key for sorting (higher = more urgent). Kept separate from
/// [`assess`] so the sort needs no clock: the "aging/stale" nuance is a display
/// concern, while the gross ordering (error > diverged > unpushed/unpublished >
/// behind > dirty > stash > detached > clean) is timeless.
pub fn rank(repo: &RepoSnapshot) -> u32 {
    let tier = if repo.has_error() {
        9
    } else if repo.ahead() > 0 && repo.behind() > 0 {
        8
    } else if repo.ahead() > 0 || is_unpublished(repo) {
        // Unpushed commits and never-pushed branches are both unbacked local work.
        7
    } else if repo.behind() > 0 {
        6
    } else if repo.is_dirty() {
        5
    } else if repo.stash_count > 0 {
        3
    } else if matches!(repo.branch, Branch::Detached(_)) {
        2
    } else {
        0
    };
    // Within a tier, more drift sorts higher (bounded so it can't cross tiers).
    let magnitude = (repo.divergence() + repo.worktree.total() + repo.stash_count).min(999);
    tier * 1000 + magnitude
}

/// Fleet-wide counts for the header summary strip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct FleetSummary {
    pub total: usize,
    pub needs_attention: usize,
    pub unpushed: usize,
    pub unpushed_aging: usize,
    pub behind: usize,
    pub dirty: usize,
    pub stash: usize,
    pub errors: usize,
}

/// Aggregate the fleet into headline counts, evaluated against `now`.
pub fn fleet_summary(repos: &[RepoSnapshot], now: i64) -> FleetSummary {
    let mut s = FleetSummary {
        total: repos.len(),
        ..Default::default()
    };
    for repo in repos {
        if assess(repo, now).needs_attention() {
            s.needs_attention += 1;
        }
        if repo.has_error() {
            s.errors += 1;
        }
        if repo.ahead() > 0 {
            s.unpushed += 1;
            if repo
                .last_commit_time()
                .is_some_and(|t| now - t > UNPUSHED_AGING_SECS)
            {
                s.unpushed_aging += 1;
            }
        }
        if repo.behind() > 0 {
            s.behind += 1;
        }
        if repo.is_dirty() {
            s.dirty += 1;
        }
        if repo.stash_count > 0 {
            s.stash += 1;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Branch, CommitMeta, RepoId, Upstream, WorktreeStatus};

    const NOW: i64 = 1_700_000_000;
    const DAY: i64 = 24 * 60 * 60;

    #[allow(clippy::too_many_arguments)]
    fn repo(
        branch: Branch,
        ahead: u32,
        behind: u32,
        worktree: (u32, u32, u32),
        stash: u32,
        stash_latest: Option<i64>,
        commit_ts: Option<i64>,
        error: Option<&str>,
    ) -> RepoSnapshot {
        RepoSnapshot {
            id: RepoId("r".into()),
            name: "r".into(),
            path: None,
            branch,
            upstream: (ahead > 0 || behind > 0).then(|| Upstream {
                name: "origin/main".into(),
                ahead,
                behind,
            }),
            worktree: WorktreeStatus {
                staged: worktree.0,
                modified: worktree.1,
                untracked: worktree.2,
                conflicted: 0,
            },
            stash_count: stash,
            stash_latest,
            remote_url: None,
            remote: None,
            last_commit: commit_ts.map(|t| CommitMeta {
                short_id: "abc".into(),
                author: "D".into(),
                timestamp: t,
                summary: "m".into(),
            }),
            error: error.map(str::to_string),
            activity: Vec::new(),
            groups: Vec::new(),
        }
    }

    fn main_branch() -> Branch {
        Branch::Named("main".into())
    }

    #[test]
    fn clean_repo_is_ok() {
        let r = repo(main_branch(), 0, 0, (0, 0, 0), 0, None, Some(NOW), None);
        let a = assess(&r, NOW);
        assert_eq!(a.severity, Severity::Ok);
        assert!(a.primary.is_none());
        assert!(!a.needs_attention());
    }

    #[test]
    fn unpublished_branch_outranks_dirtiness_but_local_only_is_fine() {
        // Has a remote, but the current branch never pushed → unbacked work.
        let mut unpublished = repo(main_branch(), 0, 0, (0, 0, 0), 0, None, Some(NOW), None);
        unpublished.remote_url = Some("https://github.com/x/y.git".into()); // upstream stays None
        let a = assess(&unpublished, NOW);
        assert_eq!(a.primary, Some(AttentionReason::Unpublished));
        assert_eq!(a.severity, Severity::Warn);
        assert!(a.needs_attention());

        // A merely-dirty repo (tracking an upstream) is only Notice → lower rank.
        let mut dirty = repo(main_branch(), 0, 0, (0, 3, 0), 0, None, Some(NOW), None);
        dirty.remote_url = Some("https://github.com/x/z.git".into());
        dirty.upstream = Some(Upstream {
            name: "origin/main".into(),
            ahead: 0,
            behind: 0,
        });
        assert_eq!(assess(&dirty, NOW).severity, Severity::Notice);
        assert!(
            rank(&unpublished) > rank(&dirty),
            "unbacked branch work must outrank local dirtiness"
        );

        // A repo with NO remote at all is intentionally local — not flagged.
        let local = repo(main_branch(), 0, 0, (0, 0, 0), 0, None, Some(NOW), None);
        assert_eq!(assess(&local, NOW).severity, Severity::Ok);
    }

    #[test]
    fn recent_unpushed_is_warn_aging_is_risk() {
        let recent = repo(
            main_branch(),
            3,
            0,
            (0, 0, 0),
            0,
            None,
            Some(NOW - DAY),
            None,
        );
        assert_eq!(
            assess(&recent, NOW).primary,
            Some(AttentionReason::Unpushed {
                commits: 3,
                aging: false
            })
        );
        assert_eq!(assess(&recent, NOW).severity, Severity::Warn);

        let old = repo(
            main_branch(),
            3,
            0,
            (0, 0, 0),
            0,
            None,
            Some(NOW - 5 * DAY),
            None,
        );
        assert_eq!(assess(&old, NOW).severity, Severity::Risk);
        assert!(matches!(
            assess(&old, NOW).primary,
            Some(AttentionReason::Unpushed { aging: true, .. })
        ));
    }

    #[test]
    fn behind_and_dirty_are_notice() {
        let behind = repo(main_branch(), 0, 5, (0, 0, 0), 0, None, Some(NOW), None);
        assert_eq!(assess(&behind, NOW).severity, Severity::Notice);
        let dirty = repo(main_branch(), 0, 0, (0, 2, 1), 0, None, Some(NOW), None);
        assert_eq!(assess(&dirty, NOW).severity, Severity::Notice);
    }

    #[test]
    fn diverged_takes_priority_over_dirty() {
        let r = repo(main_branch(), 2, 3, (0, 1, 0), 0, None, Some(NOW), None);
        let a = assess(&r, NOW);
        assert_eq!(
            a.primary,
            Some(AttentionReason::Diverged {
                ahead: 2,
                behind: 3
            })
        );
        // Both reasons are recorded.
        assert_eq!(a.reasons.len(), 2);
    }

    #[test]
    fn stash_freshness_changes_severity() {
        let fresh = repo(
            main_branch(),
            0,
            0,
            (0, 0, 0),
            1,
            Some(NOW - DAY),
            Some(NOW),
            None,
        );
        assert_eq!(assess(&fresh, NOW).severity, Severity::Info);
        let stale = repo(
            main_branch(),
            0,
            0,
            (0, 0, 0),
            1,
            Some(NOW - 10 * DAY),
            Some(NOW),
            None,
        );
        assert_eq!(assess(&stale, NOW).severity, Severity::Warn);
    }

    #[test]
    fn error_is_risk() {
        let r = repo(
            Branch::Unborn,
            0,
            0,
            (0, 0, 0),
            0,
            None,
            None,
            Some("permission denied"),
        );
        let a = assess(&r, NOW);
        assert_eq!(a.severity, Severity::Risk);
        assert_eq!(a.primary, Some(AttentionReason::Unreadable));
    }

    #[test]
    fn rank_orders_by_urgency_then_magnitude() {
        let clean = repo(main_branch(), 0, 0, (0, 0, 0), 0, None, Some(NOW), None);
        let dirty = repo(main_branch(), 0, 0, (0, 1, 0), 0, None, Some(NOW), None);
        let behind = repo(main_branch(), 0, 4, (0, 0, 0), 0, None, Some(NOW), None);
        let unpushed = repo(main_branch(), 2, 0, (0, 0, 0), 0, None, Some(NOW), None);
        let err = repo(Branch::Unborn, 0, 0, (0, 0, 0), 0, None, None, Some("x"));
        assert!(rank(&err) > rank(&unpushed));
        assert!(rank(&unpushed) > rank(&behind));
        assert!(rank(&behind) > rank(&dirty));
        assert!(rank(&dirty) > rank(&clean));
        // Magnitude breaks ties within a tier.
        let dirty_more = repo(main_branch(), 0, 0, (1, 3, 2), 0, None, Some(NOW), None);
        assert!(rank(&dirty_more) > rank(&dirty));
    }

    #[test]
    fn fleet_summary_counts_categories() {
        let repos = vec![
            repo(
                main_branch(),
                3,
                0,
                (0, 0, 0),
                0,
                None,
                Some(NOW - 5 * DAY),
                None,
            ), // unpushed aging
            repo(main_branch(), 0, 2, (0, 0, 0), 0, None, Some(NOW), None), // behind
            repo(main_branch(), 0, 0, (0, 1, 0), 0, None, Some(NOW), None), // dirty
            repo(
                main_branch(),
                0,
                0,
                (0, 0, 0),
                1,
                Some(NOW - DAY),
                Some(NOW),
                None,
            ), // fresh stash
            repo(Branch::Unborn, 0, 0, (0, 0, 0), 0, None, None, Some("x")), // error
            repo(main_branch(), 0, 0, (0, 0, 0), 0, None, Some(NOW), None), // clean
        ];
        let s = fleet_summary(&repos, NOW);
        assert_eq!(s.total, 6);
        assert_eq!(s.unpushed, 1);
        assert_eq!(s.unpushed_aging, 1);
        assert_eq!(s.behind, 1);
        assert_eq!(s.dirty, 1);
        assert_eq!(s.stash, 1);
        assert_eq!(s.errors, 1);
        // needs_attention excludes the fresh stash (Info) and the clean repo.
        assert_eq!(s.needs_attention, 4);
    }
}
