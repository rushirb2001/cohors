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

use cohors_core::{CiStatus, Contributor, PullRequest, RemoteDetail, RemoteInfo, RepoSnapshot};

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
///
/// `gh` is resolved against `$PATH` first, then a few well-known install
/// locations. This matters because an MCP server spawned by an agent (or any
/// GUI-launched process) often runs with a stripped `PATH` that omits Homebrew's
/// bin dir — without the fallback, a fully authenticated user still gets no
/// remote data because `gh` simply isn't found.
fn gh_auth_token() -> Option<String> {
    for gh in gh_candidates() {
        let Ok(output) = std::process::Command::new(&gh)
            .args(["auth", "token"])
            .output()
        else {
            continue; // not found at this location — try the next
        };
        if !output.status.success() {
            continue;
        }
        let token = String::from_utf8(output.stdout).ok()?.trim().to_string();
        if !token.is_empty() {
            return Some(token);
        }
    }
    None
}

/// Candidate `gh` binaries, in order: the bare name (honours `$PATH` when it's
/// intact), then the usual install locations a stripped `PATH` would miss —
/// Homebrew on Apple Silicon and Intel, Linuxbrew, and `~/.local/bin`.
fn gh_candidates() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let mut v = vec![
        PathBuf::from("gh"),
        PathBuf::from("/opt/homebrew/bin/gh"),
        PathBuf::from("/usr/local/bin/gh"),
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin/gh"),
    ];
    if let Ok(home) = std::env::var("HOME") {
        v.push(PathBuf::from(home).join(".local/bin/gh"));
    }
    v
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
/// - `GET /repos/{owner}/{repo}/commits/{branch}/check-runs` → Actions checks
fn fetch_remote(
    agent: &ureq::Agent,
    token: &str,
    owner: &str,
    repo: &str,
) -> Result<RemoteInfo, FetchError> {
    // 1) Repo metadata → default branch + identity/popularity (one call).
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

    // 4) CI status of the default branch's latest commit, from the **Checks API**
    //    (GitHub Actions and other Checks-API integrations) only. We deliberately
    //    do *not* read the legacy commit Status API: its combined endpoint reports
    //    "pending" even when a repo posts zero statuses, which made Actions-only
    //    repos look perpetually pending (and produced stale/broken signals). The
    //    Checks API is the modern, accurate source.
    let ci = {
        let checks = match get_json::<CheckRunsResponse>(
            agent,
            token,
            &format!("/repos/{owner}/{repo}/commits/{default_branch}/check-runs?per_page=100"),
        ) {
            Ok(c) => Some(c),
            Err(FetchError::RateLimited) => return Err(FetchError::RateLimited),
            Err(FetchError::Other(_)) => None,
        };
        combine_ci(checks.as_ref())
    };

    Ok(RemoteInfo {
        host: "github.com".to_string(),
        owner: owner.to_string(),
        repo: repo.to_string(),
        default_branch,
        open_prs,
        prs_awaiting_review,
        ci,
        description: repo_json.description.filter(|d| !d.trim().is_empty()),
        topics: repo_json.topics,
        stars: repo_json.stargazers_count,
        forks: repo_json.forks_count,
        watchers: repo_json.subscribers_count,
    })
}

