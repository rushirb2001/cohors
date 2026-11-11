//! The `cohors web` HTTP server — the native half of the web front-end.
//!
//! The web app is just another front-end over the **same local scan** the TUI,
//! CLI, and MCP run. This server takes a scan closure (so any caller — the TUI's
//! `Scanner`, or the standalone binary — supplies the fleet), serves those
//! `cohors-core` snapshots as JSON, and now also *acts* on them: `POST /api/action`
//! dispatches through the shared `cohors-actions` orchestration the MCP uses, so
//! the web surface is no longer read-only. The browser renders the reads through
//! the same `assess`/sort logic — your *local* fleet, not a remote account view.
//!
//! Endpoints:
//! - `GET  /api/repos[?enrich=1]` — the local scan (optionally GitHub-enriched).
//! - `GET  /api/detail?path=…&url=…` — one repo's drill-in (local + remote).
//! - `GET  /api/meta` — the roots being scanned + whether `--watch` is on.
//! - `GET  /api/overview` — the shared overview model (`cohors_core::Overview`):
//!   "Do next", key figures, one line per repo, tasks and commits. The same
//!   struct the terminal UI, the editor extension and the MCP server render.
//! - `GET  /api/launchpad` — every repo's launchpad entry + the focus board.
//! - `GET  /api/standup?window=today|week` — the author's commits across the fleet.
//! - `POST /api/action` — run a registry verb across a selector (gated by [`Caps`]).
//!
//! A tiny blocking server (`tiny_http`) on a few threads — no async, matching the
//! rest of cohors. The GitHub token stays here (server-side) and never reaches the
//! browser. Mutating verbs are gated like the MCP (ADR-025): default read-only;
//! writes need `caps.allow_writes`, `run` needs `caps.allow_run`, destructive
//! verbs additionally need `confirm:true`, and `dry_run` previews before any gate.

use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use cohors_core::RepoSnapshot;
use serde::Serialize;
use serde_json::{Value, json};
use tiny_http::{Header, Method, Response, Server};

/// A source of fleet snapshots — injected so the TUI can pass its `Scanner` and
/// the standalone binary can pass its own discovery, sharing one server.
pub type ScanFn = dyn Fn() -> Vec<RepoSnapshot> + Send + Sync;

/// Which mutating tiers the server permits, chosen at launch (ADR-025) — the
/// shared `cohors-actions` type; enforcement lives in its `dispatch`.
pub use cohors_actions::Caps;

/// Everything `POST /api/action` needs that the read endpoints don't.
#[derive(Clone)]
struct ActionCtx {
    scan: Arc<ScanFn>,
    caps: Caps,
    allowlist: Arc<Vec<String>>,
    max_targets: usize,
}

/// Byte cap for the working-tree patch served to the page — matches the MCP
/// `changes` tool's default, so the web and agent surfaces truncate alike.
const DETAIL_PATCH_BYTES: usize = 20_000;

/// Session metadata for the page: the roots being scanned and whether `--watch`
/// asked for live re-scans (so the page can poll).
#[derive(Serialize)]
struct MetaResponse {
    roots: Vec<String>,
    watch: bool,
    /// True when the server was started with `--demo`: the page says so in
    /// plain sight, so sample data is never mistaken for a real fleet.
    demo: bool,
    /// Which mutating tiers are enabled, so the page can explain a disabled
    /// button before the user clicks it rather than after.
    allow_writes: bool,
    allow_run: bool,
}

/// A root as the page shows it: `~/code` rather than the full path of the
/// home directory, the way the terminal dashboard writes it.
fn shown_root(root: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    match root.strip_prefix(home.trim_end_matches('/')) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("~{rest}")
        }
        _ => root.to_string(),
    }
}

