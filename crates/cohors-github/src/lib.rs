//! `cohors-github` — the remote data source (v0.2).
//!
//! Enriches `cohors_core::RepoSnapshot`s with GitHub [`RemoteInfo`] (open PRs,
//! CI status). Per ADR-010 it exposes a plain *blocking* API that returns core
//! models; the TUI runs [`enrich`] on a background thread (ADR-012). Per ADR-016
//! it talks to the GitHub REST API over a blocking HTTP client, with the token
//! passed in (native: `gh auth token` / `$GITHUB_TOKEN`; web: OAuth, later).
#![forbid(unsafe_code)]

mod url;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use cohors_core::{CiStatus, RemoteInfo, RepoSnapshot};

/// Base URL of the GitHub REST API.
const API_BASE: &str = "https://api.github.com";

/// Per-request timeout. The TUI calls [`enrich`] on a background thread, so a
/// slow or hung endpoint must never stall the whole scan past this bound.
const HTTP_TIMEOUT: Duration = Duration::from_secs(8);

/// How long a fetched [`RemoteInfo`] stays fresh. Every scan/refresh re-runs
/// [`enrich`]; the cache keeps that from hammering the API for unchanged repos.
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);

/// Process-wide cache of `owner/repo` → (fetched-at, info), guarding the GitHub
/// API against repeated scans. `Instant` is fine here — this is a native crate,
/// not WASM-bound `cohors-core`.
fn cache() -> &'static Mutex<HashMap<String, (Instant, RemoteInfo)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, RemoteInfo)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Fill `remote` on every snapshot whose `remote_url` resolves to GitHub.
///
/// With no `token`, or on network/rate-limit failure, snapshots are left
/// untouched (the dashboard shows "—") — this never blocks or errors out.
pub fn enrich(snapshots: &mut [RepoSnapshot], token: Option<&str>) {
    // No token → nothing to do. Leave every `remote` as `None`.
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return;
    };

    // One agent (and so one connection pool) for the whole pass, with the
    // timeout baked in so no single request can hang the background thread.
    let agent = ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).build();

    // Once GitHub tells us the rate limit is exhausted, stop making network
    // calls this round — further requests would just 403. Cache hits are still
    // served below since they don't touch the network.
    let mut rate_limited = false;

    for snap in snapshots.iter_mut() {
        let Some(remote_url) = snap.remote_url.as_deref() else {
            continue;
        };
        let Some((owner, repo)) = url::parse_repo(remote_url) else {
            // Non-GitHub or unparseable — leave `remote` as `None`.
            continue;
        };
        let key = format!("{owner}/{repo}");

        // Serve from cache if still fresh.
        if let Some(info) = cached(&key) {
            snap.remote = Some(info);
            continue;
        }

        if rate_limited {
            // Skip the network; a later scan (after the limit resets) fills it in.
            continue;
        }

        match fetch_remote(&agent, token, &owner, &repo) {
            Ok(info) => {
                store(&key, info.clone());
                snap.remote = Some(info);
            }
            Err(FetchError::RateLimited) => {
                tracing::warn!(repo = %key, "github rate limit hit; skipping remaining repos this round");
                rate_limited = true;
            }
            Err(FetchError::Other(reason)) => {
                // One bad repo (network blip, private/404, transient 5xx) must
                // never break the dashboard — log and leave `remote` as `None`.
                tracing::debug!(repo = %key, error = %reason, "github enrich failed; leaving remote empty");
            }
        }
    }
}

