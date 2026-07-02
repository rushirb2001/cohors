//! The browser's client for the `cohors web` local server.
//!
//! The page renders the **same local scan** the TUI/CLI/MCP produce: it GETs the
//! `cohors-core` snapshots the server serves (over the local folder under
//! `--root`) and deserializes them into the very same models, then drives the UI
//! through `cohors-core`'s `assess`/sort logic. The browser does no GitHub work
//! itself — the server enriches with remote CI/PRs and serves the result.

use cohors_core::{RemoteDetail, RepoChanges, RepoDetail, RepoSnapshot};
use gloo_net::http::Request;
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// One repo's drill-in detail, as served by `/api/detail`: the local view (the
/// TUI's `Enter` pane) plus the remote view (PRs, contributors, issues, release).
#[derive(Deserialize, Clone, Default)]
pub struct RepoDetailResponse {
    pub local: RepoDetail,
    /// The working-tree changes: the changed-file list and a size-capped patch
    /// (`#[serde(default)]` so older servers that omit it still deserialize).
    #[serde(default)]
    pub changes: RepoChanges,
    pub remote: Option<RemoteDetail>,
}

/// Session metadata: the roots being scanned and whether `--watch` is on.
#[derive(Deserialize, Clone, Default)]
pub struct Meta {
    pub roots: Vec<String>,
    pub watch: bool,
}

/// The server's session metadata (roots + `--watch`).
pub async fn fetch_meta() -> Meta {
    get_json("/api/meta").await.unwrap_or_default()
}

/// The local scan (fast — remote signals left empty), for first paint.
pub async fn fetch_repos() -> Result<Vec<RepoSnapshot>, String> {
    get_json("/api/repos").await
}

/// The scan enriched with remote CI/PRs (fetched after first paint so it doesn't
/// block the initial render).
pub async fn fetch_enriched() -> Result<Vec<RepoSnapshot>, String> {
    get_json("/api/repos?enrich=1").await
}

/// One repo's drill-in. `path` is the local repo path (drives the local detail);
/// `remote_url`, when present, drives the remote detail. Best-effort — a failure
/// yields an empty detail rather than an error.
pub async fn fetch_detail(path: &str, remote_url: Option<&str>) -> RepoDetailResponse {
    let mut url = format!("/api/detail?path={}", encode(path));
    if let Some(u) = remote_url.filter(|u| !u.is_empty()) {
        url.push_str("&url=");
        url.push_str(&encode(u));
    }
    get_json(&url).await.unwrap_or_default()
}

/// Run a registry action (`fetch`/`pull`/`push`/`commit`/`stash`/`run`) over the
/// repos matching `selector`, server-side (`POST /api/action`). Returns the raw
/// JSON result (`{ targets, results }`, a dry-run preview, or `{ error }`). This
/// is the first write path from the browser — the server enforces the gates, so
/// a read-only server just refuses with a message the caller can surface.
pub async fn post_action(body: serde_json::Value) -> Result<serde_json::Value, String> {
    let resp = Request::post("/api/action")
        .json(&body)
        .map_err(|e| format!("could not build request: {e}"))?
        .send()
        .await
        .map_err(|e| format!("network error: {e}"))?;
    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| format!("could not parse the server response: {e}"))
}

/// GET `url` and decode its JSON body into `T`.
async fn get_json<T: DeserializeOwned>(url: &str) -> Result<T, String> {
    let resp = Request::get(url)
        .send()
        .await
        .map_err(|e| format!("network error: {e}"))?;
    if !resp.ok() {
        return Err(format!("the cohors server returned HTTP {}", resp.status()));
    }
    resp.json::<T>()
        .await
        .map_err(|e| format!("could not parse the server response: {e}"))
}

/// Percent-encode a query value (everything outside the URL "unreserved" set), so
/// repo paths with slashes, spaces, or unicode round-trip through the server's
/// matching decoder.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
