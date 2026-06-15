//! Cross-fleet search — the agent's entry point for cross-repo refactors
//! (JTBD-9). This module owns the *metadata* kinds (path / name / branch), which
//! are cheap, pure filters over snapshots. The `content` kind needs to read
//! files, so it lives in the git adapter (`cohors_git::search_content`); core
//! stays I/O-free and WASM-safe.

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::model::{RepoId, RepoSnapshot};

/// What a search matches against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    /// The repo's canonical path.
    Path,
    /// The repo's alias / directory name.
    Name,
    /// The current branch label.
    Branch,
    /// File contents (handled by the git adapter, not this module).
    Content,
}

/// A single search result. For metadata kinds, `line`/`text` describe the match
/// (e.g. the matched name); for content hits (built by the adapter) they carry
/// the file line and its text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchHit {
    pub repo: RepoId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Utf8PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Search snapshot metadata (path / name / branch) with a case-insensitive
/// substring match. `Content` returns nothing here — it's an adapter concern.
pub fn search_metadata(repos: &[RepoSnapshot], query: &str, kind: SearchKind) -> Vec<SearchHit> {
    let needle = query.to_lowercase();
    repos
        .iter()
        .filter_map(|repo| match kind {
            SearchKind::Name => contains(&repo.name, &needle).then(|| SearchHit {
                repo: repo.id.clone(),
                path: repo.path.clone(),
                line: None,
                text: Some(repo.name.clone()),
            }),
            SearchKind::Path => repo
                .path
                .as_ref()
                .filter(|p| contains(p.as_str(), &needle))
                .map(|p| SearchHit {
                    repo: repo.id.clone(),
                    path: Some(p.clone()),
                    line: None,
                    text: None,
                }),
            SearchKind::Branch => {
                let label = repo.branch.label();
                contains(&label, &needle).then(|| SearchHit {
                    repo: repo.id.clone(),
                    path: repo.path.clone(),
                    line: None,
                    text: Some(label),
                })
            }
            SearchKind::Content => None,
        })
        .collect()
}

/// Case-insensitive substring test (`haystack` raw, `needle` already lowercased).
fn contains(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Branch, sample};

    fn fleet() -> Vec<RepoSnapshot> {
        let mut detached = sample("d", "detached-repo", false, 0, 0, Some(0));
        detached.branch = Branch::Detached("a1b2c3d".into());
        vec![
            sample("api", "payments-api", false, 0, 0, Some(0)),
            sample("web", "payments-web", false, 0, 0, Some(0)),
            sample("infra", "infra", false, 0, 0, Some(0)),
            detached,
        ]
    }

    #[test]
    fn name_search_is_case_insensitive_substring() {
        let hits = search_metadata(&fleet(), "PAYMENTS", SearchKind::Name);
        assert_eq!(hits.len(), 2);
        assert!(
            hits.iter()
                .all(|h| h.text.as_deref().unwrap().contains("payments"))
        );
    }

    #[test]
    fn path_search_matches_canonical_path() {
        // sample() sets path to /repos/<id>.
        let hits = search_metadata(&fleet(), "/repos/infra", SearchKind::Path);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].repo, RepoId("infra".into()));
        assert!(hits[0].path.is_some());
    }

    #[test]
    fn branch_search_matches_label() {
        let main = search_metadata(&fleet(), "main", SearchKind::Branch);
        assert_eq!(main.len(), 3); // the three named-main repos
        let detached = search_metadata(&fleet(), "detached", SearchKind::Branch);
        assert_eq!(detached.len(), 1);
    }

    #[test]
    fn content_kind_is_empty_here() {
        assert!(search_metadata(&fleet(), "anything", SearchKind::Content).is_empty());
    }
}
