//! The `cohors web` local server.
//!
//! The web app is just another front-end over the **same local scan** the TUI,
//! CLI, and MCP run. This server discovers the repos under `--root`/config,
//! snapshots their local state (and, with a token, enriches with remote CI/PRs),
//! and serves those `cohors-core` snapshots as JSON. The browser renders them
//! through the same `assess`/sort logic — so the page shows your *local* fleet
//! (why each repo needs you), not a remote GitHub account view.
//!
//! Endpoints:
//! - `GET /api/repos` — the local scan (fast; remote left empty).
//! - `GET /api/repos?enrich=1` — the scan enriched with GitHub CI/PRs.
//! - `GET /api/detail?path=…&url=…` — one repo's drill-in: the local detail
//!   (recent commits, changed files, branches, stashes) plus remote detail.
//!
//! A tiny blocking server (`tiny_http`) on a few threads — no async, matching the
//! rest of the binary. The GitHub token stays here (server-side) and never
//! reaches the browser.

use std::sync::Arc;
use std::thread;

use anyhow::{Result, anyhow};
use camino::Utf8Path;
use serde::Serialize;
use tiny_http::{Header, Method, Response, Server};

use crate::scan::Scanner;

/// One repo's drill-in detail: the local view (the TUI's `Enter` pane), the
/// working-tree changes (file list + a size-capped patch), plus the remote view
/// (open PRs, contributors, issues, latest release).
#[derive(Serialize)]
struct DetailResponse {
    local: cohors_core::RepoDetail,
    changes: cohors_core::RepoChanges,
    remote: Option<cohors_core::RemoteDetail>,
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

/// Serve `dist_dir` on `127.0.0.1:port`, exposing the local scan under `/api`.
/// Blocks until the process is stopped.
pub fn serve(
    dist_dir: &Utf8Path,
    port: u16,
    scanner: Arc<Scanner>,
    token: Option<String>,
    watch: bool,
) -> Result<()> {
    let server = Server::http(("127.0.0.1", port))
        .map_err(|e| anyhow!("could not start web server: {e}"))?;
    let server = Arc::new(server);
    let dist = dist_dir.to_owned();
    let token = Arc::new(token);

    // Enough workers that a slow enrich/detail fetch (which hits GitHub) doesn't
    // stall the static assets or other API calls the page fires concurrently.
    let mut workers = Vec::new();
    for _ in 0..8 {
        let server = server.clone();
        let dist = dist.clone();
        let token = token.clone();
        let scanner = scanner.clone();
        workers.push(thread::spawn(move || {
            while let Ok(req) = server.recv() {
                handle(req, &dist, &scanner, token.as_deref(), watch);
            }
        }));
    }
    for w in workers {
        let _ = w.join();
    }
    Ok(())
}

fn handle(
    req: tiny_http::Request,
    dist: &Utf8Path,
    scanner: &Scanner,
    token: Option<&str>,
    watch: bool,
) {
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("/");
    match (req.method(), path) {
        (Method::Get, "/api/repos") => api_repos(req, &url, scanner, token),
        (Method::Get, "/api/detail") => api_detail(req, &url, token),
        (Method::Get, "/api/meta") => respond_json(
            req,
            &MetaResponse {
                roots: scanner.roots(),
                watch,
            },
        ),
        _ => serve_static(req, dist, &url),
    }
}

/// `GET /api/repos[?enrich=1]` — the local scan, optionally enriched with remote
/// CI/PRs. The browser loads the plain scan first (instant), then re-requests
/// with `enrich=1` so remote signals fill in without blocking first paint.
fn api_repos(req: tiny_http::Request, url: &str, scanner: &Scanner, token: Option<&str>) {
    let mut snapshots = scanner.scan();
    if query_param(url, "enrich").as_deref() == Some("1") {
        cohors_github::enrich(&mut snapshots, token);
    }
    respond_json(req, &snapshots);
}

/// `GET /api/detail?path=…&url=…` — one repo's drill-in. `path` (the local repo
/// path) drives the local detail; `url` (the remote URL) drives the remote one.
fn api_detail(req: tiny_http::Request, url: &str, token: Option<&str>) {
    let (local, changes) = match query_param(url, "path") {
        Some(p) => {
            let path = Utf8Path::new(&p);
            // Two local reads of the same repo: the drill-in (commits/branches/…)
            // and the working-tree diff (with the patch, so the drawer can show it).
            (
                cohors_git::repo_detail(path),
                cohors_git::repo_changes(path, true, DETAIL_PATCH_BYTES),
            )
        }
        None => (
            cohors_core::RepoDetail::default(),
            cohors_core::RepoChanges::default(),
        ),
    };
    let remote = match (token, query_param(url, "url")) {
        (Some(t), Some(remote_url)) if !remote_url.is_empty() => {
            cohors_github::fetch_repo_detail(t, &remote_url)
        }
        _ => None,
    };
    respond_json(
        req,
        &DetailResponse {
            local,
            changes,
            remote,
        },
    );
}

/// Serialize `value` to JSON and respond (500 with a JSON error on failure).
fn respond_json<T: Serialize>(req: tiny_http::Request, value: &T) {
    let (status, body) = match serde_json::to_string(value) {
        Ok(json) => (200, json),
        Err(e) => (
            500,
            serde_json::json!({ "error": format!("serializing response: {e}") }).to_string(),
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
