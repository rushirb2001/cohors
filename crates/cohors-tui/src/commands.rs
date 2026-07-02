//! Implementations of the CLI subcommands.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use cohors_core::{Selector, SortMode};

use crate::cli::Cli;

/// `cohors init` — write a starter config and print its path. Auto-detects where
/// the user keeps repos and seeds `roots` with them, so the first `cohors` run
/// shows a populated fleet instead of an empty one.
pub fn init(cli: &Cli, force: bool) -> Result<()> {
    let path: Utf8PathBuf = match &cli.config {
        Some(p) => Utf8PathBuf::from(p),
        None => cohors_config::paths::config_file().context("resolving the config path")?,
    };
    let home = cohors_config::paths::home_dir().ok();
    let roots = cohors_fleet::detect_roots(home.as_deref());
    cohors_config::write_starter(&path, force, &roots)
        .with_context(|| format!("writing config to {path}"))?;
    println!("Wrote starter config to {path}");
    if roots.is_empty() {
        println!("No repos auto-detected — edit `roots` to point at your code, then run `cohors`.");
    } else {
        println!("Detected roots: {}", roots.join(", "));
        println!("Run `cohors` to see your fleet (edit `roots` to adjust).");
    }
    Ok(())
}

/// `cohors scan` — discover + snapshot all repos and print JSON to stdout.
///
/// With `--select`, only the matching repos are printed, in dirty-first order,
/// using the same [`cohors_core::resolve`] the dashboard and (later) the MCP
/// server use — so `scan --select behind` and `list_repos({behind:true})` agree.
pub fn scan(cli: &Cli, select: Option<&str>) -> Result<()> {
    let scanner = crate::scan::from_cli(cli)?;
    let mut snapshots = scanner.scan();

    // An empty fleet is cryptic on the scriptable surface. Keep stdout a clean
    // `[]` (the JSON contract), but nudge a human via stderr so they aren't left
    // guessing why nothing came back. Scripts piping `scan` ignore stderr.
    if snapshots.is_empty() {
        eprintln!(
            "cohors: no git repositories found under {}.\n  Run `cohors init` to detect your repos, or pass --root <dir>.",
            scanner.roots().join(", ")
        );
    }

    if let Some(query) = select {
        let selector = parse_selector(query)?;
        let order = cohors_core::resolve(&snapshots, &selector, SortMode::DirtyFirst, now_secs());
        // Reorder/filter the snapshots to the resolved set, preserving order.
        let by_id: HashMap<&str, usize> = snapshots
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.0.as_str(), i))
            .collect();
        let picked: Vec<usize> = order
            .iter()
            .filter_map(|id| by_id.get(id.0.as_str()).copied())
            .collect();
        snapshots = picked.into_iter().map(|i| snapshots[i].clone()).collect();
    }

    let json = serde_json::to_string_pretty(&snapshots).context("serializing snapshots")?;
    println!("{json}");
    Ok(())
}

/// A bulk git action issued from the CLI. Mirrors the TUI verbs and the MCP
/// action tools — one core (`crate::action`), three surfaces.
pub enum CliAction {
    Fetch,
    Pull,
    Push,
    Commit(String),
    Stash,
}

impl CliAction {
    /// The verb used in the human-facing summary lines.
    fn verb(&self) -> &'static str {
        match self {
            CliAction::Fetch => "fetch",
            CliAction::Pull => "pull",
            CliAction::Push => "push",
            CliAction::Commit(_) => "commit",
            CliAction::Stash => "stash",
        }
    }
}

/// Resolve a `--select` query to the actionable repos: the matching, readable
/// repos in dirty-first order, using the same [`cohors_core::resolve`] the
/// dashboard and MCP use — so `cohors push --select behind` hits exactly what
/// `scan --select behind` lists.
fn action_targets<'a>(
    snaps: &'a [cohors_core::RepoSnapshot],
    select: &str,
) -> Result<Vec<&'a cohors_core::RepoSnapshot>> {
    let selector = parse_selector(select)?;
    let order = cohors_core::resolve(snaps, &selector, SortMode::DirtyFirst, now_secs());
    let by_id: HashMap<&str, &cohors_core::RepoSnapshot> =
        snaps.iter().map(|s| (s.id.0.as_str(), s)).collect();
    Ok(order
        .iter()
        .filter_map(|id| by_id.get(id.0.as_str()).copied())
        .filter(|s| !s.has_error() && s.path.is_some())
        .collect())
}

