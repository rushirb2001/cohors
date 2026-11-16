//! The `cohors web` HTTP server — the native half of the web front-end.
//!
//! The web app is just another front-end over the **same local scan** the TUI,
//! CLI, and MCP run. The server reads the fleet from a [`Source`]: in normal
//! use the live fleet watch (`cohors_fleet::FleetWatch`), which keeps a scan
//! current in the background, so a request reads its newest state instead of
//! scanning the disk again, and `/api/events` tells the page the moment
//! something changed. The demo and the tests pass a plain scan closure instead.
//! It serves those `cohors-core` snapshots as JSON, and also *acts* on them:
//! `POST /api/action` dispatches through the shared `cohors-actions`
//! orchestration the MCP uses. The browser renders the reads through the same
//! `assess`/sort logic — your *local* fleet, not a remote account view.
//!
//! Endpoints:
//! - `GET  /api/repos[?enrich=1]` — the local scan (optionally GitHub-enriched).
//! - `GET  /api/detail?path=…&url=…` — one repo's drill-in (local + remote).
//! - `GET  /api/meta` — the roots being scanned, the enabled tiers, whether
//!   the fleet is live (has an event stream), and a newer cohors when one is
//!   known (`update`: `{version, notice, detail}`, the shape the MCP server's
//!   `meta.update` and each `cohors watch` line carry).
//! - `GET  /api/overview[?fresh=1]` — the shared overview model
//!   (`cohors_core::Overview`): "Do next", key figures, one line per repo, tasks
//!   and commits. The same struct the terminal UI, the editor extension and the
//!   MCP server render. `fresh=1` rescans first (a refresh the user asked for).
//! - `GET  /api/launchpad` — every repo's launchpad entry + the focus board.
//! - `GET  /api/next` — the suggestions (`{ list, sessions, tips }`): agent
//!   conversations to resume, "do next" steps and focus items, each with the
//!   command that carries it out, and at most one setup tip (ADR-065).
//! - `GET  /api/standup?window=today|week` — the author's commits across the fleet.
//! - `GET  /api/events` — server-sent events: one `fleet` event (with its `seq`)
//!   per change of the fleet, and a comment every few seconds to keep the
//!   connection open. `204 No Content` when the fleet is not live (the demo).
//! - `POST /api/action` — run a registry verb across a selector (gated by [`Caps`]).
//!
//! A tiny blocking server (`tiny_http`) on a few threads — no async, matching the
//! rest of cohors. An event stream gets a thread of its own, so an open page
//! never holds up another request. The GitHub token stays here (server-side)
//! and never reaches the browser. Mutating verbs are gated like the MCP
//! (ADR-025): default read-only; writes need `caps.allow_writes`, `run` needs
//! `caps.allow_run`, destructive verbs additionally need `confirm:true`, and
//! `dry_run` previews before any gate.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use cohors_core::RepoSnapshot;
use cohors_core::update::UpdateState;
use cohors_fleet::{FleetState, FleetWatch, LaunchpadBundle, Scanner, Subscription, WatchOptions};
use serde::Serialize;
use serde_json::{Value, json};
use tiny_http::{Header, Method, Response, Server};

/// A source of fleet snapshots — injected so the TUI can pass its `Scanner` and
/// the standalone binary can pass its own discovery, sharing one server.
pub type ScanFn = dyn Fn() -> Vec<RepoSnapshot> + Send + Sync;

/// Which mutating tiers the server permits, chosen at launch (ADR-025) — the
/// shared `cohors-actions` type; enforcement lives in its `dispatch`.
pub use cohors_actions::Caps;

/// Whether a newer cohors is known, asked on every `/api/meta` so a server
/// that runs for days notices a release found while it runs.
pub type UpdateFn = dyn Fn() -> Option<UpdateState> + Send + Sync;

/// The binary's answer to "is a newer cohors known?" ([`UpdateFn`]), wrapped
/// so [`ServeOptions`] can still derive `Debug` and `Clone`: a closure has no
/// `Debug` of its own, and `Arc` makes the clone a cheap shared handle.
#[derive(Clone)]
pub struct UpdateSource(pub Arc<UpdateFn>);

impl std::fmt::Debug for UpdateSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UpdateSource(..)")
    }
}

