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

/// Directory names whose *contents* are secrets or repo internals — a search
/// must never descend into (or report from) these.
const SENSITIVE_DIRS: &[&str] = &[
    ".git", ".ssh", ".gnupg", ".aws", ".kube", ".gcloud", ".azure", ".docker",
];

/// Exact file names that carry credentials.
const SENSITIVE_FILES: &[&str] = &[
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
    "credentials",
];

/// File extensions that are key material.
const SENSITIVE_EXTS: &[&str] = &["pem", "key", "p12", "pfx", "jks", "keystore"];

/// `.env` variants that are *templates*, not real secrets — the one exception
/// to the `.env*` block.
const ENV_TEMPLATES: &[&str] = &[".env.example", ".env.sample", ".env.template", ".env.dist"];

/// The fleet-wide privacy rule for search: whether a path (relative or
/// absolute) points at — or into — something that must never appear in search
/// results (key material, credential stores, `.env` files, repo internals).
///
/// This is the *judgment*; adapters apply it before reading a file, and any
/// surface can re-apply it defensively on hits it receives. Deliberately
/// conservative: skipping a harmless file named `server.key` costs a missed
/// hit, while surfacing a real one leaks a secret.
/// Whether a single directory *name* is a secret store nothing should descend
/// into (`.ssh`, `.aws`, `.git`, …). Narrower than [`is_sensitive_path`] on
/// purpose: discovery prunes with this, and a *project* that happens to be
/// named `credentials` must still show up in the fleet — only the well-known
/// dot-directories are off limits.
pub fn is_sensitive_dir(name: &str) -> bool {
    SENSITIVE_DIRS.contains(&name.to_lowercase().as_str())
}

pub fn is_sensitive_path(path: &str) -> bool {
    path.split(['/', '\\']).any(|component| {
        let c = component.to_lowercase();
        if c.is_empty() {
            return false;
        }
        if SENSITIVE_DIRS.contains(&c.as_str()) || SENSITIVE_FILES.contains(&c.as_str()) {
            return true;
        }
        // `.env`, `.env.local`, `.env.production`, … — but not the templates.
        if c.starts_with(".env") && !ENV_TEMPLATES.contains(&c.as_str()) {
            return true;
        }
        // Private key files by conventional name or extension.
        if c.starts_with("id_rsa")
            || c.starts_with("id_ecdsa")
            || c.starts_with("id_ed25519")
            || c.starts_with("id_dsa")
        {
            return true;
        }
        c.rsplit_once('.')
            .is_some_and(|(_, ext)| SENSITIVE_EXTS.contains(&ext))
    })
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

    #[test]
    fn sensitive_paths_are_flagged() {
        for path in [
            ".env",
            ".env.local",
            "config/.env.production",
            ".git/config",
            "sub/.ssh/id_rsa",
            "id_ed25519.pub",
            "certs/server.pem",
            "keys/signing.key",
            ".aws/credentials",
            "app/credentials",
            ".npmrc",
            r"win\.ssh\key.pem",
        ] {
            assert!(is_sensitive_path(path), "{path} should be sensitive");
        }
    }

    #[test]
    fn ordinary_paths_and_env_templates_are_not_flagged() {
        for path in [
            "src/main.rs",
            "docs/environment.md", // "environment" ≠ ".env"
            ".env.example",
            "config/.env.template",
            "keyboard.rs",  // contains "key" but isn't a .key file
            "monkey.patch", // ends in neither list
            ".github/workflows/ci.yml",
        ] {
            assert!(!is_sensitive_path(path), "{path} should be fine");
        }
    }
}
