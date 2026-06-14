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
}

/// Render the commits as markdown grouped by repo (alphabetical), newest commit
/// first within each repo — a digest you can paste into a standup channel.
pub fn to_markdown(commits: &[StandupCommit], window: StandupWindow) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;

    let mut by_repo: BTreeMap<&str, Vec<&StandupCommit>> = BTreeMap::new();
    for commit in commits {
        by_repo
            .entry(commit.repo.as_str())
            .or_default()
            .push(commit);
    }

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
        by_repo.len(),
        plural(by_repo.len()),
    );
    let _ = writeln!(out);

    for (repo, mut list) in by_repo {
        list.sort_by_key(|c| std::cmp::Reverse(c.timestamp)); // newest first
        let _ = writeln!(out, "### {repo}");
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
    fn groups_by_repo_alphabetically_newest_first() {
        let commits = vec![
            commit("web", "aaa1111", "older web", NOW - 2 * DAY),
            commit("api", "bbb2222", "api change", NOW - DAY),
            commit("web", "ccc3333", "newer web", NOW),
        ];
        let md = to_markdown(&commits, StandupWindow::Week);

        assert!(md.contains("_3 commits across 2 repos_"));
        // api group comes before web (alphabetical).
        let api = md.find("### api").unwrap();
        let web = md.find("### web").unwrap();
        assert!(api < web);
        // Within web, the newer commit is listed first.
        let newer = md.find("newer web").unwrap();
        let older = md.find("older web").unwrap();
        assert!(newer < older);
        // Commits render as markdown bullets with the short id.
        assert!(md.contains("- `ccc3333` newer web"));
    }

    #[test]
    fn singular_counts_have_no_plural_s() {
        let md = to_markdown(
            &[commit("solo", "deadbee", "only one", NOW)],
            StandupWindow::Today,
        );
        assert!(md.contains("_1 commit across 1 repo_"));
    }
}
