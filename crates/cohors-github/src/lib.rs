//! `cohors-github` — the remote data source (v0.2).
//!
//! Enriches `cohors_core::RepoSnapshot`s with GitHub [`RemoteInfo`] (open PRs,
//! CI status). Per ADR-010 it exposes a plain *blocking* API that returns core
//! models; the TUI runs [`enrich`] on a background thread (ADR-012). Per ADR-016
//! it talks to the GitHub REST API over a blocking HTTP client, with the token
//! passed in (native: `gh auth token` / `$GITHUB_TOKEN`; web: OAuth, later).
#![forbid(unsafe_code)]

use cohors_core::RepoSnapshot;

/// Fill `remote` on every snapshot whose `remote_url` resolves to GitHub.
///
/// With no `token`, or on network/rate-limit failure, snapshots are left
/// untouched (the dashboard shows "—") — this never blocks or errors out.
pub fn enrich(snapshots: &mut [RepoSnapshot], token: Option<&str>) {
    // Stub — filled by the cohors-github implementation step.
    let _ = (snapshots, token);
}

/// Acquire a GitHub token for native use: `gh auth token`, then `$GITHUB_TOKEN`.
pub fn discover_token() -> Option<String> {
    // Stub — the implementation step adds the `gh auth token` shell-out.
    std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty())
}
