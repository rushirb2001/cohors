//! Weekly-standup aggregation (pure): group the user's commits across repos and
//! render them as a shareable markdown digest. Clock-injected like the rest of
//! core. (v0.2)
//!
//! `cohors-git` collects the commits ([`StandupCommit`]); this module turns them
//! into markdown ([`to_markdown`]).

use serde::{Deserialize, Serialize};

/// One commit authored by the user, tagged with its repo. Produced by
/// `cohors-git`'s standup collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandupCommit {
    pub repo: String,
    pub short_id: String,
    pub summary: String,
    /// Commit time, Unix seconds (UTC).
    pub timestamp: i64,
}

/// The time window for a standup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandupWindow {
    Today,
    Week,
    Custom { since: i64, until: i64 },
}

impl StandupWindow {
    /// Resolve to a `[since, until)` range (Unix seconds) against `now`. Day
    /// boundaries are UTC.
    pub fn range(self, now: i64) -> (i64, i64) {
        const DAY: i64 = 24 * 60 * 60;
        match self {
            StandupWindow::Today => (now - now.rem_euclid(DAY), now),
            StandupWindow::Week => (now - 7 * DAY, now),
            StandupWindow::Custom { since, until } => (since, until),
        }
    }

    /// Short label for the header, e.g. `today`, `this week`.
    pub fn label(self) -> &'static str {
        match self {
            StandupWindow::Today => "today",
            StandupWindow::Week => "this week",
            StandupWindow::Custom { .. } => "custom range",
        }
    }

    /// Cycle through the preset windows (today ↔ this week).
    pub fn next(self) -> Self {
        match self {
            StandupWindow::Today => StandupWindow::Week,
            _ => StandupWindow::Today,
        }
    }
}

/// Group commits by repo, ordered by how much was done there (most commits
/// first, ties alphabetical), each repo's commits newest-first. Shared by the
/// markdown digest and the TUI standup view so both agree on ordering.
pub fn group_commits(commits: &[StandupCommit]) -> Vec<(String, Vec<StandupCommit>)> {
    use std::collections::BTreeMap;

    let mut by_repo: BTreeMap<&str, Vec<StandupCommit>> = BTreeMap::new();
    for commit in commits {
        by_repo
            .entry(commit.repo.as_str())
            .or_default()
            .push(commit.clone());
    }
    let mut groups: Vec<(String, Vec<StandupCommit>)> = by_repo
        .into_iter()
        .map(|(repo, list)| (repo.to_string(), list))
        .collect();
    groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
    for (_, list) in &mut groups {
        list.sort_by_key(|c| std::cmp::Reverse(c.timestamp)); // newest first
    }
    groups
}

/// Render the commits as markdown grouped by repo (most-active first), newest
/// commit first within each repo — a digest you can paste into a standup channel.
pub fn to_markdown(commits: &[StandupCommit], window: StandupWindow) -> String {
    use std::fmt::Write;

    let groups = group_commits(commits);

    let mut out = String::new();
    let _ = writeln!(out, "## Standup — {}", window.label());
    let _ = writeln!(out);

    if commits.is_empty() {
        let _ = writeln!(out, "_No commits {}._", window.label());
        return out;
    }

    let _ = writeln!(
        out,
        "_{} commit{} across {} repo{}_",
        commits.len(),
        plural(commits.len()),
        groups.len(),
        plural(groups.len()),
    );
    let _ = writeln!(out);

    for (repo, list) in &groups {
        let _ = writeln!(out, "### {repo} ({})", list.len());
        for commit in list {
            let _ = writeln!(out, "- `{}` {}", commit.short_id, commit.summary);
        }
        let _ = writeln!(out);
    }
    out
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 24 * 60 * 60;
    const NOW: i64 = 1_700_000_000;

    fn commit(repo: &str, id: &str, summary: &str, ts: i64) -> StandupCommit {
        StandupCommit {
            repo: repo.to_string(),
            short_id: id.to_string(),
            summary: summary.to_string(),
            timestamp: ts,
        }
    }

    #[test]
    fn window_ranges_are_correct() {
        // Today starts at the UTC midnight on or before `now`.
        let (since, until) = StandupWindow::Today.range(NOW);
        assert_eq!(until, NOW);
        assert_eq!(since, NOW - NOW.rem_euclid(DAY));
        assert!(since <= NOW && NOW - since < DAY);

        assert_eq!(StandupWindow::Week.range(NOW), (NOW - 7 * DAY, NOW));
        assert_eq!(
            StandupWindow::Custom {
                since: 10,
                until: 20
            }
            .range(NOW),
            (10, 20)
        );
    }

    #[test]
    fn empty_renders_no_commits() {
        let md = to_markdown(&[], StandupWindow::Week);
        assert!(md.contains("## Standup — this week"));
        assert!(md.contains("_No commits this week._"));
    }

    #[test]
    fn groups_by_repo_most_active_first_newest_within() {
        let commits = vec![
            commit("web", "aaa1111", "older web", NOW - 2 * DAY),
            commit("api", "bbb2222", "api change", NOW - DAY),
            commit("web", "ccc3333", "newer web", NOW),
        ];
        let md = to_markdown(&commits, StandupWindow::Week);

        assert!(md.contains("_3 commits across 2 repos_"));
        // Repo headers carry their commit count.
        assert!(md.contains("### web (2)"));
        assert!(md.contains("### api (1)"));
        // web (2 commits) is listed before api (1) — most active first.
        let web = md.find("### web (2)").unwrap();
        let api = md.find("### api (1)").unwrap();
        assert!(web < api);
        // Within web, the newer commit is listed first.
        let newer = md.find("newer web").unwrap();
        let older = md.find("older web").unwrap();
        assert!(newer < older);
        // Commits render as markdown bullets with the short id.
        assert!(md.contains("- `ccc3333` newer web"));
    }

    #[test]
    fn ties_in_count_fall_back_to_alphabetical() {
        let commits = vec![
            commit("zebra", "aaa1111", "z change", NOW),
            commit("alpha", "bbb2222", "a change", NOW),
        ];
        let md = to_markdown(&commits, StandupWindow::Week);
        // Both have 1 commit, so alphabetical: alpha before zebra.
        assert!(md.find("### alpha (1)").unwrap() < md.find("### zebra (1)").unwrap());
    }

    #[test]
    fn singular_counts_have_no_plural_s() {
        let md = to_markdown(
            &[commit("solo", "deadbee", "only one", NOW)],
            StandupWindow::Today,
        );
        assert!(md.contains("_1 commit across 1 repo_"));
        assert!(md.contains("### solo (1)"));
    }
}
