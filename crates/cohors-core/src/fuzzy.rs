//! Fuzzy filtering over repo name + path, backed by `nucleo-matcher` (ADR-005).
//!
//! We use the low-level *matcher* (not the high-level `nucleo` crate, which
//! spawns worker threads) so this stays WASM-safe. Smart case + smart unicode
//! normalization come from `nucleo`'s default pattern parsing, which gives the
//! "type lowercase, match anything" feel users expect.

use crate::model::RepoSnapshot;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// One fuzzy match: which repo, its score, and the char positions matched in
/// the repo *name* (for highlighting the Repo column).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyHit {
    /// Index into the `repos` slice.
    pub index: usize,
    /// Match score; higher is better.
    pub score: u32,
    /// Char positions in the name to highlight. Empty when the match was only
    /// in the path (so there's nothing to highlight in the visible name).
    pub name_highlights: Vec<u32>,
}

/// Rank `candidates` (indices into `repos`) against `query`, best match first.
///
/// Each repo is scored by the better of its name and path match; non-matching
/// repos are dropped. Ties are broken by name then id so the order is
/// deterministic. An empty/whitespace query matches everything with score 0 —
/// but callers (see [`crate::view::compute_view`]) normally skip fuzzy entirely
/// in that case and sort instead.
pub fn rank(repos: &[RepoSnapshot], candidates: &[usize], query: &str) -> Vec<FuzzyHit> {
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);

    let mut hits: Vec<FuzzyHit> = candidates
        .iter()
        .filter_map(|&i| {
            let repo = &repos[i];

            // Name match — capture positions for highlighting.
            // Fresh buffers per repo: `Utf32Str::new` only allocates for
            // non-ASCII text, so the common case is free.
            let mut name_buf = Vec::new();
            let mut positions = Vec::new();
            let name_hay = Utf32Str::new(&repo.name, &mut name_buf);
            let name_score = pattern.indices(name_hay, &mut matcher, &mut positions);

            // Path match — score only.
            let path_score = repo.path.as_ref().and_then(|p| {
                let mut path_buf = Vec::new();
                let path_hay = Utf32Str::new(p.as_str(), &mut path_buf);
                pattern.score(path_hay, &mut matcher)
            });

            let score = match (name_score, path_score) {
                (Some(n), Some(p)) => n.max(p),
                (Some(n), None) => n,
                (None, Some(p)) => p,
                (None, None) => return None, // matched neither → drop
            };

            // Only keep highlights if the name itself matched; tidy them for
            // rendering.
            let mut name_highlights = if name_score.is_some() {
                positions
            } else {
                Vec::new()
            };
            name_highlights.sort_unstable();
            name_highlights.dedup();

            Some(FuzzyHit {
                index: i,
                score,
                name_highlights,
            })
        })
        .collect();

    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score) // higher score first
            .then_with(|| {
                repos[a.index]
                    .name
                    .to_lowercase()
                    .cmp(&repos[b.index].name.to_lowercase())
            })
            .then_with(|| repos[a.index].id.0.cmp(&repos[b.index].id.0))
    });
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::sample;

    fn repos() -> Vec<RepoSnapshot> {
        vec![
            sample("payments", "payments", false, 0, 0, Some(1)),
            sample("payments-web", "payments-web", false, 0, 0, Some(1)),
            sample("auth-service", "auth-service", false, 0, 0, Some(1)),
        ]
    }

    fn ranked_names(repos: &[RepoSnapshot], query: &str) -> Vec<String> {
        let all: Vec<usize> = (0..repos.len()).collect();
        rank(repos, &all, query)
            .into_iter()
            .map(|h| repos[h.index].name.clone())
            .collect()
    }

    #[test]
    fn matches_subsequence_and_drops_non_matches() {
        let repos = repos();
        let names = ranked_names(&repos, "pay");
        assert!(names.contains(&"payments".to_string()));
        assert!(names.contains(&"payments-web".to_string()));
        assert!(!names.contains(&"auth-service".to_string()));
    }

    #[test]
    fn lowercase_query_matches_mixed_case_names() {
        // Smart case: an all-lowercase query ignores case, so "pay" matches
        // "Payments". (An uppercase query like "PAY" would match case-sensitively
        // — intended, so users can be specific when they want to be.)
        let repos = vec![sample("p", "Payments", false, 0, 0, Some(1))];
        assert_eq!(ranked_names(&repos, "pay"), ["Payments"]);
    }

    #[test]
    fn returns_highlight_positions_for_name_matches() {
        let repos = repos();
        let all: Vec<usize> = (0..repos.len()).collect();
        let hits = rank(&repos, &all, "pay");
        let hit = hits
            .iter()
            .find(|h| repos[h.index].name == "payments")
            .expect("payments should match");
        // "pay" is a prefix of "payments" → positions 0,1,2.
        assert_eq!(hit.name_highlights, vec![0, 1, 2]);
    }

    #[test]
    fn matches_on_path_even_when_name_does_not() {
        // Path is "/repos/<id>"; query the id fragment, which isn't in the name.
        let repos = vec![sample("xyz123", "totally-different", false, 0, 0, Some(1))];
        let all = vec![0usize];
        let hits = rank(&repos, &all, "xyz123");
        assert_eq!(hits.len(), 1);
        // Matched via path, so no name highlights.
        assert!(hits[0].name_highlights.is_empty());
    }

    #[test]
    fn respects_the_candidate_set() {
        let repos = repos();
        // Only consider index 1 (payments-web); index 0 (payments) is excluded.
        let hits = rank(&repos, &[1], "pay");
        assert_eq!(hits.len(), 1);
        assert_eq!(repos[hits[0].index].name, "payments-web");
    }
}