/// Where the page's files (HTML, JavaScript, WebAssembly, CSS) come from.
#[derive(Debug, Clone)]
pub enum Assets {
    /// A directory on disk, as `trunk` builds it in a checkout of cohors.
    Dir(Utf8PathBuf),
    /// Files compiled into the binary, as `(path, bytes)` pairs with `/`
    /// between directories. A release build carries the page this way, so an
    /// installed `cohors web` needs no checkout and no toolchain.
    Embedded(&'static [(&'static str, &'static [u8])]),
}

impl Assets {
    /// The bytes of one file, or `None` when there is no such file.
    fn read(&self, rel: &str) -> Option<std::borrow::Cow<'static, [u8]>> {
        match self {
            Assets::Dir(dir) => std::fs::read(dir.join(rel).as_std_path())
                .ok()
                .map(std::borrow::Cow::Owned),
            Assets::Embedded(files) => files
                .iter()
                .find(|(path, _)| *path == rel)
                .map(|(_, bytes)| std::borrow::Cow::Borrowed(*bytes)),
        }
    }
}

/// How to run the server: everything except the assets directory and the scan.
/// A struct rather than a long argument list, so adding an option does not
/// ripple through every caller.
#[derive(Debug, Clone, Default)]
pub struct ServeOptions {
    pub port: u16,
    /// The roots being scanned, shown on the page.
    pub roots: Vec<String>,
    /// GitHub token for enrichment. Stays server-side.
    pub token: Option<String>,
    /// `--watch`: the page polls for fresh scans.
    pub watch: bool,
    pub caps: Caps,
    /// Glob patterns restricting `run` (empty = any command).
    pub allowlist: Vec<String>,
    /// Action-target cap (0 = no cap).
    pub max_targets: usize,
    /// `--demo`: the scan closure returns the sample fleet, so the sample
    /// launchpad is served to match and the page labels itself as demo data.
    pub demo: bool,
    /// Whose commits the standup lists (`git config user.email`).
    pub author_email: Option<String>,
}

/// What the read endpoints need to know about this session.
struct Site {
    roots: Vec<String>,
    token: Option<String>,
    watch: bool,
    demo: bool,
    author_email: Option<String>,
}

/// Serve the page from `assets` on `127.0.0.1:port`, exposing the local scan
/// under `/api` and the action endpoint gated by `options.caps`. Blocks until
/// the process is stopped.
pub fn serve(assets: Assets, scan: Arc<ScanFn>, options: ServeOptions) -> Result<()> {
    let server = Server::http(("127.0.0.1", options.port))
        .map_err(|e| anyhow!("could not start web server: {e}"))?;
    let server = Arc::new(server);
    let dist = Arc::new(assets);
    let action = ActionCtx {
        scan: scan.clone(),
        caps: options.caps,
        allowlist: Arc::new(options.allowlist),
        max_targets: options.max_targets,
    };
    let site = Arc::new(Site {
        roots: options.roots,
        token: options.token,
        watch: options.watch,
        demo: options.demo,
        author_email: options.author_email,
    });

    // Enough workers that a slow enrich/detail fetch (which hits GitHub) doesn't
    // stall the static assets or other API calls the page fires concurrently.
    let mut workers = Vec::new();
    for _ in 0..8 {
        let server = server.clone();
        let dist = dist.clone();
        let site = site.clone();
        let scan = scan.clone();
        let action = action.clone();
        workers.push(thread::spawn(move || {
            while let Ok(req) = server.recv() {
                handle(req, &dist, &*scan, &site, &action);
            }
        }));
    }
    for w in workers {
        let _ = w.join();
    }
    Ok(())
}