/// A newer cohors as `/api/meta` reports it, in the words every surface uses
/// ([`UpdateState::notice`]); `detail` is the state itself.
#[derive(Serialize)]
struct UpdateNotice {
    version: String,
    notice: String,
    detail: UpdateState,
}

impl From<UpdateState> for UpdateNotice {
    fn from(state: UpdateState) -> Self {
        Self {
            version: state.version().to_string(),
            notice: state.notice(),
            detail: state,
        }
    }
}

/// Where the server's fleet comes from.
pub enum Source {
    /// Scanned again for every request. For sample data (`--demo`), which
    /// never changes, and for tests.
    Scan(Arc<ScanFn>),
    /// Kept current by a fleet watch: requests read its newest state, and
    /// `/api/events` streams its changes. The scanner is kept for the rare
    /// case the watch stops (its thread died): the server then scans itself.
    Live {
        watch: Arc<FleetWatch>,
        scanner: Arc<Scanner>,
    },
}

impl Source {
    /// A live source over `scanner`, watched with the default options.
    pub fn live(scanner: Arc<Scanner>) -> Self {
        Self::live_with(scanner, WatchOptions::default())
    }

    /// [`Source::live`] with chosen watch options.
    pub fn live_with(scanner: Arc<Scanner>, options: WatchOptions) -> Self {
        let watch = Arc::new(FleetWatch::start(scanner.clone(), options));
        Self::Live { watch, scanner }
    }
}

impl From<Arc<ScanFn>> for Source {
    fn from(scan: Arc<ScanFn>) -> Self {
        Self::Scan(scan)
    }
}

/// How long a request waits for the watch's first scan, or for a rescan it
/// asked for, before it scans by itself.
const WATCH_WAIT: Duration = Duration::from_secs(60);

/// An event stream with nothing to say writes a comment this often, so a
/// closed page is noticed (the write fails) and proxies keep it open.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// At most this many event streams at once; more are refused (`503`), so a
/// runaway page cannot pile up threads. One open tab uses one.
const MAX_STREAMS: usize = 32;

/// The fleet as the request handlers see it.
struct Fleet {
    source: Source,
    demo: bool,
    /// Where the focus board and the activity log live (`~/.cohors`).
    launchpad_home: Option<PathBuf>,
    /// Where the coding agents keep their conversations (the home folder).
    /// `None` for sample data, which must not show the machine's sessions.
    user_home: Option<PathBuf>,
    /// Event streams open now.
    streams: Arc<AtomicUsize>,
    /// The setup tips, and the note that the page was opened. `None` unless
    /// [`ServeOptions::tips`] asked for them.
    tips: Option<cohors_fleet::tips::TipSource>,
}

/// The fleet as one request reads it.
enum Reading {
    Live(Arc<FleetState>),
    Scanned {
        snapshots: Vec<RepoSnapshot>,
        launchpad: LaunchpadBundle,
    },
}

impl Reading {
    fn snapshots(&self) -> &[RepoSnapshot] {
        match self {
            Reading::Live(state) => &state.snapshots,
            Reading::Scanned { snapshots, .. } => snapshots,
        }
    }

    fn launchpad(&self) -> &LaunchpadBundle {
        match self {
            Reading::Live(state) => &state.launchpad,
            Reading::Scanned { launchpad, .. } => launchpad,
        }
    }
}

impl Fleet {
    fn new(source: Source, demo: bool, tips: bool) -> Self {
        let user_home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .filter(|h| !h.is_empty())
            .map(PathBuf::from);
        Self {
            source,
            demo,
            launchpad_home: cohors_fleet::launchpad::launchpad_home(),
            user_home: if demo { None } else { user_home },
            streams: Arc::new(AtomicUsize::new(0)),
            tips: (tips && !demo)
                .then(cohors_fleet::tips::TipEnv::from_env)
                .flatten()
                .map(cohors_fleet::tips::TipSource::new),
        }
    }

    /// The fleet now. `fresh` rescans first: for a refresh the user asked for,
    /// and around an action, which must see the disk as it is this moment.
    fn read(&self, fresh: bool) -> Reading {
        match &self.source {
            Source::Live { watch, scanner } => {
                if watch.is_running() {
                    let state = if fresh {
                        watch.rescan(WATCH_WAIT)
                    } else {
                        watch.wait_latest(WATCH_WAIT)
                    };
                    if let Some(state) = state {
                        return Reading::Live(state);
                    }
                }
                // The watch is gone (or hopelessly slow): read the disk.
                let scanner = watch
                    .latest()
                    .map_or_else(|| scanner.clone(), |s| s.scanner.clone());
                self.scanned(scanner.scan())
            }
            Source::Scan(scan) => self.scanned(scan()),
        }
    }