/// Fetch the per-repo drill-in detail (open PRs + top contributors) for the
/// detail pane. Best-effort: a section that fails (or has no data) is empty
/// rather than failing the whole call. Returns `None` only when the remote
/// isn't a parseable GitHub repo or there's no token.
pub fn fetch_repo_detail(token: &str, remote_url: &str) -> Option<RemoteDetail> {
    if token.is_empty() {
        return None;
    }
    let (owner, repo) = url::parse_repo(remote_url)?;
    let agent = ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).build();

    let prs = get_json::<Vec<PrResponse>>(
        &agent,
        token,
        &format!("/repos/{owner}/{repo}/pulls?state=open&per_page=20"),
    )
    .map(|raw| {
        raw.into_iter()
            .map(|p| PullRequest {
                number: p.number,
                title: p.title,
                author: p.user.map(|u| u.login).unwrap_or_default(),
                draft: p.draft.unwrap_or(false),
                branch: p.head.map(|h| h.r#ref).unwrap_or_default(),
                url: p.html_url,
                requested_reviewers: p.requested_reviewers.into_iter().map(|u| u.login).collect(),
            })
            .collect()
    })
    .unwrap_or_default();

    let contributors = get_json::<Vec<ContributorResponse>>(
        &agent,
        token,
        &format!("/repos/{owner}/{repo}/contributors?per_page=10"),
    )
    .map(|raw| {
        raw.into_iter()
            .filter_map(|c| {
                c.login.map(|login| Contributor {
                    login,
                    contributions: c.contributions,
                })
            })
            .collect()
    })
    .unwrap_or_default();

    // Open issues (excluding PRs) via the search API; best-effort → 0.
    let open_issues = {
        let q = format!("repo:{owner}/{repo} is:issue is:open");
        get_json::<SearchResponse>(
            &agent,
            token,
            &format!("/search/issues?q={}&per_page=1", encode_query(&q)),
        )
        .map(|r| r.total_count)
        .unwrap_or(0)
    };

    // Latest release tag (404 when the repo has none → `None`).
    let latest_release = get_json::<ReleaseResponse>(
        &agent,
        token,
        &format!("/repos/{owner}/{repo}/releases/latest"),
    )
    .ok()
    .map(|r| r.tag_name);

    // ── Per-repo network extras. These live here (not in the fleet enrich pass)
    // deliberately: each costs a request, and the detail pane fetches exactly one
    // repo on demand, so the rate limit only pays for what the user inspects.

    // Open issues assigned to me; best-effort → 0.
    let assigned_issues = {
        let q = format!("repo:{owner}/{repo} is:issue is:open assignee:@me");
        get_json::<SearchResponse>(
            &agent,
            token,
            &format!("/search/issues?q={}&per_page=1", encode_query(&q)),
        )
        .map(|r| r.total_count)
        .unwrap_or(0)
    };

    // Is the default branch protected? Two hops: the repo call names the default
    // branch, the branch call carries `protected`. Failure → `None` (unknown).
    let default_branch_protected =
        get_json::<RepoResponse>(&agent, token, &format!("/repos/{owner}/{repo}"))
            .ok()
            .and_then(|r| {
                get_json::<BranchResponse>(
                    &agent,
                    token,
                    &format!("/repos/{owner}/{repo}/branches/{}", r.default_branch),
                )
                .ok()
            })
            .map(|b| b.protected);

    // The most recent Actions workflow run (any branch), folded to a CiStatus.
    let latest_run = get_json::<WorkflowRunsResponse>(
        &agent,
        token,
        &format!("/repos/{owner}/{repo}/actions/runs?per_page=1"),
    )
    .ok()
    .and_then(|w| w.workflow_runs.into_iter().next())
    .map(|run| run_ci(&run));

    Some(RemoteDetail {
        prs,
        contributors,
        open_issues,
        latest_release,
        assigned_issues,
        default_branch_protected,
        latest_run,
    })
}

/// Fold one workflow run's `status`/`conclusion` into a [`CiStatus`], mirroring
/// [`combine_ci`]'s vocabulary: not-completed → pending; failure-like conclusions
/// → failing; success-like → passing; anything unrecognized → `None` ("no signal").
fn run_ci(run: &WorkflowRun) -> CiStatus {
    if !run.status.trim().eq_ignore_ascii_case("completed") {
        return CiStatus::Pending;
    }
    match run
        .conclusion
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure" => {
            CiStatus::Failing
        }
        "success" | "neutral" | "skipped" | "stale" => CiStatus::Passing,
        _ => CiStatus::None,
    }
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

/// `GET /repos/{owner}/{repo}` — default branch plus the identity/popularity
/// fields that ride along in the same response (zero extra requests).
/// `watchers_count` is deliberately NOT read: it's a legacy alias for stars;
/// `subscribers_count` is the real "watching" number.
#[derive(serde::Deserialize)]
struct RepoResponse {
    default_branch: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    stargazers_count: u32,
    #[serde(default)]
    forks_count: u32,
    #[serde(default)]
    subscribers_count: u32,
}