fn handle(req: tiny_http::Request, dist: &Assets, scan: &ScanFn, site: &Site, action: &ActionCtx) {
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("/");
    // Binding to 127.0.0.1 is not enough on its own: a web page on another
    // site can point a hostname it controls at 127.0.0.1 (DNS rebinding) and
    // then read these endpoints as if it were same-origin. Such a request
    // still carries the attacker's hostname in `Host`, so refuse any request
    // not addressed to this machine by a local name, and any that says it
    // comes from a foreign origin.
    let header = |name: &'static str| {
        req.headers()
            .iter()
            .find(|h| h.field.equiv(name))
            .map(|h| h.value.as_str().to_string())
    };
    if !is_local_host(header("Host").as_deref())
        || header("Origin").is_some_and(|o| !is_local_origin(&o))
    {
        let _ = req.respond(
            Response::from_string("cohors web only answers requests addressed to localhost\n")
                .with_status_code(403),
        );
        return;
    }
    let token = site.token.as_deref();
    match (req.method(), path) {
        (Method::Get, "/api/repos") => api_repos(req, &url, scan, token),
        (Method::Get, "/api/detail") => api_detail(req, &url, token),
        (Method::Get, "/api/meta") => respond_json(
            req,
            &MetaResponse {
                roots: site.roots.iter().map(|root| shown_root(root)).collect(),
                watch: site.watch,
                demo: site.demo,
                allow_writes: action.caps.allow_writes,
                allow_run: action.caps.allow_run,
            },
        ),
        (Method::Get, "/api/overview") => respond_json(req, &overview_payload(&scan(), site.demo)),
        (Method::Get, "/api/launchpad") => {
            respond_json(req, &launchpad_payload(&scan(), site.demo))
        }
        (Method::Get, "/api/standup") => {
            let window = query_param(&url, "window");
            respond_json(
                req,
                &standup_payload(&scan(), site.author_email.as_deref(), window.as_deref()),
            )
        }
        (Method::Post, "/api/action") => api_action(req, action),
        _ => serve_static(req, dist, &url),
    }
}

/// True when a `Host` header names this machine: `localhost`, any
/// `*.localhost` name (the URL `cohors web` prints is `cohors.localhost`), or a
/// loopback address — with or without a port. A missing header is refused.
fn is_local_host(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return false;
    };
    let host = host.trim().to_ascii_lowercase();
    // Drop the port: `[::1]:8080` → `[::1]`, `localhost:8080` → `localhost`.
    let name = if host.starts_with('[') {
        host.split_once(']')
            .map_or(host.as_str(), |(addr, _)| &host[..addr.len() + 1])
    } else {
        host.split(':').next().unwrap_or("")
    };
    name == "localhost" || name.ends_with(".localhost") || name == "127.0.0.1" || name == "[::1]"
}

/// True when an `Origin` header is one of this server's own local origins.
fn is_local_origin(origin: &str) -> bool {
    origin
        .strip_prefix("http://")
        .is_some_and(|rest| is_local_host(Some(rest)))
}

/// `GET /api/repos[?enrich=1]` — the local scan, optionally enriched with remote
/// CI/PRs. The browser loads the plain scan first (instant), then re-requests
/// with `enrich=1` so remote signals fill in without blocking first paint.
fn api_repos(req: tiny_http::Request, url: &str, scan: &ScanFn, token: Option<&str>) {
    let mut snapshots = scan();
    if query_param(url, "enrich").as_deref() == Some("1") {
        cohors_fleet::enrich(&mut snapshots, token);
    }
    respond_json(req, &snapshots);
}

/// `GET /api/launchpad` — what each repo offers and what is waiting in it: the
/// committed `.cohors/` contribution, flows detected from manifests, the
/// user's cross-repo focus board and recent agent activity (ADR-054).
///
/// The sample launchpad is served only when the server was started with
/// `--demo`. A real fleet with nothing set up gets an honest empty launchpad,
/// never invented tasks.
fn launchpad_payload(snapshots: &[RepoSnapshot], demo: bool) -> Value {
    if demo {
        let (entries, board) = cohors_core::launchpad::demo();
        return json!({ "entries": entries, "board": board, "recent_activity": [] });
    }
    let home = cohors_fleet::launchpad::launchpad_home();
    json!(cohors_fleet::launchpad_bundle(snapshots, home.as_deref()))
}

/// `GET /api/overview` — the one model every front-end draws (see
/// `cohors_core::overview`). Built by the fleet facade so the numbers match
/// the terminal UI, the editor extension and the MCP `overview` tool exactly.
fn overview_payload(snapshots: &[RepoSnapshot], demo: bool) -> cohors_core::Overview {
    if demo {
        let (entries, board) = cohors_core::launchpad::demo();
        return cohors_core::overview(snapshots, &entries, &board, now_secs());
    }
    let home = cohors_fleet::launchpad::launchpad_home();
    cohors_fleet::overview(snapshots, home.as_deref(), now_secs())
}

