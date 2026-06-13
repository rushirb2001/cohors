//! The view pipeline: turn a `&[RepoSnapshot]` plus the current view state
//! (sort mode, dirty-only toggle, fuzzy query) into an ordered list of rows to
//! render.
//!
//! This is the single source of "what to show, and in what order" — shared
//! verbatim by the TUI and (later) the web app. Keeping it here, returning
//! plain indices, means the front-ends only map a view-model onto widgets.

use crate::fuzzy;
use crate::model::RepoSnapshot;
use crate::sort::{SortMode, sort_indices};

/// Current view state. Borrows the query so the caller can pass its live input
/// buffer without allocating.
#[derive(Debug, Clone, Copy)]
pub struct ViewParams<'a> {
    pub sort: SortMode,
    pub dirty_only: bool,
    pub query: &'a str,
}

/// One row of the rendered list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewRow {
    /// Index into the `repos` slice passed to [`compute_view`].
    pub index: usize,
    /// Char positions in the repo name to highlight (fuzzy matches). Empty when
    /// not filtering, or when the match was only in the path.
    pub name_highlights: Vec<u32>,
}

/// Build the ordered, filtered rows for the current view.
///
/// Order of operations:
/// 1. Narrow to the dirty-only set if the toggle is on.
/// 2. If there's a query, fuzzy-rank the survivors (best match first), ignoring
///    the sort mode.
/// 3. Otherwise, order the survivors by the active sort mode.
pub fn compute_view(repos: &[RepoSnapshot], params: &ViewParams<'_>) -> Vec<ViewRow> {
    let mut candidates: Vec<usize> = (0..repos.len()).collect();
    if params.dirty_only {
        candidates.retain(|&i| repos[i].needs_attention());
    }

    let query = params.query.trim();
    if !query.is_empty() {
        return fuzzy::rank(repos, &candidates, query)
            .into_iter()
            .map(|hit| ViewRow {
                index: hit.index,
                name_highlights: hit.name_highlights,
            })
            .collect();
    }

    sort_indices(repos, &mut candidates, params.sort);
    candidates
        .into_iter()
        .map(|index| ViewRow {
            index,
            name_highlights: Vec::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::sample;

    fn fixture() -> Vec<RepoSnapshot> {
        vec![
            sample("payments", "payments", true, 2, 0, Some(100)), // dirty + ahead
            sample("web-app", "web-app", false, 0, 0, Some(200)),  // clean, newest
            sample("auth", "auth", false, 0, 0, Some(50)),         // clean, oldest
        ]
    }

    fn names(rows: &[ViewRow], repos: &[RepoSnapshot]) -> Vec<String> {
        rows.iter().map(|r| repos[r.index].name.clone()).collect()
    }

    #[test]
    fn no_query_orders_by_sort_mode() {
        let repos = fixture();
        let rows = compute_view(
            &repos,
            &ViewParams {
                sort: SortMode::DirtyFirst,
                dirty_only: false,
                query: "",
            },
        );
        // payments needs attention → first; then clean by recency.
        assert_eq!(names(&rows, &repos), ["payments", "web-app", "auth"]);
        assert!(rows.iter().all(|r| r.name_highlights.is_empty()));
    }

    #[test]
    fn dirty_only_filters_to_attention_repos() {
        let repos = fixture();
        let rows = compute_view(
            &repos,
            &ViewParams {
                sort: SortMode::DirtyFirst,
                dirty_only: true,
                query: "",
            },
        );
        assert_eq!(names(&rows, &repos), ["payments"]);
    }

    #[test]
    fn query_overrides_sort_and_filters_by_match() {
        let repos = fixture();
        let rows = compute_view(
            &repos,
            &ViewParams {
                sort: SortMode::Name,
                dirty_only: false,
                query: "auth",
            },
        );
        assert_eq!(names(&rows, &repos), ["auth"]);
    }

    #[test]
    fn whitespace_query_is_treated_as_no_query() {
        let repos = fixture();
        let rows = compute_view(
            &repos,
            &ViewParams {
                sort: SortMode::Recent,
                dirty_only: false,
                query: "   ",
            },
        );
        assert_eq!(names(&rows, &repos), ["web-app", "payments", "auth"]);
    }

    #[test]
    fn dirty_only_and_query_combine() {
        let repos = fixture();
        // dirty-only keeps just "payments"; query "web" then matches none of it.
        let rows = compute_view(
            &repos,
            &ViewParams {
                sort: SortMode::DirtyFirst,
                dirty_only: true,
                query: "web",
            },
        );
        assert!(rows.is_empty());
    }
}