/// `cohors fetch|pull|push|commit|stash --select <q>` — run a bulk git action
/// across the matching repos. The human typing the command is the consent, so
/// there are no capability flags here (unlike the agent-facing MCP server);
/// `--dry-run` previews the targets without acting. Safety still holds in the
/// action layer: pull is ff-only, push never force-pushes, stash/commit can't
/// lose work.
pub fn run_action(cli: &Cli, action: CliAction, select: &str, dry_run: bool) -> Result<()> {
    let scanner = crate::scan::from_cli(cli)?;
    let snapshots = scanner.scan();
    let targets = action_targets(&snapshots, select)?;
    let verb = action.verb();

    if targets.is_empty() {
        eprintln!("cohors: no repos match `{select}`.");
        return Ok(());
    }
    if dry_run {
        println!("Would {verb} {} repo(s):", targets.len());
        for s in &targets {
            println!("  {}  {}", s.name, s.path.as_ref().unwrap());
        }
        return Ok(());
    }

    let mut ok = 0usize;
    for s in &targets {
        let path = s.path.as_ref().unwrap();
        let result = match &action {
            CliAction::Fetch => crate::action::fetch(path, &s.name),
            CliAction::Pull => crate::action::pull_ff(path, &s.name),
            CliAction::Push => crate::action::push(path, &s.name),
            CliAction::Commit(message) => crate::action::commit(path, &s.name, message),
            CliAction::Stash => crate::action::stash_push(path, &s.name),
        };
        match result {
            Ok(message) => {
                ok += 1;
                println!("  ✓ {message}");
            }
            Err(message) => println!("  ✗ {message}"),
        }
    }
    println!("{verb}: {ok}/{} ok", targets.len());
    Ok(())
}

/// `cohors run <command> --select <q>` — run a shell command in each matching
/// repo, bounded per-repo by `timeout` seconds, printing each repo's output and
/// a pass/fail summary. `--dry-run` previews the targets without running.
pub fn run_command_action(
    cli: &Cli,
    select: &str,
    command: &str,
    timeout: u64,
    dry_run: bool,
) -> Result<()> {
    let scanner = crate::scan::from_cli(cli)?;
    let snapshots = scanner.scan();
    let targets = action_targets(&snapshots, select)?;

    if targets.is_empty() {
        eprintln!("cohors: no repos match `{select}`.");
        return Ok(());
    }
    if dry_run {
        println!("Would run `{command}` in {} repo(s):", targets.len());
        for s in &targets {
            println!("  {}  {}", s.name, s.path.as_ref().unwrap());
        }
        return Ok(());
    }

    let timeout = std::time::Duration::from_secs(timeout.max(1));
    // The shared bounded-pool runner (ADR-020) executes the targets concurrently
    // and returns results in target order, so we zip them back to print by name.
    let owned: Vec<cohors_core::RepoSnapshot> = targets.iter().map(|s| (*s).clone()).collect();
    let results = cohors_actions::run_each(&owned, command, timeout);
    let mut ok = 0usize;
    for (s, r) in targets.iter().zip(&results) {
        if r.ok {
            ok += 1;
        }
        let tag = if r.timed_out {
            "timed out"
        } else if r.ok {
            "ok"
        } else {
            "fail"
        };
        println!("── {} ({tag})", s.name);
        if !r.stdout.trim().is_empty() {
            print!("{}", r.stdout);
        }
        if !r.stderr.trim().is_empty() {
            eprint!("{}", r.stderr);
        }
    }
    println!("run: {ok}/{} ok", targets.len());
    Ok(())
}

/// Current Unix time in seconds (for the clock-dependent `attention` predicate).
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse a `--select` value into a [`Selector`]. A value starting with `{` is
/// JSON; anything else is the shared shorthand (parsed in `cohors-core`, so
/// every surface speaks the same language). `~` expansion reads `$HOME` here —
/// an environment concern that stays out of core.
fn parse_selector(query: &str) -> Result<Selector> {
    let query = query.trim();
    let mut selector = if query.starts_with('{') {
        serde_json::from_str::<Selector>(query).context("parsing --select JSON")?
    } else {
        cohors_core::parse_shorthand(query)?
    };
    cohors_core::expand_selector_tilde(&mut selector, std::env::var("HOME").ok().as_deref());
    Ok(selector)
}

/// The branded local host the dashboard is served at. `*.localhost` is reserved
/// loopback (RFC 6761) — modern browsers resolve it to 127.0.0.1 with no
/// `/etc/hosts` edit and no privileges — so `http://cohors.localhost:<port>` is a
/// clean, device-local URL that just works.
const WEB_HOST: &str = "cohors.localhost";

