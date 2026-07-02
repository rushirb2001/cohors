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
use camino::Utf8Path;
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
}

/// Serve `dist_dir` on `127.0.0.1:port`, exposing the local scan under `/api` and
/// the action endpoint gated by `caps`. Blocks until the process is stopped.
#[allow(clippy::too_many_arguments)]
pub fn serve(
    dist_dir: &Utf8Path,
    port: u16,
    scan: Arc<ScanFn>,
    roots: Vec<String>,
    token: Option<String>,
    watch: bool,
    caps: Caps,
    allowlist: Vec<String>,
    max_targets: usize,
) -> Result<()> {
    let server = Server::http(("127.0.0.1", port))
        .map_err(|e| anyhow!("could not start web server: {e}"))?;
    let server = Arc::new(server);
    let dist = dist_dir.to_owned();
    let token = Arc::new(token);
    let roots = Arc::new(roots);
    let action = ActionCtx {
        scan: scan.clone(),
        caps,
        allowlist: Arc::new(allowlist),
        max_targets,
    };

    // Enough workers that a slow enrich/detail fetch (which hits GitHub) doesn't
    // stall the static assets or other API calls the page fires concurrently.
    let mut workers = Vec::new();
    for _ in 0..8 {
        let server = server.clone();
        let dist = dist.clone();
        let token = token.clone();
        let roots = roots.clone();
        let scan = scan.clone();
        let action = action.clone();
        workers.push(thread::spawn(move || {
            while let Ok(req) = server.recv() {
                handle(req, &dist, &*scan, &roots, token.as_deref(), watch, &action);
            }
        }));
    }
    for w in workers {
        let _ = w.join();
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle(
    req: tiny_http::Request,
    dist: &Utf8Path,
    scan: &ScanFn,
    roots: &[String],
    token: Option<&str>,
    watch: bool,
    action: &ActionCtx,
) {
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("/");
    match (req.method(), path) {
        (Method::Get, "/api/repos") => api_repos(req, &url, scan, token),
        (Method::Get, "/api/detail") => api_detail(req, &url, token),
        (Method::Get, "/api/meta") => respond_json(
            req,
            &MetaResponse {
                roots: roots.to_vec(),
                watch,
            },
        ),
        (Method::Post, "/api/action") => api_action(req, action),
        _ => serve_static(req, dist, &url),
    }
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
fn serve_static(req: tiny_http::Request, dist: &Utf8Path, url: &str) {
    let path = url.split('?').next().unwrap_or("/").trim_start_matches('/');
    let rel = if path.is_empty() { "index.html" } else { path };
    if rel.contains("..") {
        let _ = req.respond(Response::from_string("bad path").with_status_code(400));
        return;
    }
    let bytes = std::fs::read(dist.join(rel).as_std_path())
        .or_else(|_| std::fs::read(dist.join("index.html").as_std_path()));
    match bytes {
        Ok(bytes) => {
            let _ = req.respond(Response::from_data(bytes).with_header(content_type(rel)));
        }
        Err(_) => {
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
    fn unknown_verb_is_rejected() {
        let args = json!({ "verb": "nuke", "selector": { "all": true } });
        assert!(run_action("nuke", &args, &ctx(Caps::default())).is_err());
    }
}