/// Acquire a GitHub token for native use: `gh auth token`, then `$GITHUB_TOKEN`.
/// Empty values count as absent. Never panics or blocks indefinitely.
pub fn discover_token() -> Option<String> {
    if let Some(token) = gh_auth_token() {
        return Some(token);
    }
    std::env::var("GITHUB_TOKEN")
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Try `gh auth token`. Returns `None` if `gh` is missing, errors, or prints
/// nothing useful. The trimmed stdout is the token.
fn gh_auth_token() -> Option<String> {
    let output = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if token.is_empty() { None } else { Some(token) }
}

/// Look up a still-fresh cache entry, cloning out the `RemoteInfo` if its TTL
/// hasn't elapsed.
fn cached(key: &str) -> Option<RemoteInfo> {
    let guard = cache().lock().ok()?;
    let (fetched_at, info) = guard.get(key)?;
    if fetched_at.elapsed() < CACHE_TTL {
        Some(info.clone())
    } else {
        None
    }
}

/// Insert/replace a cache entry, stamped with the current time. A poisoned lock
/// is ignored — caching is best-effort.
fn store(key: &str, info: RemoteInfo) {
    if let Ok(mut guard) = cache().lock() {
        guard.insert(key.to_string(), (Instant::now(), info));
    }
}

/// Why a single repo's enrichment failed. `RateLimited` is special-cased so the
/// caller can stop hitting the API for the rest of the round.
enum FetchError {
    /// The GitHub rate limit is exhausted (HTTP 403 with `x-ratelimit-remaining: 0`).
    RateLimited,
    /// Any other failure: transport error, non-2xx, or malformed JSON.
    Other(String),
}

/// Fetch the three GitHub facts that make up a [`RemoteInfo`] for `owner/repo`.
///
/// Endpoints used (all under <https://api.github.com>):
/// - `GET /repos/{owner}/{repo}` → `default_branch`
/// - `GET /search/issues?q=repo:{o}/{r}+is:pr+is:open` → open PR `total_count`
/// - `GET /search/issues?...+review-requested:@me` → review-requested `total_count`
/// - `GET /repos/{owner}/{repo}/commits/{branch}/status` → combined `state`
fn fetch_remote(
    agent: &ureq::Agent,
    token: &str,
    owner: &str,
    repo: &str,
) -> Result<RemoteInfo, FetchError> {
    // 1) Repo metadata → default branch.
    let repo_json: RepoResponse = get_json(agent, token, &format!("/repos/{owner}/{repo}"))?;
    let default_branch = repo_json.default_branch;

    // 2) Open PR count via the search API (`total_count`, capped page size of 1
    //    since we only want the count).
    let open_prs = {
        let q = format!("repo:{owner}/{repo} is:pr is:open");
        let path = format!("/search/issues?q={}&per_page=1", encode_query(&q));
        let res: SearchResponse = get_json(agent, token, &path)?;
        res.total_count
    };

    // 3) PRs awaiting *my* review. Best-effort: a failure here shouldn't sink the
    //    whole repo, so fall back to 0 rather than propagating.
    let prs_awaiting_review = {
        let q = format!("repo:{owner}/{repo} is:pr is:open review-requested:@me");
        let path = format!("/search/issues?q={}&per_page=1", encode_query(&q));
        match get_json::<SearchResponse>(agent, token, &path) {
            Ok(res) => res.total_count,
            // A rate-limit here still aborts the round; other errors degrade to 0.
            Err(FetchError::RateLimited) => return Err(FetchError::RateLimited),
            Err(FetchError::Other(_)) => 0,
        }
    };

    // 4) Combined CI status of the default branch's latest commit.
    let ci = {
        let path = format!("/repos/{owner}/{repo}/commits/{default_branch}/status");
        let res: StatusResponse = get_json(agent, token, &path)?;
        ci_status_from_state(&res.state)
    };

    Ok(RemoteInfo {
        host: "github.com".to_string(),
        owner: owner.to_string(),
        repo: repo.to_string(),
        default_branch,
        open_prs,
        prs_awaiting_review,
        ci,
    })
}

/// Issue a `GET {API_BASE}{path}` with the standard GitHub headers and decode
/// the JSON body into `T`. Maps non-2xx / rate-limit / transport / parse issues
/// onto [`FetchError`].
fn get_json<T: serde::de::DeserializeOwned>(
    agent: &ureq::Agent,
    token: &str,
    path: &str,
) -> Result<T, FetchError> {
    let resp = agent
        .get(&format!("{API_BASE}{path}"))
        .set("Authorization", &format!("Bearer {token}"))
        .set("User-Agent", "cohors")
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .call();

    match resp {
        Ok(response) => response
            .into_json::<T>()
            .map_err(|e| FetchError::Other(format!("decoding {path}: {e}"))),
        // Non-2xx HTTP status. 403 with a zeroed rate-limit header is the
        // exhausted-quota signal; everything else is a plain failure.
        Err(ureq::Error::Status(code, response)) => {
            if is_rate_limited(code, &response) {
                Err(FetchError::RateLimited)
            } else {
                Err(FetchError::Other(format!("{path} → HTTP {code}")))
            }
        }
        // Transport-level failure (DNS, TLS, timeout, connection reset, …).
        Err(ureq::Error::Transport(t)) => Err(FetchError::Other(format!("{path}: {t}"))),
    }
}

/// Is this response GitHub's "rate limit exhausted" signal? GitHub returns 403
/// (sometimes 429) with `x-ratelimit-remaining: 0` when the quota is spent.
fn is_rate_limited(code: u16, response: &ureq::Response) -> bool {
    if code != 403 && code != 429 {
        return false;
    }
    response
        .header("x-ratelimit-remaining")
        .map(|v| v.trim() == "0")
        // A 403 without the header (e.g. secondary rate limit / abuse detection)
        // is still treated as rate-limited so we back off rather than spin.
        .unwrap_or(true)
}

/// Minimal percent-encoding for a search query's reserved characters. We build
/// the query from known-safe `owner/repo` fragments plus literal qualifiers, so
/// only spaces, `:`, `/`, `@`, and `+` need escaping for the `q=` parameter.
fn encode_query(q: &str) -> String {
    let mut out = String::with_capacity(q.len());
    for b in q.bytes() {
        match b {
            // Unreserved per RFC 3986 — safe to leave as-is.
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Map a GitHub combined-status `state` string onto [`CiStatus`].
///
/// `"success"` → `Passing`, `"failure"`/`"error"` → `Failing`, `"pending"` →
/// `Pending`, and anything else (including `""` when no checks are configured)
/// → `None`. Matching is case-insensitive.
fn ci_status_from_state(state: &str) -> CiStatus {
    match state.trim().to_ascii_lowercase().as_str() {
        "success" => CiStatus::Passing,
        "failure" | "error" => CiStatus::Failing,
        "pending" => CiStatus::Pending,
        _ => CiStatus::None,
    }
}

/// `GET /repos/{owner}/{repo}` — we only need the default branch.
#[derive(serde::Deserialize)]
struct RepoResponse {
    default_branch: String,
}

/// `GET /search/issues` — we only need the result count.
#[derive(serde::Deserialize)]
struct SearchResponse {
    total_count: u32,
}

/// `GET /repos/{owner}/{repo}/commits/{ref}/status` — combined status state.
#[derive(serde::Deserialize)]
struct StatusResponse {
    state: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ci_state_success_is_passing() {
        assert_eq!(ci_status_from_state("success"), CiStatus::Passing);
    }

    #[test]
    fn ci_state_failure_and_error_are_failing() {
        assert_eq!(ci_status_from_state("failure"), CiStatus::Failing);
        assert_eq!(ci_status_from_state("error"), CiStatus::Failing);
    }

    #[test]
    fn ci_state_pending_is_pending() {
        assert_eq!(ci_status_from_state("pending"), CiStatus::Pending);
    }

    #[test]
    fn ci_state_empty_or_unknown_is_none() {
        assert_eq!(ci_status_from_state(""), CiStatus::None);
        assert_eq!(ci_status_from_state("unknown"), CiStatus::None);
        // No combined-status checks configured can come back as "neutral"/etc.
        assert_eq!(ci_status_from_state("neutral"), CiStatus::None);
    }

    #[test]
    fn ci_state_is_case_insensitive_and_trims() {
        assert_eq!(ci_status_from_state("SUCCESS"), CiStatus::Passing);
        assert_eq!(ci_status_from_state("  Failure  "), CiStatus::Failing);
    }

    #[test]
    fn encode_query_escapes_reserved_chars() {
        // Spaces, colons, slashes and `@` must be percent-encoded for `q=`.
        assert_eq!(
            encode_query("repo:o/r is:pr review-requested:@me"),
            "repo%3Ao%2Fr%20is%3Apr%20review-requested%3A%40me"
        );
    }

    #[test]
    fn encode_query_leaves_unreserved_chars() {
        assert_eq!(encode_query("abcXYZ_0-9.~"), "abcXYZ_0-9.~");
    }
}