/// `cohors web` — one command to build, serve, and open the web dashboard.
///
/// It locates the `cohors-web` crate, ensures Trunk (the WASM bundler) is
/// installed, starts the dev server bound to loopback, waits until it's actually
/// accepting connections, then prints + opens the branded local URL
/// (`http://cohors.localhost:<port>`). Blocks until Ctrl-C. Must run from inside
/// the cohors repository, since Trunk builds the app from source.
pub fn run_web(
    cli: &Cli,
    port: u16,
    open: bool,
    install: bool,
    allow_writes: bool,
    allow_run: bool,
) -> Result<()> {
    // `cohors web` builds the dashboard from source, so it needs the repo. This
    // is the *developer* path. End users won't run this at all: once the
    // dashboard is deployed (v0.5 slice 4), an installed `cohors web` outside a
    // checkout will simply open the hosted URL — no local build, no Trunk, no
    // Cargo. So Trunk is a dev-only dependency, never something a distributed
    // binary must ship (TODO: wire the hosted-URL branch when we deploy).
    let web_dir = find_web_crate().context(
        "couldn't find `crates/cohors-web` — `cohors web` builds the dashboard from source, so \
         run it inside the cohors repository (a hosted version arrives with the deploy milestone)",
    )?;
    ensure_trunk(install)?;

    // If the requested port is busy (e.g. another `cohors web`), step to the next
    // free one rather than failing.
    let chosen = pick_port(port);
    if chosen != port {
        eprintln!("port {port} is in use — serving on {chosen} instead.");
    }
    let port = chosen;

    // The web app is just another front-end over the SAME local scan the TUI,
    // CLI, and MCP run: discover the repos under `--root`/config, snapshot their
    // local state, and (with a token) enrich with remote CI/PRs. The server does
    // the scan and serves the `cohors-core` snapshots as JSON; the browser
    // renders them through the same `assess`/sort logic. The token is the SAME
    // one the TUI uses (`gh auth token` / `$GITHUB_TOKEN`) and never leaves here.
    let scanner = std::sync::Arc::new(crate::scan::from_cli(cli)?);
    let token = scanner.github_token();

    // Build the WASM assets to dist/ and keep watching for rebuilds while we
    // serve. (Our own server — not `trunk serve` — so we can proxy GitHub.)
    println!("Building the dashboard…");
    let mut watcher = std::process::Command::new("trunk")
        .arg("watch")
        .current_dir(&web_dir)
        .spawn()
        .context("starting `trunk watch` (is Trunk installed and on PATH?)")?;

    let dist = web_dir.join("dist");
    if !wait_for_file(
        &dist.join("index.html"),
        std::time::Duration::from_secs(240),
    ) {
        let _ = watcher.kill();
        bail!("the WASM build didn't finish in time");
    }

    let url = format!("http://{WEB_HOST}:{port}");
    let roots = scanner.roots().join(", ");
    let auth = if token.is_some() {
        "remote CI/PRs enriched with your GitHub login"
    } else {
        "no GitHub login found — local status only (run `gh auth login` for CI/PRs)"
    };
    println!("\n  cohors web is live → {url}\n  scanning {roots}\n  {auth}\n  Ctrl-C to stop.\n");
    if open {
        let url = url.clone();
        std::thread::spawn(move || {
            if wait_for_port(port, std::time::Duration::from_secs(15)) {
                let _ = open_url(&url);
            }
        });
    }

    // Serve until stopped; then tear down the watcher. `--watch` makes the page
    // poll `/api/repos` so a fresh scan shows up without a manual rescan. The
    // server lives in `cohors-web` now (the write seam is no longer trapped in
    // this binary); we hand it a scan closure + the shared safety config.
    let mcp_config = scanner.mcp_config();
    let roots = scanner.roots();
    let caps = cohors_web::Caps {
        allow_writes,
        allow_run,
    };
    let scan: std::sync::Arc<cohors_web::ScanFn> = {
        let scanner = scanner.clone();
        std::sync::Arc::new(move || scanner.scan())
    };
    let result = cohors_web::serve(
        &dist,
        port,
        scan,
        roots,
        token,
        cli.watch,
        caps,
        mcp_config.run_allowlist,
        mcp_config.max_action_targets,
    );
    let _ = watcher.kill();
    result
}

