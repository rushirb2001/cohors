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

use cohors_core::{
    Branch, CiStatus, CommitMeta, Contributor, PullRequest, RemoteInfo, RepoId, RepoSnapshot,
    WorktreeStatus,
};
use gloo_net::http::Request;
use serde::Deserialize;
use serde::de::DeserializeOwned;

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

// ── Per-repo enrichment: CI status + open PRs (the fleet-health signals) ──────

#[derive(Deserialize)]
struct CheckRunsResponse {
    #[serde(default)]
    check_runs: Vec<CheckRun>,
}

#[derive(Deserialize)]
struct CheckRun {
    #[serde(default)]
    status: String,
    conclusion: Option<String>,
}

/// Minimal — we only count open PRs.
#[derive(Deserialize)]
struct PrRow {}

/// Enrich one repo with its default-branch CI status and open-PR count, via the
/// proxy. Best-effort: a failed sub-fetch degrades to `None`/`0`, never errors —
/// enrichment must never break the fleet. Returns the `RemoteInfo` to attach to
/// the snapshot.
pub async fn enrich(full_name: &str, branch: &str) -> Option<RemoteInfo> {
    let (owner, repo) = full_name.split_once('/')?;

    // CI: combine GitHub Actions check runs on the default branch (ADR-040's
    // logic, reimplemented for the browser).
    let ci = {
        let url = format!("{PROXY}/repos/{owner}/{repo}/commits/{branch}/check-runs?per_page=100");
        match Request::get(&url).send().await {
            Ok(resp) if resp.ok() => match resp.json::<CheckRunsResponse>().await {
                Ok(c) => combine_ci(&c.check_runs),
                Err(_) => CiStatus::None,
            },
            _ => CiStatus::None,
        }
    };

    // Open PRs: the length of the open-pulls list (capped at a page).
    let open_prs = {
        let url = format!("{PROXY}/repos/{owner}/{repo}/pulls?state=open&per_page=100");
        match Request::get(&url).send().await {
            Ok(resp) if resp.ok() => resp
                .json::<Vec<PrRow>>()
                .await
                .map(|v| v.len() as u32)
                .unwrap_or(0),
            _ => 0,
        }
    };

    Some(RemoteInfo {
        host: "github.com".to_string(),
        owner: owner.to_string(),
        repo: repo.to_string(),
        default_branch: branch.to_string(),
        open_prs,
        prs_awaiting_review: 0,
        ci,
    })
}

// ── On-demand per-repo detail (the web's inspection window) ───────────────────

/// The rich, drill-in detail for one repo — the browser analog of the TUI's
/// `Enter` view (everything fetchable from GitHub).
#[derive(Clone, Default)]
pub struct RepoDetail {
    pub commits: Vec<CommitMeta>,
    pub prs: Vec<PullRequest>,
    pub contributors: Vec<Contributor>,
    pub open_issues: u32,
    pub latest_release: Option<String>,
    /// Commits on the default branch since `latest_release` (remote analog of the
    /// TUI's ahead/behind — the browser has no local clone to diff). `None` when
    /// there's no release to compare against.
    pub commits_since_release: Option<u32>,
    pub stars: u32,
    pub language: Option<String>,
}

/// Fetch a repo's full detail via the proxy. Each section is best-effort (a
/// failed/absent section is just empty), so the panel always renders. Called
/// on-demand when a repo is selected.
pub async fn fetch_detail(full_name: &str, branch: &str) -> RepoDetail {
    let mut d = RepoDetail::default();
    let Some((owner, repo)) = full_name.split_once('/') else {
        return d;
    };

    if let Some(m) = get_json::<RepoMeta>(&format!("{PROXY}/repos/{owner}/{repo}")).await {
        d.stars = m.stargazers_count;
        d.language = m.language;
    }
    if let Some(cs) = get_json::<Vec<CommitResponse>>(&format!(
        "{PROXY}/repos/{owner}/{repo}/commits?sha={branch}&per_page=10"
    ))
    .await
    {
        d.commits = cs.into_iter().map(map_commit).collect();
    }
    if let Some(prs) = get_json::<Vec<PrResponse>>(&format!(
        "{PROXY}/repos/{owner}/{repo}/pulls?state=open&per_page=20"
    ))
    .await
    {
        d.prs = prs.into_iter().map(map_pr).collect();
    }
    if let Some(cs) = get_json::<Vec<ContributorResponse>>(&format!(
        "{PROXY}/repos/{owner}/{repo}/contributors?per_page=8"
    ))
    .await
    {
        d.contributors = cs.into_iter().filter_map(map_contributor).collect();
    }
    if let Some(s) = get_json::<SearchResponse>(&format!(
        "{PROXY}/search/issues?q=repo:{owner}/{repo}+is:issue+is:open&per_page=1"
    ))
    .await
    {
        d.open_issues = s.total_count;
    }
    if let Some(r) =
        get_json::<ReleaseResponse>(&format!("{PROXY}/repos/{owner}/{repo}/releases/latest")).await
    {
        // Sync = how far the branch has moved past the released tag (`ahead_by`).
        if let Some(c) = get_json::<CompareResponse>(&format!(
            "{PROXY}/repos/{owner}/{repo}/compare/{tag}...{branch}",
            tag = r.tag_name
        ))
        .await
        {
            d.commits_since_release = Some(c.ahead_by);
        }
        d.latest_release = Some(r.tag_name);
    }
    d
}

