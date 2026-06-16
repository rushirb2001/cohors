//! The browser's GitHub data source (v0.5 slice 2).
//!
//! The browser is sandboxed: it can't run `gh auth token` or read env vars like
//! the native TUI. So `cohors web` (the native server) holds the token and
//! proxies GitHub for us — the page calls a **same-origin** `/gh/<path>` and the
//! server injects the `Authorization` header (see `cohors-tui/src/web.rs`). The
//! token never reaches the browser, and the user does nothing. The responses map
//! onto the same `cohors-core` models as the TUI (ADR-002).
//!
//! When the page is served *without* that proxy (e.g. plain `trunk serve`, or a
//! machine with no GitHub login), `/gh/...` fails and the app falls back to the
//! demo fleet.

use cohors_core::{Branch, CommitMeta, RepoId, RepoSnapshot, WorktreeStatus};
use gloo_net::http::Request;
use serde::Deserialize;

/// Same-origin path proxied to the GitHub REST API by the local `cohors web`
/// server, which injects the token.
const PROXY: &str = "/gh";

/// One repo as returned by `GET /user/repos`. Only the fields we render.
#[derive(Deserialize)]
struct RepoItem {
    name: String,
    full_name: String,
    default_branch: String,
    pushed_at: Option<String>,
    description: Option<String>,
    #[serde(default)]
    private: bool,
    #[serde(default)]
    fork: bool,
    #[serde(default)]
    archived: bool,
    html_url: String,
    owner: Owner,
}

#[derive(Deserialize)]
struct Owner {
    login: String,
}

/// Fetch the authenticated user's repositories (most-recently-pushed first) via
/// the local proxy and map them onto `cohors-core` snapshots. Paginates up to 300
/// repos. Local-only fields (worktree, ahead/behind, stash) are empty — the
/// browser has no working copy; remote signals (CI/PRs) are enriched in a later
/// slice.
pub async fn fetch_repos() -> Result<Vec<RepoSnapshot>, String> {
    let mut out = Vec::new();
    for page in 1..=3u32 {
        let url = format!("{PROXY}/user/repos?per_page=100&sort=pushed&page={page}");
        let resp = Request::get(&url)
            .send()
            .await
            .map_err(|e| format!("network error: {e}"))?;

        if !resp.ok() {
            return Err(match resp.status() {
                401 => "not signed in to GitHub — run `gh auth login`".to_string(),
                403 => "GitHub rate-limited the request (or the token lacks scope)".to_string(),
                502 => "couldn't reach GitHub from the local server".to_string(),
                other => format!("GitHub API returned HTTP {other}"),
            });
        }

        let items: Vec<RepoItem> = resp
            .json()
            .await
            .map_err(|e| format!("could not parse GitHub response: {e}"))?;
        let got = items.len();
        out.extend(items.into_iter().map(to_snapshot));
        if got < 100 {
            break; // last page
        }
    }
    Ok(out)
}

/// Map a GitHub repo onto a `RepoSnapshot`. The "last commit" carries the push
/// time (the cheap activity signal the list endpoint gives) and the repo
/// description as its summary; the real commit message would cost a call per repo.
fn to_snapshot(it: RepoItem) -> RepoSnapshot {
    let ts = it.pushed_at.as_deref().and_then(parse_iso);
    let last_commit = ts.map(|t| CommitMeta {
        short_id: String::new(),
        author: it.owner.login.clone(),
        timestamp: t,
        summary: it.description.clone().unwrap_or_default(),
    });

    // Surface visibility/state as a tag in the name so the fleet reads at a glance.
    let mut tags = Vec::new();
    if it.private {
        tags.push("private");
    }
    if it.fork {
        tags.push("fork");
    }
    if it.archived {
        tags.push("archived");
    }
    let name = if tags.is_empty() {
        it.name
    } else {
        format!("{}  ·  {}", it.name, tags.join(" · "))
    };

    RepoSnapshot {
        id: RepoId(it.full_name),
        name,
        path: None, // purely remote — no local working copy in the browser
        branch: Branch::Named(it.default_branch),
        upstream: None,
        worktree: WorktreeStatus::default(),
        stash_count: 0,
        stash_latest: None,
        last_commit,
        remote_url: Some(it.html_url),
        remote: None, // CI/PR enrichment lands in a later slice
        error: None,
    }
}

/// Parse an ISO-8601 timestamp (e.g. `2026-06-10T14:02:11Z`) to Unix seconds via
/// the browser's `Date.parse`, so we need no date crate in the WASM bundle.
fn parse_iso(s: &str) -> Option<i64> {
    let ms = js_sys::Date::parse(s);
    if ms.is_nan() {
        None
    } else {
        Some((ms / 1000.0) as i64)
    }
}