/// Make sure Trunk (the WASM bundler) is available, installing it via Cargo when
/// missing (unless `install` is false). Falls back to pointing at a binary
/// install when Cargo isn't present (a prebuilt-binary, non-Rust setup).
fn ensure_trunk(install: bool) -> Result<()> {
    if trunk_available() {
        return Ok(());
    }
    if !install {
        bail!(
            "the web app needs Trunk (the WASM bundler). Install it with:\n\
             \n    cargo install trunk      # or: brew install trunk\n\n\
             …or just run `cohors web` (it installs Trunk for you unless you pass --no-install)."
        );
    }
    if cargo_available() {
        eprintln!(
            "Trunk (the WASM bundler the web app needs) isn't installed — installing it with \
             `cargo install trunk` (one-time, a few minutes)…"
        );
        let status = std::process::Command::new("cargo")
            .args(["install", "trunk"])
            .status()
            .context("running `cargo install trunk`")?;
        if !status.success() {
            bail!(
                "`cargo install trunk` failed — install it manually (`brew install trunk`), then retry"
            );
        }
        Ok(())
    } else {
        bail!(
            "the web app needs Trunk, and Cargo isn't available to install it automatically.\n\
             Install Trunk with `brew install trunk` (or see https://trunkrs.dev), then re-run `cohors web`."
        )
    }
}

/// The first free loopback port at or after `preferred` (scanning a small range),
/// so a busy port doesn't sink `cohors web`. Falls back to `preferred` if none
/// in range bind (then the server reports the real error).
fn pick_port(preferred: u16) -> u16 {
    (preferred..preferred.saturating_add(20))
        .find(|&p| std::net::TcpListener::bind(("127.0.0.1", p)).is_ok())
        .unwrap_or(preferred)
}

/// Poll until `path` exists, or the timeout elapses (waiting for Trunk's first build).
fn wait_for_file(path: &Utf8Path, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    false
}

/// Walk up from the current directory to find the `cohors-web` crate.
fn find_web_crate() -> Option<Utf8PathBuf> {
    let cwd = Utf8PathBuf::from_path_buf(std::env::current_dir().ok()?).ok()?;
    cwd.ancestors()
        .map(|d| d.join("crates/cohors-web"))
        .find(|d| d.join("Cargo.toml").exists())
}

/// Is the `trunk` CLI on PATH?
fn trunk_available() -> bool {
    std::process::Command::new("trunk")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Is `cargo` on PATH? (Used to decide whether we can auto-install Trunk, or must
/// point the user at a binary install instead — e.g. a non-Rust, prebuilt-binary
/// distribution.)
fn cargo_available() -> bool {
    std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Poll loopback `port` until it accepts a TCP connection (the server is up), or
/// the timeout elapses. Used to open the browser only once the page will load.
fn wait_for_port(port: u16, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    false
}

/// Open a URL in the user's default browser (best-effort, non-blocking).
fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url).spawn().map(|_| ())
}

/// Bare `cohors` — launch the interactive dashboard.
pub fn run_tui(cli: &Cli) -> Result<()> {
    let scanner = Arc::new(crate::scan::from_cli(cli)?);
    crate::tui::run(scanner, cli, !cli.no_cache, cli.watch)
}

/// `cohors demo` — launch the dashboard on a built-in sample fleet. No config,
/// no scanning, no disk access; a zero-setup way to try the tool.
pub fn run_demo() -> Result<()> {
    crate::tui::run_demo()
}

/// `cohors mcp` — speak the Model Context Protocol over stdio. Read-only unless
/// the matching `--allow-*` flags are passed (none have an effect yet — only the
/// read tools are implemented).
pub fn run_mcp(cli: &Cli, allow_writes: bool, allow_run: bool, allow_open: bool) -> Result<()> {
    let scanner = crate::scan::from_cli(cli)?;
    let caps = cohors_mcp::Caps {
        allow_writes,
        allow_run,
        allow_open,
    };
    let roots = scanner.roots();
    let config_path = scanner.config_path();
    let token = scanner.github_token();
    let mcp_config = scanner.mcp_config();
    let scan = || scanner.scan();
    cohors_mcp::serve_stdio(
        &scan,
        token.as_deref(),
        &roots,
        &config_path,
        caps,
        &mcp_config.run_allowlist,
        mcp_config.max_action_targets,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_passthrough() {
        let sel = parse_selector(r#"{"behind": true, "name": "api"}"#).unwrap();
        assert!(sel.behind);
        assert_eq!(sel.name.as_deref(), Some("api"));
    }

    #[test]
    fn unknown_token_errors() {
        assert!(parse_selector("frobnicate").is_err());
        assert!(parse_selector("ci:sideways").is_err());
    }
}