/// `GET /search/issues` — we only need the result count.
#[derive(serde::Deserialize)]
struct SearchResponse {
    total_count: u32,
}

/// `GET /repos/{owner}/{repo}/commits/{ref}/check-runs` — GitHub Actions (and
/// other Checks-API) results for the commit.
#[derive(serde::Deserialize)]
struct CheckRunsResponse {
    #[serde(default)]
    check_runs: Vec<CheckRun>,
}

/// One check run. `status` is `queued`/`in_progress`/`completed`; `conclusion`
/// (set once completed) is `success`/`failure`/`neutral`/`cancelled`/`skipped`/
/// `timed_out`/`action_required`/`stale`/…
#[derive(serde::Deserialize)]
struct CheckRun {
    #[serde(default)]
    status: String,
    conclusion: Option<String>,
}

/// Fold the Checks API (GitHub Actions) results into one [`CiStatus`]. A failing
/// run wins, then a still-running one, then a passing one; with no check runs at
/// all it's [`CiStatus::None`] — "no CI", *not* pending. The legacy commit Status
/// API is deliberately not consulted (it reported "pending" for repos that post
/// no statuses, giving stale/broken signals).
fn combine_ci(checks: Option<&CheckRunsResponse>) -> CiStatus {
    let (mut signal, mut fail, mut pending, mut pass) = (false, false, false, false);

    if let Some(c) = checks {
        for run in &c.check_runs {
            signal = true;
            if !run.status.trim().eq_ignore_ascii_case("completed") {
                pending = true;
            } else {
                match run
                    .conclusion
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "failure" | "timed_out" | "cancelled" | "action_required"
                    | "startup_failure" => fail = true,
                    "success" | "neutral" | "skipped" | "stale" => pass = true,
                    _ => {}
                }
            }
        }
    }

    if !signal {
        CiStatus::None
    } else if fail {
        CiStatus::Failing
    } else if pending {
        CiStatus::Pending
    } else if pass {
        CiStatus::Passing
    } else {
        CiStatus::None
    }
}

/// `GET /repos/{owner}/{repo}/pulls` — the fields the detail pane shows.
/// `requested_reviewers` rides along in this list response for free; the
/// `mergeable`/`mergeable_state` fields do NOT (they exist only on the
/// per-PR endpoint, +1 request each) and are deliberately not fetched.
#[derive(serde::Deserialize)]
struct PrResponse {
    number: u32,
    title: String,
    html_url: String,
    #[serde(default)]
    draft: Option<bool>,
    user: Option<UserResponse>,
    head: Option<HeadResponse>,
    #[serde(default)]
    requested_reviewers: Vec<UserResponse>,
}

#[derive(serde::Deserialize)]
struct UserResponse {
    login: String,
}

#[derive(serde::Deserialize)]
struct HeadResponse {
    #[serde(rename = "ref")]
    r#ref: String,
}

/// `GET /repos/{owner}/{repo}/contributors` — login + commit count. `login` is
/// optional because anonymous entries omit it (we filter those out).
#[derive(serde::Deserialize)]
struct ContributorResponse {
    login: Option<String>,
    #[serde(default)]
    contributions: u32,
}

/// `GET /repos/{owner}/{repo}/releases/latest` — we only need the tag.
#[derive(serde::Deserialize)]
struct ReleaseResponse {
    tag_name: String,
}

/// `GET /repos/{owner}/{repo}/branches/{branch}` — we only need `protected`.
#[derive(serde::Deserialize)]
struct BranchResponse {
    #[serde(default)]
    protected: bool,
}

/// `GET /repos/{owner}/{repo}/actions/runs?per_page=1` — the latest run only.
#[derive(serde::Deserialize)]
struct WorkflowRunsResponse {
    #[serde(default)]
    workflow_runs: Vec<WorkflowRun>,
}

