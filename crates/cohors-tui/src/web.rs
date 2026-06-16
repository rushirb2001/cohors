//! The `cohors web` local server.
//!
//! It serves the built WASM dashboard from `dist/` and proxies `/gh/<path>` to
//! the GitHub REST API, injecting the machine's token (the same one the TUI
//! discovers — `gh auth token` / `$GITHUB_TOKEN`) **server-side**. So the browser
//! uses your existing GitHub login with zero setup, and the token never reaches
//! the page (no pasted token, no token in browser storage). A tiny blocking
//! server (`tiny_http`) on a few threads — no async, matching the rest of the
//! binary.

use std::sync::Arc;
use std::thread;

use anyhow::{Result, anyhow};
use camino::Utf8Path;
use tiny_http::{Header, Method, Response, Server};

const GITHUB_API: &str = "https://api.github.com";

/// Serve `dist_dir` on `127.0.0.1:port`, proxying `/gh/<path>` to GitHub with
/// `token` injected (when present). Blocks until the process is stopped.
pub fn serve(dist_dir: &Utf8Path, port: u16, token: Option<String>) -> Result<()> {
    let server = Server::http(("127.0.0.1", port))
        .map_err(|e| anyhow!("could not start web server: {e}"))?;
    let server = Arc::new(server);
    let dist = dist_dir.to_owned();
    let token = Arc::new(token);

    // A few workers so a slow GitHub proxy call doesn't stall static assets (the
    // page fires several requests at once).
    let mut workers = Vec::new();
    for _ in 0..4 {
        let server = server.clone();
        let dist = dist.clone();
        let token = token.clone();
        workers.push(thread::spawn(move || {
            while let Ok(req) = server.recv() {
                handle(req, &dist, token.as_deref());
            }
        }));
    }
    for w in workers {
        let _ = w.join();
    }
    Ok(())
}

fn handle(req: tiny_http::Request, dist: &Utf8Path, token: Option<&str>) {
    let url = req.url().to_string();
    if matches!(req.method(), Method::Get) && url.starts_with("/gh/") {
        proxy_github(req, &url, token);
    } else {
        serve_static(req, dist, &url);
    }
}

/// Proxy a GET to the GitHub REST API with the token injected.
fn proxy_github(req: tiny_http::Request, url: &str, token: Option<&str>) {
    let target = format!("{GITHUB_API}{}", &url["/gh".len()..]); // keep the leading '/'
    let mut r = ureq::get(&target)
        .set("User-Agent", "cohors-web")
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28");
    if let Some(t) = token {
        r = r.set("Authorization", &format!("Bearer {t}"));
    }
    let (status, body) = match r.call() {
        Ok(resp) => (resp.status(), resp.into_string().unwrap_or_default()),
        Err(ureq::Error::Status(code, resp)) => (code, resp.into_string().unwrap_or_default()),
        Err(e) => (
            502,
            serde_json::json!({ "message": format!("proxy error: {e}") }).to_string(),
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