    fn scanned(&self, snapshots: Vec<RepoSnapshot>) -> Reading {
        let launchpad = launchpad_for(&snapshots, self.demo, self.launchpad_home.as_deref());
        Reading::Scanned {
            snapshots,
            launchpad,
        }
    }

    /// The watch to stream from, if the fleet is live.
    fn watch(&self) -> Option<&Arc<FleetWatch>> {
        match &self.source {
            Source::Live { watch, .. } if watch.is_running() => Some(watch),
            _ => None,
        }
    }
}

/// Everything `POST /api/action` needs that the read endpoints don't.
#[derive(Clone)]
struct ActionCtx {
    fleet: Arc<Fleet>,
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
    /// True when the fleet is live: `/api/events` announces every change, so
    /// the page need not poll.
    live: bool,
    /// True when the server was started with `--demo`: the page says so in
    /// plain sight, so sample data is never mistaken for a real fleet.
    demo: bool,
    /// Which mutating tiers are enabled, so the page can explain a disabled
    /// button before the user clicks it rather than after.
    allow_writes: bool,
    allow_run: bool,
    /// A newer cohors, when one is known (`null` otherwise), so the page can
    /// say so in its header the way the terminal dashboard does.
    update: Option<UpdateNotice>,
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
    /// Offer a setup tip in `/api/next` and note that the page was opened
    /// (ADR-065). The real `cohors web` sets it; the demo and tests do not,
    /// so they never touch the cache folder.
    pub tips: bool,
    /// Whether a newer cohors is known, for `/api/meta`. The real `cohors
    /// web` passes what its background look remembered; the demo and the
    /// tests pass `None`, so the page never mentions updates.
    pub update: Option<UpdateSource>,
}

/// What the read endpoints need to know about this session.
struct Site {
    roots: Vec<String>,
    token: Option<String>,
    watch: bool,
    author_email: Option<String>,
    update: Option<UpdateSource>,
}

/// Serve the page from `assets` on `127.0.0.1:port`, exposing the fleet from
/// `source` under `/api` and the action endpoint gated by `options.caps`.
/// Blocks until the process is stopped.
pub fn serve(assets: Assets, source: impl Into<Source>, options: ServeOptions) -> Result<()> {
    let server = Server::http(("127.0.0.1", options.port))
        .map_err(|e| anyhow!("could not start web server: {e}"))?;
    serve_on(server, assets, source.into(), options);
    Ok(())
}

/// [`serve`] on a server that is already listening (tests bind port 0).
fn serve_on(server: Server, assets: Assets, source: Source, options: ServeOptions) {
    let server = Arc::new(server);
    let dist = Arc::new(assets);
    let fleet = Arc::new(Fleet::new(source, options.demo, options.tips));
    let action = ActionCtx {
        fleet: fleet.clone(),
        caps: options.caps,
        allowlist: Arc::new(options.allowlist),
        max_targets: options.max_targets,
    };
    let site = Arc::new(Site {
        roots: options.roots,
        token: options.token,
        watch: options.watch,
        author_email: options.author_email,
        update: options.update,
    });

    // Enough workers that a slow enrich/detail fetch (which hits GitHub) doesn't
    // stall the static assets or other API calls the page fires concurrently.
    let mut workers = Vec::new();
    for _ in 0..8 {
        let server = server.clone();
        let dist = dist.clone();
        let site = site.clone();
        let fleet = fleet.clone();
        let action = action.clone();
        workers.push(thread::spawn(move || {
            while let Ok(req) = server.recv() {
                handle(req, &dist, &fleet, &site, &action);
            }
        }));
    }
    for w in workers {
        let _ = w.join();
    }
}