/// GET a proxied path and decode JSON, or `None` on any failure (best-effort).
async fn get_json<T: DeserializeOwned>(path: &str) -> Option<T> {
    let resp = Request::get(path).send().await.ok()?;
    if !resp.ok() {
        return None;
    }
    resp.json::<T>().await.ok()
}

#[derive(Deserialize)]
struct RepoMeta {
    #[serde(default)]
    stargazers_count: u32,
    language: Option<String>,
}
#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    total_count: u32,
}
#[derive(Deserialize)]
struct ReleaseResponse {
    tag_name: String,
}
#[derive(Deserialize)]
struct CompareResponse {
    #[serde(default)]
    ahead_by: u32,
}
#[derive(Deserialize)]
struct CommitResponse {
    sha: String,
    commit: CommitBody,
    author: Option<UserLogin>,
}
#[derive(Deserialize)]
struct CommitBody {
    #[serde(default)]
    message: String,
    author: Option<GitAuthor>,
}
#[derive(Deserialize)]
struct GitAuthor {
    #[serde(default)]
    name: String,
    #[serde(default)]
    date: String,
}
#[derive(Deserialize)]
struct UserLogin {
    login: String,
}
#[derive(Deserialize)]
struct PrResponse {
    number: u32,
    #[serde(default)]
    title: String,
    user: Option<UserLogin>,
    #[serde(default)]
    draft: bool,
    head: Option<HeadRef>,
    #[serde(default)]
    html_url: String,
}
#[derive(Deserialize)]
struct HeadRef {
    #[serde(rename = "ref", default)]
    r#ref: String,
}
#[derive(Deserialize)]
struct ContributorResponse {
    login: Option<String>,
    #[serde(default)]
    contributions: u32,
}

fn map_commit(c: CommitResponse) -> CommitMeta {
    let date = c.commit.author.as_ref().map(|a| a.date.as_str()).unwrap_or("");
    let author = c
        .author
        .map(|u| u.login)
        .or_else(|| c.commit.author.as_ref().map(|a| a.name.clone()))
        .unwrap_or_default();
    CommitMeta {
        short_id: c.sha.chars().take(7).collect(),
        author,
        timestamp: parse_iso(date).unwrap_or(0),
        summary: c.commit.message.lines().next().unwrap_or("").to_string(),
    }
}

fn map_pr(p: PrResponse) -> PullRequest {
    PullRequest {
        number: p.number,
        title: p.title,
        author: p.user.map(|u| u.login).unwrap_or_default(),
        draft: p.draft,
        branch: p.head.map(|h| h.r#ref).unwrap_or_default(),
        url: p.html_url,
    }
}

fn map_contributor(c: ContributorResponse) -> Option<Contributor> {
    c.login.map(|login| Contributor {
        login,
        contributions: c.contributions,
    })
}

/// Fold check runs into one status: failing wins, then pending, then passing;
/// no runs ⇒ `None` ("no CI").
fn combine_ci(runs: &[CheckRun]) -> CiStatus {
    let (mut signal, mut fail, mut pending, mut pass) = (false, false, false, false);
    for run in runs {
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
                "failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure" => {
                    fail = true
                }
                "success" | "neutral" | "skipped" | "stale" => pass = true,
                _ => {}
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
