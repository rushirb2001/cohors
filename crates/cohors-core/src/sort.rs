//! Sort modes and the (pure) ordering logic.
//!
//! Everything operates on indices into a `&[RepoSnapshot]`, so callers never
//! clone snapshots and the original slice is left untouched. Every mode ends
//! with deterministic tiebreaks (name, then id) so the order is *total* —
//! important for a non-jittery UI and stable snapshot tests.

use crate::attention;
use crate::model::RepoSnapshot;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// How the repo list is ordered. Cycled with the `s` key (see [`SortMode::next`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SortMode {
    /// Repos needing attention first, then most-recently committed. (Default.)
    #[default]
    DirtyFirst,
    /// Most-recently committed first.
    Recent,
    /// Alphabetical by name (case-insensitive).
    Name,
    /// Most out-of-sync with upstream first.
    AheadBehind,
}

impl SortMode {
    /// All modes, in the order the `s` key cycles through them.
    pub const ALL: [SortMode; 4] = [
        SortMode::DirtyFirst,
        SortMode::Recent,
        SortMode::Name,
        SortMode::AheadBehind,
    ];

    /// The next mode in the cycle (wraps around).
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Short label for the header, e.g. `dirty-first`.
    pub fn label(self) -> &'static str {
        match self {
            SortMode::DirtyFirst => "dirty-first",
            SortMode::Recent => "recent",
            SortMode::Name => "name",
            SortMode::AheadBehind => "ahead/behind",
        }
    }
}

/// Sort `indices` (which point into `repos`) in place according to `mode`.
pub fn sort_indices(repos: &[RepoSnapshot], indices: &mut [usize], mode: SortMode) {
    indices.sort_by(|&a, &b| compare(&repos[a], &repos[b], mode));
}

/// The comparator for a given mode. Returns how `a` orders relative to `b`
/// (`Less` means `a` comes first).
fn compare(a: &RepoSnapshot, b: &RepoSnapshot, mode: SortMode) -> Ordering {
    match mode {
        SortMode::DirtyFirst => by_rank(a, b)
            .then_with(|| recency(a, b))
            .then_with(|| by_name(a, b)),
        SortMode::Recent => recency(a, b).then_with(|| by_name(a, b)),
        SortMode::Name => by_name(a, b),
        // Larger divergence first; then recency, then name.
        SortMode::AheadBehind => b
            .divergence()
            .cmp(&a.divergence())
            .then_with(|| recency(a, b))
            .then_with(|| by_name(a, b)),
    }
}

/// Higher attention rank (more urgent) sorts first — a richer "dirty-first"
/// than a plain bool: errors, then diverged/unpushed, then behind, dirty, etc.
fn by_rank(a: &RepoSnapshot, b: &RepoSnapshot) -> Ordering {
    attention::rank(b).cmp(&attention::rank(a))
}

/// Most-recent commit first; repos with no known commit sort last.
fn recency(a: &RepoSnapshot, b: &RepoSnapshot) -> Ordering {
    match (a.last_commit_time(), b.last_commit_time()) {
        (Some(ta), Some(tb)) => tb.cmp(&ta), // larger timestamp (newer) first
        (Some(_), None) => Ordering::Less,   // known time sorts before unknown
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Case-insensitive name, then id, for a fully deterministic tiebreak.
fn by_name(a: &RepoSnapshot, b: &RepoSnapshot) -> Ordering {
    a.name
        .to_lowercase()
        .cmp(&b.name.to_lowercase())
        .then_with(|| a.id.0.cmp(&b.id.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::sample;

    /// Order the full set and return the resulting names, for compact asserts.
    fn order(repos: &[RepoSnapshot], mode: SortMode) -> Vec<String> {
        let mut idx: Vec<usize> = (0..repos.len()).collect();
        sort_indices(repos, &mut idx, mode);
        idx.into_iter().map(|i| repos[i].name.clone()).collect()
    }

    #[test]
    fn cycle_visits_every_mode_and_wraps() {
        assert_eq!(SortMode::default(), SortMode::DirtyFirst);
        assert_eq!(SortMode::DirtyFirst.next(), SortMode::Recent);
        assert_eq!(SortMode::Recent.next(), SortMode::Name);
        assert_eq!(SortMode::Name.next(), SortMode::AheadBehind);
        assert_eq!(SortMode::AheadBehind.next(), SortMode::DirtyFirst);
    }

    #[test]
    fn dirty_first_puts_attention_repos_on_top_then_recency() {
        let repos = vec![
            sample("clean-new", "clean-new", false, 0, 0, Some(100)),
            sample("dirty-old", "dirty-old", true, 0, 0, Some(10)),
            sample("clean-old", "clean-old", false, 0, 0, Some(5)),
            sample("dirty-new", "dirty-new", true, 0, 0, Some(50)),
        ];
        // Dirty repos first (newest of them first), then clean (newest first).
        assert_eq!(
            order(&repos, SortMode::DirtyFirst),
            ["dirty-new", "dirty-old", "clean-new", "clean-old"]
        );
    }

    #[test]
    fn recent_orders_by_timestamp_desc_with_none_last() {
        let repos = vec![
            sample("a", "a", false, 0, 0, Some(10)),
            sample("b", "b", false, 0, 0, None), // no commits → last
            sample("c", "c", false, 0, 0, Some(30)),
        ];
        assert_eq!(order(&repos, SortMode::Recent), ["c", "a", "b"]);
    }

    #[test]
    fn name_is_case_insensitive() {
        let repos = vec![
            sample("z", "Zebra", false, 0, 0, Some(1)),
            sample("a", "alpha", false, 0, 0, Some(1)),
            sample("b", "Beta", false, 0, 0, Some(1)),
        ];
        assert_eq!(order(&repos, SortMode::Name), ["alpha", "Beta", "Zebra"]);
    }

    #[test]
    fn ahead_behind_orders_by_total_divergence() {
        let mut a = sample("a", "a", false, 1, 0, Some(1)); // ahead 1
        a.upstream = Some(crate::model::Upstream {
            name: "origin/main".into(),
            ahead: 1,
            behind: 4,
        }); // divergence 5
        let b = sample("b", "b", false, 3, 0, Some(1)); // divergence 3
        let c = sample("c", "c", false, 0, 0, Some(1)); // divergence 0
        let repos = vec![c, a, b];
        assert_eq!(order(&repos, SortMode::AheadBehind), ["a", "b", "c"]);
    }

    #[test]
    fn sort_is_total_and_deterministic_on_ties() {
        // Identical sort keys except id → tiebroken by id, stably.
        let repos = vec![
            sample("id2", "same", false, 0, 0, Some(7)),
            sample("id1", "same", false, 0, 0, Some(7)),
        ];
        assert_eq!(order(&repos, SortMode::DirtyFirst), ["same", "same"]);
        let mut idx: Vec<usize> = (0..repos.len()).collect();
        sort_indices(&repos, &mut idx, SortMode::DirtyFirst);
        // id1 < id2, so index 1 (id1) comes first.
        assert_eq!(idx, [1, 0]);
    }
}