/// `GET /api/standup?window=today|week` — the configured author's commits
/// across the fleet, grouped by repo (most active first), plus the same
/// markdown digest the terminal UI copies to the clipboard. `window` defaults
/// to `week`. With no `user.email` configured the list is empty and `author`
/// is null, so the page can say why rather than show a blank.
fn standup_payload(
    snapshots: &[RepoSnapshot],
    author: Option<&str>,
    window: Option<&str>,
) -> Value {
    use cohors_core::StandupWindow;
    let window = match window {
        Some("today") => StandupWindow::Today,
        _ => StandupWindow::Week,
    };
    let (since, until) = window.range(now_secs());
    let commits = match author {
        Some(email) => cohors_fleet::fleet_commits(snapshots, email, since, until),
        None => Vec::new(),
    };
    let groups: Vec<Value> = cohors_core::group_commits(&commits)
        .into_iter()
        .map(|(repo, commits)| json!({ "repo": repo, "commits": commits }))
        .collect();
    json!({
        "window": window.label(),
        "since": since,
        "until": until,
        "author": author,
        "total": commits.len(),
        "groups": groups,
        "markdown": cohors_core::to_markdown(&commits, window),
    })
}

/// `GET /api/detail?path=…&url=…` — one repo's drill-in. `path` (the local repo
/// path) drives the local detail; `url` (the remote URL) drives the remote one.
/// The three-read composition itself lives in `cohors_fleet::detail_bundle`,
/// shared with any other surface that offers a drill-in.
fn api_detail(req: tiny_http::Request, url: &str, token: Option<&str>) {
    let path = query_param(url, "path");
    let remote_url = query_param(url, "url");
    let bundle = cohors_fleet::detail_bundle(
        path.as_deref().map(Utf8Path::new),
        remote_url.as_deref(),
        token,
        DETAIL_PATCH_BYTES,
    );
    respond_json(req, &bundle);
}

/// `POST /api/action` — run a registry verb across a selector, server-side. The
/// body is `{ verb, selector, confirm?, dry_run?, message?, command?, timeout_secs? }`.
/// Dispatch reuses the shared `cohors-actions` orchestration (the same path the
/// MCP takes), so the resolve → cap → dry-run → gate → run → audit flow is identical.
fn api_action(mut req: tiny_http::Request, ctx: &ActionCtx) {
    let mut body = String::new();
    if std::io::Read::read_to_string(req.as_reader(), &mut body).is_err() {
        return respond_json(req, &json!({ "error": "could not read request body" }));
    }
    let args: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return respond_json(req, &json!({ "error": format!("invalid JSON: {e}") })),
    };
    let verb = args.get("verb").and_then(Value::as_str).unwrap_or("");
    match run_action(verb, &args, ctx) {
        Ok(value) => respond_json(req, &value),
        Err(message) => respond_json(req, &json!({ "error": message })),
    }
}

/// Resolve and run one action verb through the shared dispatcher. The verb→
/// primitive mapping and every gate (writes tier, confirm, run allowlist) live
/// in `cohors-actions`; this surface only supplies its Caps and its hint.
fn run_action(verb: &str, args: &Value, ctx: &ActionCtx) -> Result<Value, String> {
    let snaps = (*ctx.scan)();
    cohors_actions::dispatch(
        verb,
        args,
        &snaps,
        ctx.caps,
        &ctx.allowlist,
        ctx.max_targets,
        now_secs(),
        "cohors web",
    )
}

/// Serialize `value` to JSON and respond (500 with a JSON error on failure).
fn respond_json<T: Serialize>(req: tiny_http::Request, value: &T) {
    let (status, body) = match serde_json::to_string(value) {
        Ok(json) => (200, json),
        Err(e) => (
            500,
            json!({ "error": format!("serializing response: {e}") }).to_string(),
        ),
    };
    let resp = Response::from_string(body)
        .with_status_code(status)
        .with_header(json_header());
    let _ = req.respond(resp);
}

