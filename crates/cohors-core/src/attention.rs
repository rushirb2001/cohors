//! Attention scoring — turning a snapshot into "what needs you, and why".
//!
//! This is the triage brain: a status table shows *state*, this layer turns it
//! into *judgment*. Pure and clock-injected (like [`crate::time`]): callers pass
//! `now` so "aging"/"stale" judgments are deterministic and WASM-safe.
//!
//! The TUI uses [`assess`] for the per-row reason and [`fleet_summary`] for the
//! header strip; the sort uses the clock-free [`rank`]. See ADR-014.

use serde::Serialize;

use crate::model::{Branch, RepoOperation, RepoSnapshot};

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
/// A remote not fetched for this long (seconds) is "stale" — ahead/behind may lie.
const STALE_FETCH_SECS: i64 = 14 * 24 * 60 * 60; // 2 weeks

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
    /// Unmerged (conflicted) files — a merge/rebase stopped mid-flight and
    /// nothing else can proceed until they're resolved.
    Conflicted { files: u32 },
    /// A multi-step git operation (merge/rebase/…) is paused in this repo.
    Operation(RepoOperation),
    /// The remote hasn't been fetched for a long time, so ahead/behind (and the
    /// CI/PR signals keyed off it) may be stale.
    StaleRemote { days: i64 },
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
            // Conflicts block everything else in the repo; treat like data risk.
            AttentionReason::Conflicted { .. } => Severity::Risk,
            AttentionReason::Operation(_) => Severity::Warn,
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
            AttentionReason::StaleRemote { .. } => Severity::Info,
        }
    }

    /// Short, presentation-neutral reason for the row (no color/glyph styling).
    pub fn label(&self) -> String {
        match self {
            AttentionReason::Unreadable => "could not read repo".to_string(),
            AttentionReason::Conflicted { files } => {
                format!(
                    "{files} conflict{} — resolve",
                    if *files == 1 { "" } else { "s" }
                )
            }
            AttentionReason::Operation(op) => format!("{} in progress", op.label()),
            AttentionReason::StaleRemote { days } => format!("not fetched in {days}d"),
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

        if repo.worktree.conflicted > 0 {
            reasons.push(AttentionReason::Conflicted {
                files: repo.worktree.conflicted,
            });
        }
        if let Some(op) = repo.operation {
            reasons.push(AttentionReason::Operation(op));
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
        // Only meaningful when the repo has a remote and has fetched at least once.
        if repo.remote_url.is_some()
            && let Some(fetched) = repo.last_fetch
            && now - fetched > STALE_FETCH_SECS
        {
            reasons.push(AttentionReason::StaleRemote {
                days: (now - fetched) / (24 * 60 * 60),
            });
        }
    }

    // Most urgent first (stable sort, so ties keep insertion order).
    reasons.sort_by_key(|r| std::cmp::Reverse(r.severity()));
    let primary = reasons.first().cloned();
    let base = primary
        .as_ref()
        .map_or(Severity::Ok, AttentionReason::severity);

    // Compound trouble escalates one tier: independent problems multiply the
    // odds something gets lost, so a repo carrying several of them outranks a
    // repo with the single worst one. Two reasons at Warn+ read as Risk; a
    // Notice-level repo with a second Notice reason reads as Warn. A single
    // reason is never escalated — its own severity stands.
    let warn_plus = reasons
        .iter()
        .filter(|r| r.severity() >= Severity::Warn)
        .count();
    let notice_plus = reasons
        .iter()
        .filter(|r| r.severity() >= Severity::Notice)
        .count();
    let severity = if warn_plus >= 2 {
        Severity::Risk
    } else if base == Severity::Notice && notice_plus >= 2 {
        Severity::Warn
    } else {
        base
    };

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

/// Fleet-wide counts for the header summary strip. Every count is derived from
/// the same per-repo [`assess`] reasons the rows show — the summary *inherits*
/// the attention judgment rather than re-deciding what "unpushed" or "dirty"
/// means with its own field checks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct FleetSummary {
    pub total: usize,
    pub needs_attention: usize,
    /// Repos whose overall assessment reaches [`Severity::Risk`] — the "drop
    /// everything" bucket.
    pub risk: usize,
    pub unpushed: usize,
    pub unpushed_aging: usize,
    pub behind: usize,
    pub dirty: usize,
    /// Repos with unmerged (conflicted) files.
    pub conflicted: usize,
    pub stash: usize,
    pub errors: usize,
}

/// Aggregate the fleet into headline counts, evaluated against `now`. One
/// [`assess`] per repo drives everything, so a repo counts as "unpushed" here
/// exactly when its row says so (a diverged repo counts as both unpushed and
/// behind — its `↑`/`↓` belong in both headline numbers).
pub fn fleet_summary(repos: &[RepoSnapshot], now: i64) -> FleetSummary {
    let mut s = FleetSummary {
        total: repos.len(),
        ..Default::default()
    };
    for repo in repos {
        let a = assess(repo, now);
        if a.needs_attention() {
            s.needs_attention += 1;
        }
        if a.severity >= Severity::Risk {
            s.risk += 1;
        }
        for reason in &a.reasons {
            match reason {
                AttentionReason::Unreadable => s.errors += 1,
                AttentionReason::Unpushed { aging, .. } => {
                    s.unpushed += 1;
                    if *aging {
                        s.unpushed_aging += 1;
                    }
                }
                AttentionReason::Diverged { .. } => {
                    s.unpushed += 1;
                    s.behind += 1;
                }
                AttentionReason::Behind { .. } => s.behind += 1,
                AttentionReason::Conflicted { .. } => s.conflicted += 1,
                AttentionReason::Uncommitted { .. } => s.dirty += 1,
                AttentionReason::Stash { .. } => s.stash += 1,
                // Row-level context with no headline number (yet).
                AttentionReason::Unpublished
                | AttentionReason::Operation(_)
                | AttentionReason::StaleRemote { .. }
                | AttentionReason::Detached => {}
            }
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

    #[test]
    fn conflicts_and_operations_outrank_plain_dirtiness() {
        let mut repo = repo(
            Branch::Named("main".into()),
            0,
            0,
            (0, 2, 0),
            0,
            None,
            Some(NOW),
            None,
        );
        repo.worktree.conflicted = 3;
        repo.operation = Some(RepoOperation::Merge);
        let a = assess(&repo, NOW);
        assert_eq!(a.severity, Severity::Risk);
        assert_eq!(
            a.primary,
            Some(AttentionReason::Conflicted { files: 3 }),
            "conflicts are the headline, not the dirty count"
        );
        assert!(
            a.reasons
                .contains(&AttentionReason::Operation(RepoOperation::Merge))
        );
        assert_eq!(
            AttentionReason::Conflicted { files: 1 }.label(),
            "1 conflict — resolve"
        );
        assert_eq!(
            AttentionReason::Operation(RepoOperation::Rebase).label(),
            "rebase in progress"
        );
    }

    #[test]
    fn stale_remote_is_informational_and_needs_a_remote() {
        let mut repo = repo(
            Branch::Named("main".into()),
            0,
            0,
            (0, 0, 0),
            0,
            None,
            Some(NOW),
            None,
        );
        repo.upstream = Some(Upstream {
            name: "origin/main".into(),
            ahead: 0,
            behind: 0,
        });
        repo.remote_url = Some("git@github.com:a/b.git".into());
        repo.last_fetch = Some(NOW - 30 * DAY);
        let a = assess(&repo, NOW);
        assert_eq!(a.primary, Some(AttentionReason::StaleRemote { days: 30 }));
        assert_eq!(a.severity, Severity::Info, "a stale remote nags gently");
        // Without a remote URL, an old (or absent) fetch stamp says nothing.
        repo.remote_url = None;
        assert!(assess(&repo, NOW).reasons.is_empty());
    }

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
            operation: None,
            default_branch: None,
            last_fetch: None,
            latest_tag: None,
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
        // Risk bucket: the errored repo and the aging-unpushed repo.
        assert_eq!(s.risk, 2);
    }

    #[test]
    fn compound_trouble_escalates_one_tier() {
        // Two Warn-level reasons (diverged + stale stash) read as Risk.
        let two_warns = repo(
            main_branch(),
            2,
            3,
            (0, 0, 0),
            1,
            Some(NOW - 10 * DAY),
            Some(NOW),
            None,
        );
        let a = assess(&two_warns, NOW);
        assert_eq!(a.severity, Severity::Risk);
        // The primary reason keeps its own severity — only the overall escalates.
        assert_eq!(
            a.primary,
            Some(AttentionReason::Diverged {
                ahead: 2,
                behind: 3
            })
        );

        // Behind AND dirty (two Notices) reads as Warn — more than either alone.
        let behind_dirty = repo(main_branch(), 0, 4, (0, 2, 0), 0, None, Some(NOW), None);
        assert_eq!(assess(&behind_dirty, NOW).severity, Severity::Warn);

        // A single reason is never escalated.
        let just_behind = repo(main_branch(), 0, 4, (0, 0, 0), 0, None, Some(NOW), None);
        assert_eq!(assess(&just_behind, NOW).severity, Severity::Notice);
    }

    #[test]
    fn fleet_summary_inherits_the_assessment_judgment() {
        // A diverged repo counts in BOTH unpushed and behind — its arrows sit
        // in both headline numbers, exactly as the row shows them.
        let diverged = repo(main_branch(), 2, 3, (0, 0, 0), 0, None, Some(NOW), None);
        let s = fleet_summary(&[diverged], NOW);
        assert_eq!((s.unpushed, s.behind), (1, 1));
        assert_eq!(s.needs_attention, 1);
    }
}
