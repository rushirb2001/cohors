//! The browser's client for the `cohors web` local server.
//!
//! Every function here is one HTTP call to the same-origin server and returns
//! the server's own `cohors-core` types. Nothing is invented on this side: a
//! failed call is an `Err` with what went wrong, and the page shows it.

use cohors_core::launchpad::{ActivityEvent, Board, LaunchpadEntry};
use cohors_core::{Overview, RemoteDetail, RepoChanges, RepoDetail, RepoSnapshot, StandupCommit};
use gloo_net::http::Request;
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// Session facts from `/api/meta`: what is being scanned and what the server
/// will let the page do.
#[derive(Deserialize, Clone, Default, PartialEq)]
#[serde(default)]
pub struct Meta {
    /// The folders being scanned.
    pub roots: Vec<String>,
    /// True when the server was started with `--demo` (sample data).
    pub demo: bool,
    /// Whether fetch / pull / push / commit / stash may run.
    pub allow_writes: bool,
    /// Whether `run` (an arbitrary shell command) may run.
    pub allow_run: bool,
}

/// The launchpad from `/api/launchpad`: what each repository declares about
/// itself, the cross-repository focus board, and recent agent sessions.
#[derive(Deserialize, Clone, Default, PartialEq)]
#[serde(default)]
pub struct Launchpad {
    pub entries: Vec<LaunchpadEntry>,
    pub board: Board,
    pub recent_activity: Vec<ActivityEvent>,
}

/// One repository's drill-in from `/api/detail`.
#[derive(Deserialize, Clone, Default)]
#[serde(default)]
pub struct Detail {
    /// Recent commits, changed files, branches and stashes.
    pub local: RepoDetail,
    /// The changed-file list again, with a size-capped patch.
    pub changes: RepoChanges,
    /// Pull requests, contributors, issues and the latest release. `None`
    /// when the repository has no remote the server can ask.
    pub remote: Option<RemoteDetail>,
}

/// One repository's commits in a standup.
#[derive(Deserialize, Clone, Default)]
#[serde(default)]
pub struct StandupGroup {
    pub repo: String,
    pub commits: Vec<StandupCommit>,
}

/// The standup from `/api/standup`.
#[derive(Deserialize, Clone, Default)]
#[serde(default)]
pub struct Standup {
    /// Start and end of the window, Unix seconds.
    pub since: i64,
    pub until: i64,
    /// The git identity whose commits are listed. `None` when git has no
    /// `user.email`, in which case the list is empty.
    pub author: Option<String>,
    pub total: usize,
    /// Most active repository first.
    pub groups: Vec<StandupGroup>,
    /// The same list as a markdown digest.
    pub markdown: String,
}

pub async fn fetch_meta() -> Result<Meta, String> {
    get_json("/api/meta").await
}

/// The shared overview model: the page renders it and computes nothing itself.
pub async fn fetch_overview() -> Result<Overview, String> {
    get_json("/api/overview").await
}

/// The raw snapshots. `enrich` asks the server to add GitHub CI and pull
/// request counts, which is slower, so the page asks for it second.
pub async fn fetch_repos(enrich: bool) -> Result<Vec<RepoSnapshot>, String> {
    let url = if enrich {
        "/api/repos?enrich=1"
    } else {
        "/api/repos"
    };
    get_json(url).await
}

pub async fn fetch_launchpad() -> Result<Launchpad, String> {
    get_json("/api/launchpad").await
}

/// `window` is `today` or `week`.
pub async fn fetch_standup(window: &str) -> Result<Standup, String> {
    get_json(&format!("/api/standup?window={window}")).await
}

/// `path` is the repository's folder; `remote_url`, when known, lets the
/// server add the remote half.
pub async fn fetch_detail(path: &str, remote_url: Option<&str>) -> Result<Detail, String> {
    let mut url = format!("/api/detail?path={}", encode(path));
    if let Some(remote) = remote_url.filter(|u| !u.is_empty()) {
        url.push_str("&url=");
        url.push_str(&encode(remote));
    }
    get_json(&url).await
}

/// Run a registry action (`fetch` / `pull` / `push` / `commit` / `stash` /
/// `run`) on the server. Returns the raw JSON: a dry-run preview, per-repo
/// results, or `{ "error": … }` when a gate refused it.
pub async fn post_action(body: serde_json::Value) -> Result<serde_json::Value, String> {
    let response = Request::post("/api/action")
        .json(&body)
        .map_err(|e| format!("could not build the request: {e}"))?
        .send()
        .await
        .map_err(|e| format!("could not reach the cohors server: {e}"))?;
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("could not read the server's answer: {e}"))
}

/// GET `url` and decode its JSON body into `T`.
async fn get_json<T: DeserializeOwned>(url: &str) -> Result<T, String> {
    let response = Request::get(url)
        .send()
        .await
        .map_err(|e| format!("could not reach the cohors server at {url}: {e}"))?;
    if !response.ok() {
        return Err(format!("{url} answered HTTP {}", response.status()));
    }
    response
        .json::<T>()
        .await
        .map_err(|e| format!("could not read the answer from {url}: {e}"))
}

/// Percent-encode a query value (everything outside the URL "unreserved" set),
/// so paths with slashes, spaces or unicode survive the trip to the server.
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