/// Serve a static file from `dist`, defaulting `/` to `index.html` and falling
/// back to `index.html` for unknown non-asset paths (SPA routing).
fn serve_static(req: tiny_http::Request, dist: &Assets, url: &str) {
    let path = url.split('?').next().unwrap_or("/").trim_start_matches('/');
    let rel = if path.is_empty() { "index.html" } else { path };
    if rel.contains("..") {
        let _ = req.respond(Response::from_string("bad path").with_status_code(400));
        return;
    }
    // Any path that is not a file is a page route; the page handles it.
    let found = match dist.read(rel) {
        Some(bytes) => Some((bytes, rel)),
        None => dist.read("index.html").map(|bytes| (bytes, "index.html")),
    };
    match found {
        Some((bytes, name)) => {
            let _ = req.respond(Response::from_data(bytes).with_header(content_type(name)));
        }
        None => {
            let _ = req.respond(Response::from_string("not found").with_status_code(404));
        }
    }
}

/// Pull a single query-string parameter out of a request URL, percent-decoded.
/// Tiny hand-rolled parser so the server needs no URL crate.
fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

/// Decode `%XX` escapes and `+` (form-encoded space) in a query value.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn json_header() -> Header {
    Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap()
}

fn content_type(name: &str) -> Header {
    let ct = if name.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if name.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if name.ends_with(".wasm") {
        "application/wasm"
    } else if name.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if name.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    };
    Header::from_bytes(&b"Content-Type"[..], ct.as_bytes()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed clock so the demo fleet's ages stay deterministic.
    const NOW: i64 = 1_700_000_000;

    /// The demo fleet — its repos carry paths, so `resolve_targets` keeps them as
    /// action targets. The tests only hit the gate/dry-run paths, never real git.
    fn demo_fleet() -> Vec<RepoSnapshot> {
        cohors_core::demo::fleet(NOW)
    }

    fn ctx(caps: Caps) -> ActionCtx {
        ActionCtx {
            scan: Arc::new(demo_fleet),
            caps,
            allowlist: Arc::new(Vec::new()),
            max_targets: 0,
        }
    }

    #[test]
    fn write_action_is_gated_without_allow_writes() {
        let args = json!({ "verb": "fetch", "selector": { "all": true } });
        let err = run_action("fetch", &args, &ctx(Caps::default())).unwrap_err();
        assert!(err.contains("--allow-writes"), "{err}");
    }

    #[test]
    fn dry_run_previews_without_a_gate() {
        let args = json!({ "verb": "fetch", "selector": { "all": true }, "dry_run": true });
        let v = run_action("fetch", &args, &ctx(Caps::default())).unwrap();
        assert_eq!(v["dry_run"], true);
        assert!(
            v["targets"].as_u64().unwrap() >= 1,
            "preview should list targets"
        );
    }

    #[test]
    fn run_is_gated_separately_from_writes() {
        // allow_writes on, allow_run off ⇒ run still refused.
        let caps = Caps {
            allow_writes: true,
            allow_run: false,
        };
        let args = json!({ "verb": "run", "command": "echo hi", "selector": { "all": true }, "confirm": true });
        let err = run_action("run", &args, &ctx(caps)).unwrap_err();
        assert!(err.contains("--allow-run"), "{err}");
    }

    #[test]
    fn the_sample_launchpad_is_served_only_in_demo_mode() {
        let demo = launchpad_payload(&demo_fleet(), true);
        assert_eq!(demo["entries"][0]["name"], "payments");
        assert_eq!(demo["board"]["focus"][0]["repo"], "payments");
        // The same fleet without `--demo` reads the disk: its paths do not
        // exist, so nothing is configured and no task is invented.
        let real = launchpad_payload(&demo_fleet(), false);
        assert!(
            real["entries"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["configured"] == false)
        );
        // No repos at all ⇒ an empty launchpad.
        assert_eq!(launchpad_payload(&[], false)["entries"], json!([]));
    }

    #[test]
    fn overview_is_the_shared_model() {
        let o = overview_payload(&demo_fleet(), true);
        assert_eq!(o.figures.repos, demo_fleet().len());
        assert_eq!(o.repos.len(), demo_fleet().len());
        assert!(!o.actions.is_empty(), "the demo fleet has work to do");
        // An empty fleet is a valid, empty overview.
        let empty = overview_payload(&[], false);
        assert!(empty.actions.is_empty() && empty.repos.is_empty());
    }

    #[test]
    fn standup_without_an_author_is_empty_and_says_so() {
        let v = standup_payload(&demo_fleet(), None, Some("today"));
        assert_eq!(v["window"], "today");
        assert_eq!(v["author"], Value::Null);
        assert_eq!(v["total"], 0);
        assert_eq!(v["groups"], json!([]));
        assert!(v["markdown"].as_str().unwrap().contains("No commits"));
        // An unknown window falls back to the week.
        assert_eq!(
            standup_payload(&[], None, Some("nope"))["window"],
            "this week"
        );
    }

    #[test]
    fn launchpad_reads_a_real_repo() {
        let dir = std::env::temp_dir().join(format!("cohors-web-lp-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".cohors")).unwrap();
        std::fs::write(
            dir.join(".cohors/repo.json"),
            r#"{"specVersion":1,"name":"Svc","tasks":[{"id":"a","title":"Do it"}]}"#,
        )
        .unwrap();
        let mut snap = demo_fleet().remove(0);
        snap.path = Some(dir.to_str().unwrap().into());
        let payload = launchpad_payload(&[snap], false);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(payload["entries"][0]["name"], "Svc");
        assert_eq!(payload["entries"][0]["configured"], true);
        assert_eq!(
            payload["entries"][0]["contribution"]["tasks"][0]["title"],
            "Do it"
        );
    }

    #[test]
    fn only_local_hosts_and_origins_are_served() {
        for ok in [
            "localhost",
            "localhost:8080",
            "cohors.localhost:8080",
            "127.0.0.1:8080",
            "[::1]:8080",
            "LOCALHOST:8080",
        ] {
            assert!(is_local_host(Some(ok)), "{ok} should be allowed");
        }
        for bad in [
            "evil.example:8080",
            "localhost.evil.example",
            "127.0.0.1.evil.example:8080",
            "notlocalhost:8080",
            "192.168.1.5:8080",
            "",
        ] {
            assert!(!is_local_host(Some(bad)), "{bad} should be refused");
        }
        assert!(!is_local_host(None));

        assert!(is_local_origin("http://cohors.localhost:8080"));
        assert!(is_local_origin("http://127.0.0.1:8080"));
        assert!(!is_local_origin("https://evil.example"));
        assert!(!is_local_origin("http://evil.example"));
        assert!(!is_local_origin("null"));
    }

    #[test]
    fn built_in_assets_are_found_by_path() {
        static FILES: [(&str, &[u8]); 2] = [("index.html", b"<html>"), ("snippets/a.js", b"x")];
        let assets = Assets::Embedded(&FILES);
        assert_eq!(assets.read("index.html").as_deref(), Some(&b"<html>"[..]));
        assert_eq!(assets.read("snippets/a.js").as_deref(), Some(&b"x"[..]));
        assert!(assets.read("missing.js").is_none());
    }

    #[test]
    fn unknown_verb_is_rejected() {
        let args = json!({ "verb": "nuke", "selector": { "all": true } });
        assert!(run_action("nuke", &args, &ctx(Caps::default())).is_err());
    }
}

/// Write parity, web half (mirrors the MCP catalog and TUI keymap tests): the
/// browser's action bar is driven by `crate::ACTION_VERBS`, so a verb added to
/// the registry without a button here fails the build.
#[cfg(test)]
mod parity_tests {
    #[test]
    fn action_bar_matches_the_registry() {
        let verbs: Vec<&str> = cohors_actions::registry().iter().map(|d| d.verb).collect();
        assert_eq!(
            crate::ACTION_VERBS,
            verbs.as_slice(),
            "web action bar drifted from the registry"
        );
        let confirm: Vec<&str> = cohors_actions::registry()
            .iter()
            .filter(|d| d.needs_confirm)
            .map(|d| d.verb)
            .collect();
        assert_eq!(
            crate::CONFIRM_VERBS,
            confirm.as_slice(),
            "web confirm set drifted from the registry"
        );
    }
}
