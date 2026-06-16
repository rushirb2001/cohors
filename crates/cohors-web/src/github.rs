//! The browser's GitHub data source (v0.5 slice 2).
//!
//! `cohors-github` talks to GitHub with `ureq` (blocking, native-only — ADR-016),
//! which can't compile to WASM. So the web app has its own tiny fetch over the
//! browser's `fetch` (via `gloo-net`) that returns the *same* `cohors-core`
//! models, keeping the front-ends aligned (ADR-002). GitHub's REST API allows
//! cross-origin browser requests, so this works directly from the page.

use cohors_core::{Branch, CommitMeta, RepoId, RepoSnapshot, WorktreeStatus};
use gloo_net::http::Request;
use serde::Deserialize;

const API: &str = "https://api.github.com";

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

/// Fetch the authenticated user's repositories (most-recently-pushed first) and
/// map them onto `cohors-core` snapshots. Paginates up to 300 repos. Local-only
/// fields (worktree, ahead/behind, stash) are empty — the browser has no working
/// copy; the remote signals (CI/PRs) are enriched in a later slice.
pub async fn fetch_repos(token: &str) -> Result<Vec<RepoSnapshot>, String> {
    let mut out = Vec::new();
    for page in 1..=3u32 {
        let url = format!("{API}/user/repos?per_page=100&sort=pushed&page={page}");
        let resp = Request::get(&url)
            .header("Authorization", &format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| format!("network error: {e}"))?;

        if !resp.ok() {
            let status = resp.status();
            return Err(match status {
                401 => "GitHub rejected the token (401) — check it has `repo` scope".to_string(),
                403 => "GitHub returned 403 — rate-limited or missing scope".to_string(),
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