fn handle(req: tiny_http::Request, dist: &Assets, fleet: &Fleet, site: &Site, action: &ActionCtx) {
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
    let fresh = query_param(&url, "fresh").as_deref() == Some("1");
    match (req.method(), path) {
        (Method::Get, "/api/repos") => api_repos(req, &url, fleet, token),
        (Method::Get, "/api/detail") => api_detail(req, &url, token),
        (Method::Get, "/api/meta") => respond_json(
            req,
            &MetaResponse {
                roots: site.roots.iter().map(|root| shown_root(root)).collect(),
                watch: site.watch,
                live: fleet.watch().is_some(),
                demo: fleet.demo,
                allow_writes: action.caps.allow_writes,
                allow_run: action.caps.allow_run,
                update: site
                    .update
                    .as_ref()
                    .and_then(|source| (source.0)())
                    .map(UpdateNotice::from),
            },
        ),
        (Method::Get, "/api/overview") => {
            let reading = fleet.read(fresh);
            respond_json(req, &overview_of(&reading, now_secs()))
        }
        (Method::Get, "/api/launchpad") => respond_json(req, fleet.read(fresh).launchpad()),
        (Method::Get, "/api/next") => respond_json(req, &next_payload(fleet, &fleet.read(fresh))),
        (Method::Get, "/api/standup") => {
            let window = query_param(&url, "window");
            respond_json(
                req,
                &standup_payload(
                    fleet.read(false).snapshots(),
                    site.author_email.as_deref(),
                    window.as_deref(),
                ),
            )
        }
        (Method::Get, "/api/events") => api_events(req, fleet),
        (Method::Post, "/api/action") => api_action(req, action),
        _ => {
            // Someone opened the page: the tip to open it is done.
            if let Some(tips) = &fleet.tips {
                tips.page_opened();
            }
            serve_static(req, dist, &url)
        }
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
fn api_repos(req: tiny_http::Request, url: &str, fleet: &Fleet, token: Option<&str>) {
    let mut snapshots = fleet.read(false).snapshots().to_vec();
    if query_param(url, "enrich").as_deref() == Some("1") {
        cohors_fleet::enrich(&mut snapshots, token);
    }
    respond_json(req, &snapshots);
}

/// What each repo offers and what is waiting in it: the committed `.cohors/`
/// contribution, flows detected from manifests, the user's cross-repo focus
/// board and recent agent activity (ADR-054). Served by `GET /api/launchpad`.
///
/// The sample launchpad is served only when the server was started with
/// `--demo`. A real fleet with nothing set up gets an honest empty launchpad,
/// never invented tasks.
fn launchpad_for(
    snapshots: &[RepoSnapshot],
    demo: bool,
    home: Option<&std::path::Path>,
) -> LaunchpadBundle {
    if demo {
        let (entries, board) = cohors_core::launchpad::demo();
        return LaunchpadBundle {
            entries,
            board,
            recent_activity: Vec::new(),
        };
    }
    cohors_fleet::launchpad_bundle(snapshots, home)
}

/// `GET /api/overview` — the one model every front-end draws (see
/// `cohors_core::overview`), from the snapshots and the launchpad read with
/// them, so the numbers match the terminal UI, the editor extension and the
/// MCP `overview` tool exactly.
fn overview_of(reading: &Reading, now: i64) -> cohors_core::Overview {
    let launchpad = reading.launchpad();
    cohors_core::overview(
        reading.snapshots(),
        &launchpad.entries,
        &launchpad.board,
        now,
    )
}

/// `GET /api/next` — the suggestions: what to pick up, with the command that
/// does it. `list` is ranked (a resume carries its `session`); `sessions` is
/// every agent conversation found in the fleet's repositories. The same list
/// `cohors watch --json` sends the editor extension.
fn next_payload(fleet: &Fleet, reading: &Reading) -> Value {
    let launchpad = reading.launchpad();
    let view = cohors_fleet::live_view(
        reading.snapshots(),
        &launchpad.entries,
        &launchpad.board,
        None,
        fleet.user_home.as_deref(),
        now_secs(),
    );
    let tips = fleet
        .tips
        .as_ref()
        .map(|t| t.for_web(now_secs()))
        .unwrap_or_default();
    json!({ "list": view.suggestions, "sessions": view.sessions, "tips": tips })
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

/// `GET /api/events` — the fleet's changes as server-sent events. The stream
/// runs on a thread of its own: `tiny_http` serves requests on a fixed set of
/// workers, and one held by an open page would starve every other request.
fn api_events(req: tiny_http::Request, fleet: &Fleet) {
    let Some(watch) = fleet.watch() else {
        // Not live: `204` tells `EventSource` not to reconnect, and the page
        // falls back to polling.
        let _ = req.respond(Response::empty(204));
        return;
    };
    if fleet.streams.fetch_add(1, Ordering::SeqCst) >= MAX_STREAMS {
        fleet.streams.fetch_sub(1, Ordering::SeqCst);
        let _ =
            req.respond(Response::from_string("too many event streams\n").with_status_code(503));
        return;
    }
    let subscription = watch.subscribe();
    let streams = fleet.streams.clone();
    let spawned = thread::Builder::new()
        .name("cohors-web-events".to_string())
        .spawn(move || {
            stream_events(req, &subscription);
            streams.fetch_sub(1, Ordering::SeqCst);
        });
    if spawned.is_err() {
        fleet.streams.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Write the event stream until the page goes away (a write fails) or the
/// watch stops. Each event carries only what changed; the page then reads the
/// endpoints it shows, all served from the watch's newest state.
fn stream_events(req: tiny_http::Request, subscription: &Subscription) {
    let mut out = req.into_writer();
    // The head is written by hand: the body has no length (it ends when the
    // connection does), and every event must leave at once, unbuffered.
    let head = "HTTP/1.1 200 OK\r\n\
                Content-Type: text/event-stream; charset=utf-8\r\n\
                Cache-Control: no-store\r\n\
                X-Content-Type-Options: nosniff\r\n\
                Connection: close\r\n\
                \r\n\
                retry: 2000\n\n";
    if out
        .write_all(head.as_bytes())
        .and_then(|()| out.flush())
        .is_err()
    {
        return;
    }
    loop {
        let chunk = match subscription.recv_timeout(KEEP_ALIVE) {
            Ok(state) => {
                let data = json!({
                    "seq": state.seq,
                    "reason": state.reason,
                    "changed": state.changed,
                });
                format!("id: {}\nevent: fleet\ndata: {data}\n\n", state.seq)
            }
            Err(RecvTimeoutError::Timeout) => ": still here\n\n".to_string(),
            Err(RecvTimeoutError::Disconnected) => return,
        };
        if out
            .write_all(chunk.as_bytes())
            .and_then(|()| out.flush())
            .is_err()
        {
            return;
        }
    }
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
///
/// Targets are resolved against a fresh scan (an action must act on the disk
/// as it is), and a real run is followed by another, so the page's next read
/// already shows what the action did.
fn run_action(verb: &str, args: &Value, ctx: &ActionCtx) -> Result<Value, String> {
    let reading = ctx.fleet.read(true);
    let result = cohors_actions::dispatch(
        verb,
        args,
        reading.snapshots(),
        ctx.caps,
        &ctx.allowlist,
        ctx.max_targets,
        now_secs(),
        "cohors web",
    );
    let dry_run = args.get("dry_run").and_then(Value::as_bool) == Some(true);
    if result.is_ok()
        && !dry_run
        && let Some(watch) = ctx.fleet.watch()
    {
        let _ = watch.rescan(WATCH_WAIT);
    }
    result
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
        let scan: Arc<ScanFn> = Arc::new(demo_fleet);
        ActionCtx {
            fleet: Arc::new(Fleet::new(Source::from(scan), true, false)),
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
        let launchpad =
            |snapshots: &[RepoSnapshot], demo| json!(launchpad_for(snapshots, demo, None));
        let demo = launchpad(&demo_fleet(), true);
        assert_eq!(demo["entries"][0]["name"], "payments");
        assert_eq!(demo["board"]["focus"][0]["repo"], "payments");
        // The same fleet without `--demo` reads the disk: its paths do not
        // exist, so nothing is configured and no task is invented.
        let real = launchpad(&demo_fleet(), false);
        assert!(
            real["entries"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["configured"] == false)
        );
        // No repos at all ⇒ an empty launchpad.
        assert_eq!(launchpad(&[], false)["entries"], json!([]));
    }

    #[test]
    fn overview_is_the_shared_model() {
        let scan: Arc<ScanFn> = Arc::new(demo_fleet);
        let o = overview_of(
            &Fleet::new(Source::from(scan), true, false).read(false),
            NOW,
        );
        assert_eq!(o.figures.repos, demo_fleet().len());
        assert_eq!(o.repos.len(), demo_fleet().len());
        assert!(!o.actions.is_empty(), "the demo fleet has work to do");
        // An empty fleet is a valid, empty overview.
        let none: Arc<ScanFn> = Arc::new(Vec::new);
        let empty = overview_of(
            &Fleet::new(Source::from(none), false, false).read(false),
            NOW,
        );
        assert!(empty.actions.is_empty() && empty.repos.is_empty());
    }

    #[test]
    fn tips_are_off_unless_asked_for_and_never_with_sample_data() {
        let scan: Arc<ScanFn> = Arc::new(demo_fleet);
        assert!(
            Fleet::new(Source::from(scan.clone()), false, false)
                .tips
                .is_none()
        );
        // Sample data never reads or writes this machine's tips.
        assert!(
            Fleet::new(Source::from(scan.clone()), true, true)
                .tips
                .is_none()
        );
        let fleet = Fleet::new(Source::from(scan), true, false);
        let next = next_payload(&fleet, &fleet.read(false));
        assert_eq!(next["tips"], json!([]));
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
        let payload = json!(launchpad_for(&[snap], false, None));
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

/// The live fleet over HTTP: `/api/events` announces each change, an open
/// stream holds up no other request, and the local-only checks guard it too.
#[cfg(test)]
mod live_tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::process::Command;
    use std::time::Instant;

    use super::*;

    const WAIT: Duration = Duration::from_secs(20);

    fn git(dir: &std::path::Path, args: &[&str]) {
        let out = Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    /// One repo under `<tmp>/code`, and a config naming that root.
    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Arc<Scanner>) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let repo = root.join("code/alpha");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["-c", "init.defaultBranch=main", "init", "-q"]);
        git(&repo, &["config", "user.email", "dev@example.com"]);
        git(&repo, &["config", "user.name", "Dev"]);
        std::fs::write(repo.join("README.md"), "hi").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let config = root.join("config.toml");
        std::fs::write(&config, format!("roots = [\"{}/code\"]\n", root.display())).unwrap();
        let config = Utf8PathBuf::from_path_buf(config).unwrap();
        let scanner = Scanner::new(Some(&config), &[])
            .unwrap()
            .with_known_file(None);
        (tmp, repo, Arc::new(scanner))
    }

    /// Serve `source` on a free port; returns the port.
    fn start(source: Source) -> u16 {
        let server = Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        thread::spawn(move || {
            serve_on(
                server,
                Assets::Embedded(&[]),
                source,
                ServeOptions::default(),
            )
        });
        port
    }

    fn connect(port: u16) -> TcpStream {
        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(WAIT)).unwrap();
        stream
    }

    /// One request on its own connection: the status and the body.
    fn get(port: u16, path: &str, extra: &str) -> (u16, String) {
        let mut stream = connect(port);
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\n{extra}Connection: close\r\n\r\n"
        )
        .unwrap();
        let mut text = String::new();
        stream.read_to_string(&mut text).unwrap();
        let status = text.split(' ').nth(1).unwrap_or("0").parse().unwrap();
        let body = text
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        (status, body)
    }

    /// Open `/api/events` and read up to the end of its head.
    fn open_events(port: u16) -> (TcpStream, String) {
        let mut stream = connect(port);
        write!(
            stream,
            "GET /api/events HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n"
        )
        .unwrap();
        let head = read_until(&mut stream, &mut String::new(), "\r\n\r\n");
        (stream, head)
    }

    /// Read from `stream` into `seen` until it holds `pattern`; return and
    /// consume everything up to and including it.
    fn read_until(stream: &mut TcpStream, seen: &mut String, pattern: &str) -> String {
        let deadline = Instant::now() + WAIT;
        let mut buf = [0u8; 4096];
        loop {
            if let Some(at) = seen.find(pattern) {
                let rest = seen.split_off(at + pattern.len());
                return std::mem::replace(seen, rest);
            }
            assert!(Instant::now() < deadline, "no {pattern:?} in {seen:?}");
            let n = stream.read(&mut buf).expect("the stream stays open");
            assert!(n > 0, "the stream closed; got {seen:?}");
            seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }

    /// The `seq` of the next `fleet` event on the stream.
    fn next_event(stream: &mut TcpStream, seen: &mut String) -> u64 {
        let _ = read_until(stream, seen, "event: fleet\ndata: ");
        let data = read_until(stream, seen, "\n\n");
        let value: Value = serde_json::from_str(data.trim()).unwrap();
        value["seq"].as_u64().unwrap()
    }

    #[test]
    fn events_announce_each_change_and_hold_up_no_other_request() {
        let (_tmp, repo, scanner) = fixture();
        let options = WatchOptions {
            probe_every: Duration::from_millis(50),
            rescan_every: Duration::from_secs(600),
            rescan_at_most: Duration::from_secs(600),
            launchpad_home: None,
        };
        let port = start(Source::live_with(scanner, options));

        let (mut stream, head) = open_events(port);
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(head.contains("text/event-stream"), "{head}");
        let mut seen = String::new();
        assert_eq!(
            next_event(&mut stream, &mut seen),
            1,
            "the current state comes first"
        );

        // More open streams than the server has workers: requests still go
        // through, because each stream has a thread of its own.
        let others: Vec<_> = (0..10).map(|_| open_events(port)).collect();
        let asked = Instant::now();
        let (status, meta) = get(port, "/api/meta", "");
        assert_eq!(status, 200);
        assert_eq!(serde_json::from_str::<Value>(&meta).unwrap()["live"], true);
        let (status, overview) = get(port, "/api/overview", "");
        assert_eq!(status, 200);
        let overview: Value = serde_json::from_str(&overview).unwrap();
        assert_eq!(overview["figures"]["repos"], 1);
        assert!(
            asked.elapsed() < Duration::from_secs(5),
            "requests waited on the streams"
        );
        drop(others);

        // A commit: an event, and the next read shows it.
        std::fs::write(repo.join("new.txt"), "x").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "second"]);
        let deadline = Instant::now() + WAIT;
        loop {
            assert!(next_event(&mut stream, &mut seen) >= 2);
            let (_, body) = get(port, "/api/overview", "");
            let overview: Value = serde_json::from_str(&body).unwrap();
            if overview["repos"][0]["commit"]["summary"] == "second" {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the commit never showed: {overview}"
            );
        }

        // The suggestions, from the same state.
        let (status, next) = get(port, "/api/next", "");
        assert_eq!(status, 200);
        let next: Value = serde_json::from_str(&next).unwrap();
        assert!(
            next["list"].is_array() && next["sessions"].is_array(),
            "{next}"
        );

        // A page elsewhere cannot open the stream.
        let (status, _) = get(port, "/api/events", "Origin: http://evil.example\r\n");
        assert_eq!(status, 403);
        let mut stream = connect(port);
        write!(
            stream,
            "GET /api/events HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut text = String::new();
        stream.read_to_string(&mut text).unwrap();
        assert!(text.starts_with("HTTP/1.1 403"), "{text}");
    }

    #[test]
    fn sample_data_has_no_event_stream() {
        let scan: Arc<ScanFn> = Arc::new(|| cohors_core::demo::fleet(1_700_000_000));
        let port = start(Source::from(scan));
        let (status, _) = get(port, "/api/events", "");
        assert_eq!(status, 204, "EventSource stops on 204 and the page polls");
        let (_, meta) = get(port, "/api/meta", "");
        assert_eq!(serde_json::from_str::<Value>(&meta).unwrap()["live"], false);
    }

    #[test]
    fn meta_says_when_a_newer_cohors_is_out_in_cohors_words() {
        let scan: Arc<ScanFn> = Arc::new(|| cohors_core::demo::fleet(1_700_000_000));
        let state = UpdateState::Available {
            version: "9.9.9".to_string(),
            command: "cohors update".to_string(),
        };
        let said = state.clone();
        let options = ServeOptions {
            update: Some(UpdateSource(Arc::new(move || Some(said.clone())))),
            ..ServeOptions::default()
        };
        let server = Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let source = Source::from(scan.clone());
        thread::spawn(move || serve_on(server, Assets::Embedded(&[]), source, options));

        let (status, meta) = get(port, "/api/meta", "");
        assert_eq!(status, 200);
        let meta: Value = serde_json::from_str(&meta).unwrap();
        assert_eq!(meta["update"]["version"], "9.9.9");
        // The words the terminal and the MCP server use, worked out here.
        assert_eq!(meta["update"]["notice"], state.notice());
        assert_eq!(meta["update"]["detail"]["command"], "cohors update");

        // With nothing to report (the demo, the tests): `null`.
        let port = start(Source::from(scan));
        let (_, meta) = get(port, "/api/meta", "");
        assert!(serde_json::from_str::<Value>(&meta).unwrap()["update"].is_null());
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