/// One workflow run — the same status/conclusion vocabulary as [`CheckRun`].
#[derive(serde::Deserialize)]
struct WorkflowRun {
    #[serde(default)]
    status: String,
    conclusion: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(status: &str, conclusion: Option<&str>) -> CheckRun {
        CheckRun {
            status: status.to_string(),
            conclusion: conclusion.map(str::to_string),
        }
    }
    fn checks(runs: Vec<CheckRun>) -> CheckRunsResponse {
        CheckRunsResponse { check_runs: runs }
    }

    #[test]
    fn ci_actions_passing_is_passing() {
        let c = checks(vec![check("completed", Some("success"))]);
        assert_eq!(combine_ci(Some(&c)), CiStatus::Passing);
    }

    #[test]
    fn ci_actions_failure_wins() {
        let c = checks(vec![
            check("completed", Some("success")),
            check("completed", Some("failure")),
            check("in_progress", None),
        ]);
        assert_eq!(combine_ci(Some(&c)), CiStatus::Failing);
    }

    #[test]
    fn ci_actions_running_is_pending() {
        let c = checks(vec![
            check("completed", Some("success")),
            check("queued", None),
        ]);
        assert_eq!(combine_ci(Some(&c)), CiStatus::Pending);
    }

    #[test]
    fn ci_actions_only_skipped_or_neutral_is_passing() {
        let c = checks(vec![check("completed", Some("skipped"))]);
        assert_eq!(combine_ci(Some(&c)), CiStatus::Passing);
    }

    #[test]
    fn ci_no_check_runs_is_none() {
        // No Actions checks at all — "no CI", not pending (the legacy Status API,
        // which used to report a phantom "pending" here, is no longer consulted).
        assert_eq!(combine_ci(Some(&checks(vec![]))), CiStatus::None);
        assert_eq!(combine_ci(None), CiStatus::None);
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

    fn run(status: &str, conclusion: Option<&str>) -> WorkflowRun {
        WorkflowRun {
            status: status.to_string(),
            conclusion: conclusion.map(str::to_string),
        }
    }

    #[test]
    fn latest_run_maps_like_check_runs() {
        assert_eq!(run_ci(&run("in_progress", None)), CiStatus::Pending);
        assert_eq!(run_ci(&run("queued", None)), CiStatus::Pending);
        assert_eq!(
            run_ci(&run("completed", Some("success"))),
            CiStatus::Passing
        );
        assert_eq!(
            run_ci(&run("completed", Some("failure"))),
            CiStatus::Failing
        );
        assert_eq!(
            run_ci(&run("completed", Some("cancelled"))),
            CiStatus::Failing
        );
        // Unknown conclusion is "no signal", not a false pass/fail.
        assert_eq!(run_ci(&run("completed", Some("mystery"))), CiStatus::None);
    }

    #[test]
    fn pr_list_response_parses_requested_reviewers() {
        // The list endpoint carries reviewers inline — this is the zero-request
        // parse the PR-readiness feature depends on.
        let json = r#"[{
            "number": 7, "title": "t", "html_url": "u", "draft": false,
            "user": {"login": "alice"},
            "head": {"ref": "feat"},
            "requested_reviewers": [{"login": "bob"}, {"login": "carol"}]
        }]"#;
        let prs: Vec<PrResponse> = serde_json::from_str(json).unwrap();
        let logins: Vec<&str> = prs[0]
            .requested_reviewers
            .iter()
            .map(|u| u.login.as_str())
            .collect();
        assert_eq!(logins, ["bob", "carol"]);
    }

    #[test]
    fn repo_response_parses_identity_fields() {
        let json = r#"{
            "default_branch": "main", "description": "a tool", "topics": ["cli","rust"],
            "stargazers_count": 42, "forks_count": 3, "subscribers_count": 5,
            "watchers_count": 42
        }"#;
        let r: RepoResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.description.as_deref(), Some("a tool"));
        assert_eq!(r.topics, ["cli", "rust"]);
        assert_eq!((r.stargazers_count, r.forks_count), (42, 3));
        // watchers = subscribers_count (5), NOT the legacy watchers_count alias (42).
        assert_eq!(r.subscribers_count, 5);
    }
}
