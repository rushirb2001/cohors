//! The `cohors mcp` server — a Model Context Protocol surface over stdio so a
//! coding agent gets the same fleet view the dashboard does (ADR-023, ADR-025).
//!
//! Transport is a hand-rolled, synchronous JSON-RPC 2.0 loop over stdin/stdout
//! (newline-delimited messages, as the MCP stdio transport specifies). This
//! keeps the binary on the project's sync, no-tokio architecture (ADR-012) and
//! adds no new dependency. The tool layer is deliberately transport-agnostic, so
//! swapping in `rmcp` later is contained to [`run`].
//!
//! Tools: reads (`list_repos`, `get_repo`, `fleet_summary`, `repo_path`,
//! `search`, and the GitHub-enriched `list_prs`/`ci_status`) are always on;
//! actions (`fetch`, `pull`, `stash`, `run`) sit behind the ADR-025 tiers —
//! `--allow-writes`, `--allow-run`, per-call `confirm`, and `dry_run` (a
//! side-effect-free preview that needs no tier or confirm). Every read carries
//! fail-loud diagnostics (see [`meta`]).

use std::io::{BufRead, Write};

use anyhow::Result;
use cohors_core::{
    RepoSnapshot, SearchKind, Selector, SortMode, assess, fleet_summary, resolve, search_metadata,
};
use serde_json::{Value, json};

/// The MCP protocol version we implement. We echo the client's requested
/// version when it sends one; this is the fallback.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Which tiers of tools are enabled, chosen by the human at launch (ADR-025).
/// Read tools are always on; write/run tools are opt-in via these flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct Caps {
    pub allow_writes: bool,
    pub allow_run: bool,
    #[allow(dead_code)] // the local-only `open` tool is deferred.
    pub allow_open: bool,
}

/// A source of fleet snapshots — injected so tests can supply a fixture fleet
/// and the binary can supply a real scan.
type ScanFn<'a> = dyn Fn() -> Vec<RepoSnapshot> + 'a;

/// Everything a tool call needs: how to scan, *where* we're scanning + the
/// config in effect (for the fail-loud diagnostics — see [`meta`]), and which
/// tiers are enabled.
struct Ctx<'a> {
    scan: &'a ScanFn<'a>,
    /// GitHub token for the remote tools (`list_prs`/`ci_status`); `None` ⇒ no
    /// enrichment, and the remote tools say so.
    token: Option<&'a str>,
    roots: &'a [String],
    config_path: &'a str,
    caps: Caps,
    /// Glob patterns restricting `run` (empty = any command). ADR-025.
    allowlist: &'a [String],
    /// Action-target cap, bypassed by `{all: true}` (0 = no cap). ADR-025.
    max_targets: usize,
}

/// Run the stdio server loop until stdin closes. Each line is one JSON-RPC
/// message; each request gets exactly one response line, notifications none.
#[allow(clippy::too_many_arguments)]
pub fn run(
    scan: &ScanFn<'_>,
    token: Option<&str>,
    roots: &[String],
    config_path: &str,
    caps: Caps,
    allowlist: &[String],
    max_targets: usize,
) -> Result<()> {
    let ctx = Ctx {
        scan,
        token,
        roots,
        config_path,
        caps,
        allowlist,
        max_targets,
    };
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut line = String::new();

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break; // EOF — the client disconnected.
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(trimmed) {
            Ok(request) => handle(&request, &ctx, now_secs()),
            // A malformed line gets a JSON-RPC parse error (id unknown ⇒ null).
            Err(_) => Some(error_response(Value::Null, -32700, "parse error")),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut out, &response)?;
            out.write_all(b"\n")?;
            out.flush()?;
        }
    }
    Ok(())
}

/// Dispatch one JSON-RPC request. Returns `None` for notifications (no `id`),
/// which take no response.
fn handle(request: &Value, ctx: &Ctx, now: i64) -> Option<Value> {
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let id = request.get("id").cloned();
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    // No `id` ⇒ a notification; we acknowledge nothing.
    let id = id?;

    Some(match method {
        "initialize" => {
            // Echo the client's protocol version when offered.
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            result_response(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "cohors", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
        }
        "ping" => result_response(id, json!({})),
        "tools/list" => result_response(id, json!({ "tools": tool_catalog() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call_tool(name, &args, ctx, now) {
                Ok(value) => result_response(id, tool_content(&value, false)),
                Err(message) => result_response(id, tool_content(&json!(message), true)),
            }
        }
        _ => error_response(id, -32601, "method not found"),
    })
}

/// The catalog returned by `tools/list`. JSON-Schema'd inputs; `repo` accepts an
/// id, name, or path.
fn tool_catalog() -> Value {
    let selector_schema = json!({
        "type": "object",
        "description": "Fleet predicate (ADR-024). Fields AND together; e.g. {\"behind\": true}, {\"dirty\": true, \"name\": \"pay*\"}, {\"all\": true}. Empty matches nothing.",
        "additionalProperties": true
    });
    json!([
        {
            "name": "list_repos",
            "description": "List repositories with their status (the cohors scan shape plus a per-repo assessment and a fleet summary). Omit the selector to get the whole fleet.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": selector_schema.clone(),
                    "sort": { "type": "string", "enum": ["dirty-first", "recent", "name", "ahead-behind"] },
                    "fields": { "type": "array", "items": { "type": "string" }, "description": "Project each repo to these top-level fields (id and name always kept)." },
                    "limit": { "type": "integer", "minimum": 1 }
                }
            }
        },
        {
            "name": "get_repo",
            "description": "Get one repository's full status by id, name, or path — the same inspect the TUI shows on Enter. Includes remote_detail (open PRs, contributors, open issues, latest release) when the repo has a GitHub remote and a token.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": { "type": "string" } },
                "required": ["repo"]
            }
        },
        {
            "name": "fleet_summary",
            "description": "Fleet-wide counts: total, needs-attention, unpushed, behind, dirty, stashed, errors. The cheapest 'anything on fire?' call.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "search",
            "description": "Search across the fleet. kind=content greps file contents (ripgrep/git grep/fallback, fixed-string); kind=path/name/branch matches snapshot metadata. Scope with an optional selector. The entry point for cross-repo refactors.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "kind": { "type": "string", "enum": ["content", "path", "name", "branch"] },
                    "selector": selector_schema.clone(),
                    "max_results": { "type": "integer", "minimum": 1 }
                },
                "required": ["query"]
            }
        },
        {
            "name": "repo_path",
            "description": "Resolve a repository (by id, name, or path) to its absolute path, so the agent can operate in it directly.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": { "type": "string" } },
                "required": ["repo"]
            }
        },
        {
            "name": "list_prs",
            "description": "Open pull-request counts per repo (GitHub-enriched). Needs a token (gh auth / GITHUB_TOKEN); says so in meta when absent. Scope with an optional selector.",
            "inputSchema": {
                "type": "object",
                "properties": { "selector": selector_schema.clone(), "state": { "type": "string", "enum": ["open", "all"] } }
            }
        },
        {
            "name": "ci_status",
            "description": "CI/checks status per repo (GitHub-enriched: passing | failing | pending). Needs a token. Scope with an optional selector.",
            "inputSchema": {
                "type": "object",
                "properties": { "selector": selector_schema.clone() }
            }
        },
        {
            "name": "fetch",
            "description": "git fetch across the selected repos (non-destructive). Execution requires the server launched with --allow-writes; dry_run:true previews the target set without it.",
            "inputSchema": {
                "type": "object",
                "properties": { "selector": selector_schema.clone(), "dry_run": { "type": "boolean" } },
                "required": ["selector"]
            }
        },
        {
            "name": "pull",
            "description": "git pull --ff-only across the selected repos — never merges or rebases, so it can't lose work. Execution requires --allow-writes; dry_run:true previews without it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": selector_schema.clone(),
                    "mode": { "type": "string", "enum": ["ff-only"] },
                    "dry_run": { "type": "boolean" }
                },
                "required": ["selector"]
            }
        },
        {
            "name": "stash",
            "description": "git stash push (tracked changes) across the selected repos. Execution requires --allow-writes and confirm:true; dry_run:true previews the target set with neither.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": selector_schema.clone(),
                    "confirm": { "type": "boolean" },
                    "dry_run": { "type": "boolean" }
                },
                "required": ["selector"]
            }
        },
        {
            "name": "run",
            "description": "Run a shell command in each selected repo; returns per-repo {exit_code, stdout, stderr, truncated, timed_out}. The fleet codemod/audit/test primitive. Execution requires --allow-run and confirm:true; dry_run:true previews the target set with neither. Each repo is bounded by timeout_secs (default 120s).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "selector": selector_schema.clone(),
                    "confirm": { "type": "boolean" },
                    "dry_run": { "type": "boolean" },
                    "timeout_secs": { "type": "integer", "minimum": 1 }
                },
                "required": ["command", "selector"]
            }
        }
    ])
}

/// Execute a read tool, returning the JSON payload to embed (or an error message
/// for an `isError` tool result). Unknown tools and bad arguments are tool-level
/// errors, not protocol errors.
fn call_tool(name: &str, args: &Value, ctx: &Ctx, now: i64) -> Result<Value, String> {
    match name {
        "list_repos" => Ok(list_repos(args, ctx, now)),
        "get_repo" => get_repo(args, ctx, now),
        "fleet_summary" => {
            let snaps = (ctx.scan)();
            let mut value = serde_json::to_value(fleet_summary(&snaps, now))
                .map_err(|e| format!("serializing fleet summary: {e}"))?;
            if let Value::Object(map) = &mut value {
                map.insert("meta".to_string(), meta(ctx, &snaps));
            }
            Ok(value)
        }
        "repo_path" => repo_path(args, ctx),
        "search" => search(args, ctx, now),
        "list_prs" => Ok(list_prs(args, ctx, now)),
        "ci_status" => Ok(ci_status(args, ctx, now)),
        "fetch" => fetch_tool(args, ctx, now),
        "pull" => pull_tool(args, ctx, now),
        "stash" => stash_tool(args, ctx, now),
        "run" => run_tool(args, ctx, now),
        "open" => Err("`open` is not available in this build (local-desktop tool).".to_string()),
        other => Err(format!("unknown tool `{other}`")),
    }
}

// ── Action tools (ADR-025 safety tiers) ──────────────────────────────────────

/// `--allow-writes` gate, with a message that tells the agent how to enable it.
fn require_writes(ctx: &Ctx, tool: &str) -> Result<(), String> {
    if ctx.caps.allow_writes {
        Ok(())
    } else {
        Err(format!(
            "`{tool}` is disabled: this server is read-only. Relaunch it with `cohors mcp --allow-writes` to enable write tools."
        ))
    }
}

/// `confirm: true` gate for the destructive / arbitrary-shell tools.
fn require_confirm(args: &Value, tool: &str) -> Result<(), String> {
    if args.get("confirm").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(format!(
            "`{tool}` needs a deliberate `confirm`: pass \"confirm\": true (preview first with \"dry_run\": true)."
        ))
    }
}

fn is_dry_run(args: &Value) -> bool {
    args.get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Parse an action's selector (required — an empty selector matches nothing).
fn action_selector(args: &Value) -> Selector {
    args.get("selector")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

/// Resolve a parsed selector to action targets, scoped to the configured roots,
/// with error/path-less repos excluded since they can't be acted on (ADR-019).
fn resolve_targets(selector: &Selector, snaps: &[RepoSnapshot], now: i64) -> Vec<RepoSnapshot> {
    if selector.is_empty() {
        return Vec::new();
    }
    let order = resolve(snaps, selector, SortMode::DirtyFirst, now);
    let by_id: std::collections::HashMap<&str, &RepoSnapshot> =
        snaps.iter().map(|s| (s.id.0.as_str(), s)).collect();
    order
        .iter()
        .filter_map(|id| by_id.get(id.0.as_str()).copied())
        .filter(|s| !s.has_error() && s.path.is_some())
        .cloned()
        .collect()
}

/// Enforce the action-target cap: too many targets without an explicit
/// `{all: true}` is refused (ADR-025), so a fumbled selector can't fan out.
fn within_cap(selector: &Selector, targets: &[RepoSnapshot], max: usize) -> Result<(), String> {
    if max > 0 && !selector.all && targets.len() > max {
        Err(format!(
            "{} repos match — over the configured cap of {max}. Narrow the selector, or pass {{\"all\": true}} to act on the whole fleet deliberately.",
            targets.len()
        ))
    } else {
        Ok(())
    }
}

/// Write a one-line audit record of an action to the log (`cohors.log`, ADR-025).
fn audit(tool: &str, selector: &Selector, targets: &[RepoSnapshot], ok: usize) {
    let ids: Vec<&str> = targets.iter().map(|s| s.id.0.as_str()).collect();
    tracing::info!(
        target: "cohors::audit",
        tool,
        selector = %serde_json::to_string(selector).unwrap_or_default(),
        targets = targets.len(),
        ok,
        failed = targets.len() - ok,
        ids = ?ids,
        "mcp action",
    );
}

/// Whether `cmd` is permitted by the allowlist (empty allows anything).
fn command_allowed(cmd: &str, allowlist: &[String]) -> bool {
    allowlist.is_empty() || allowlist.iter().any(|pat| command_matches(pat, cmd))
}

/// A small `*`-glob match for allowlist patterns like `cargo *` or `git *`.
fn command_matches(pattern: &str, cmd: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = cmd.chars().collect();
    let (mut pi, mut ti, mut star, mut resume) = (0usize, 0usize, None, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            resume = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            resume += 1;
            ti = resume;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// The "you selected nothing" result — explicit, so the agent doesn't read an
/// empty action as "done."
fn no_targets() -> Value {
    json!({
        "targets": 0,
        "results": [],
        "note": "Selector resolved to 0 repos. An empty selector matches nothing — pass predicates or {\"all\": true}. (Repos that errored or have no local path are excluded from actions.)"
    })
}

/// A `dry_run` preview: the exact target set + the action, with no side effects.
fn dry_run_preview(action: Value, targets: &[RepoSnapshot]) -> Value {
    let repos: Vec<Value> = targets
        .iter()
        .map(|s| json!({ "repo": s.id.0, "name": s.name, "path": s.path }))
        .collect();
    json!({ "dry_run": true, "action": action, "targets": repos.len(), "repos": repos })
}

/// Run a per-repo git action (fetch / pull / stash) over the targets and collect
/// `{repo, ok, message}` results.
fn git_action(
    tool: &str,
    action: Value,
    args: &Value,
    ctx: &Ctx,
    now: i64,
    authorize: impl Fn() -> Result<(), String>,
    op: impl Fn(&camino::Utf8Path, &str) -> Result<String, String>,
) -> Result<Value, String> {
    let snaps = (ctx.scan)();
    let selector = action_selector(args);
    let targets = resolve_targets(&selector, &snaps, now);
    if targets.is_empty() {
        return Ok(no_targets());
    }
    within_cap(&selector, &targets, ctx.max_targets)?;
    // A dry run is side-effect-free, so it's allowed *before* the gate/confirm:
    // an agent can preview the exact target set (even on a read-only server) for
    // a human to approve, then enable the tier and act.
    if is_dry_run(args) {
        return Ok(dry_run_preview(action, &targets));
    }
    authorize()?;
    let results: Vec<Value> = targets
        .iter()
        .map(|s| {
            let path = s.path.as_ref().expect("action targets have a path");
            match op(path, &s.name) {
                Ok(message) => json!({ "repo": s.id.0, "ok": true, "message": message }),
                Err(message) => json!({ "repo": s.id.0, "ok": false, "message": message }),
            }
        })
        .collect();
    let ok = results.iter().filter(|r| r["ok"] == true).count();
    audit(tool, &selector, &targets, ok);
    Ok(json!({ "targets": targets.len(), "results": results }))
}

fn fetch_tool(args: &Value, ctx: &Ctx, now: i64) -> Result<Value, String> {
    git_action(
        "fetch",
        json!("fetch"),
        args,
        ctx,
        now,
        || require_writes(ctx, "fetch"),
        crate::action::fetch,
    )
}

fn pull_tool(args: &Value, ctx: &Ctx, now: i64) -> Result<Value, String> {
    git_action(
        "pull",
        json!("pull --ff-only"),
        args,
        ctx,
        now,
        || require_writes(ctx, "pull"),
        crate::action::pull_ff,
    )
}

fn stash_tool(args: &Value, ctx: &Ctx, now: i64) -> Result<Value, String> {
    git_action(
        "stash",
        json!("stash push"),
        args,
        ctx,
        now,
        || {
            require_writes(ctx, "stash")?;
            require_confirm(args, "stash")
        },
        crate::action::stash_push,
    )
}

/// Maximum captured output per stream per repo (matches the TUI runner, ADR-020).
const RUN_OUTPUT_CAP: usize = 64 * 1024;
/// Per-repo wall-clock bound for `run` when the caller doesn't set `timeout_secs`,
/// so one hung command can't stall the whole fan-out.
const DEFAULT_RUN_TIMEOUT_SECS: u64 = 120;

fn run_tool(args: &Value, ctx: &Ctx, now: i64) -> Result<Value, String> {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .filter(|c| !c.trim().is_empty())
        .ok_or("missing required argument `command`")?
        .to_string();

    let snaps = (ctx.scan)();
    let selector = action_selector(args);
    let targets = resolve_targets(&selector, &snaps, now);
    if targets.is_empty() {
        return Ok(no_targets());
    }
    within_cap(&selector, &targets, ctx.max_targets)?;
    // Side-effect-free preview is allowed before the gates (see `git_action`).
    if is_dry_run(args) {
        return Ok(dry_run_preview(json!({ "run": command }), &targets));
    }

    // Real execution: the `run` tier, a deliberate confirm, and the allowlist.
    if !ctx.caps.allow_run {
        return Err(
            "`run` is disabled: relaunch the server with `cohors mcp --allow-run` to enable arbitrary commands.".to_string(),
        );
    }
    require_confirm(args, "run")?;
    if !command_allowed(&command, ctx.allowlist) {
        return Err(format!(
            "`{command}` is not permitted by the configured run_allowlist ({:?}).",
            ctx.allowlist
        ));
    }

    let timeout = std::time::Duration::from_secs(
        args.get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_RUN_TIMEOUT_SECS)
            .max(1),
    );
    let run_id = next_run_id();
    // Fan out across a bounded pool (ADR-020) so a large fleet finishes fast
    // without spawning a process per repo at once. `par_iter().collect()`
    // preserves target order.
    let results: Vec<Value> = run_each(&targets, &command, timeout);
    let ok = results.iter().filter(|r| r["ok"] == true).count();
    audit("run", &selector, &targets, ok);
    Ok(json!({
        "run_id": run_id, "command": command, "targets": targets.len(),
        "ok": ok, "failed": targets.len() - ok, "results": results
    }))
}

/// Truncate a captured stream to [`RUN_OUTPUT_CAP`] bytes on a char boundary.
fn cap_output(mut s: String) -> (String, bool) {
    if s.len() <= RUN_OUTPUT_CAP {
        return (s, false);
    }
    let mut end = RUN_OUTPUT_CAP;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    (s, true)
}

/// Monotonic, process-global run id (ADR-020).
fn next_run_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// How many repos `run` executes concurrently (ADR-020).
const RUN_POOL_SIZE: usize = 8;

/// Run `command` in each target's directory over a bounded thread pool, returning
/// per-repo `{repo, ok, exit_code, stdout, stderr, truncated, timed_out}` results
/// in target order. Falls back to sequential if the pool can't be built.
fn run_each(targets: &[RepoSnapshot], command: &str, timeout: std::time::Duration) -> Vec<Value> {
    use rayon::prelude::*;

    let exec = |s: &RepoSnapshot| {
        let path = s.path.as_ref().expect("action targets have a path");
        let out = crate::action::run_command_timeout(path, command, timeout);
        let (stdout, t1) = cap_output(out.stdout);
        let (stderr, t2) = cap_output(out.stderr);
        json!({
            "repo": s.id.0, "ok": out.code == 0 && !out.timed_out, "exit_code": out.code,
            "stdout": stdout, "stderr": stderr, "truncated": t1 || t2, "timed_out": out.timed_out
        })
    };

    match rayon::ThreadPoolBuilder::new()
        .num_threads(RUN_POOL_SIZE)
        .build()
    {
        Ok(pool) => pool.install(|| targets.par_iter().map(exec).collect()),
        Err(_) => targets.iter().map(exec).collect(),
    }
}

// ── Remote read tools (GitHub enrichment) ────────────────────────────────────

/// Resolve a read tool's target snapshots: optional selector, defaulting to the
/// whole fleet (reads, unlike actions, never need an explicit selector).
fn resolve_read<'a>(args: &Value, snaps: &'a [RepoSnapshot], now: i64) -> Vec<&'a RepoSnapshot> {
    let mut selector: Selector = args
        .get("selector")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    if selector.is_empty() {
        selector.all = true;
    }
    let order = resolve(snaps, &selector, SortMode::DirtyFirst, now);
    let by_id: std::collections::HashMap<&str, &RepoSnapshot> =
        snaps.iter().map(|s| (s.id.0.as_str(), s)).collect();
    order
        .iter()
        .filter_map(|id| by_id.get(id.0.as_str()).copied())
        .collect()
}

/// Fail-loud meta for the remote tools. `excluded` is how many *selected* repos
/// were dropped for having no GitHub remote, so an agent can reconcile "18 repos
/// but 14 CI rows" instead of guessing. A missing token is called out too.
fn remote_meta(ctx: &Ctx, snaps: &[RepoSnapshot], excluded: usize) -> Value {
    let mut m = meta(ctx, snaps);
    m["excluded"] = json!(excluded);
    if ctx.token.is_none() {
        m["note"] = json!(
            "No GitHub token found (run `gh auth login` or set $GITHUB_TOKEN); PR/CI data is unavailable."
        );
    } else if excluded > 0 {
        m["note"] = json!(format!(
            "{excluded} selected repo(s) have no GitHub remote and were omitted from the results."
        ));
    }
    m
}

fn list_prs(args: &Value, ctx: &Ctx, now: i64) -> Value {
    let mut snaps = (ctx.scan)();
    cohors_github::enrich(&mut snaps, ctx.token);
    let selected = resolve_read(args, &snaps, now);
    let repos: Vec<Value> = selected
        .iter()
        .filter_map(|s| {
            s.remote.as_ref().map(|r| {
                json!({ "repo": s.id.0, "open_prs": r.open_prs, "awaiting_review": r.prs_awaiting_review })
            })
        })
        .collect();
    let excluded = selected.len() - repos.len();
    json!({ "repos": repos, "meta": remote_meta(ctx, &snaps, excluded) })
}

fn ci_status(args: &Value, ctx: &Ctx, now: i64) -> Value {
    let mut snaps = (ctx.scan)();
    cohors_github::enrich(&mut snaps, ctx.token);
    let selected = resolve_read(args, &snaps, now);
    let repos: Vec<Value> = selected
        .iter()
        .filter_map(|s| {
            s.remote
                .as_ref()
                .map(|r| json!({ "repo": s.id.0, "ci": r.ci }))
        })
        .collect();
    let excluded = selected.len() - repos.len();
    json!({ "repos": repos, "meta": remote_meta(ctx, &snaps, excluded) })
}

fn list_repos(args: &Value, ctx: &Ctx, now: i64) -> Value {
    let snaps = (ctx.scan)();

    // Reads default to the whole fleet; only actions require an explicit selector.
    let mut selector: Selector = args
        .get("selector")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    if selector.is_empty() {
        selector.all = true;
    }

    let sort = parse_sort(args.get("sort").and_then(Value::as_str));
    let order = resolve(&snaps, &selector, sort, now);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(usize::MAX);
    let fields: Option<Vec<String>> = args.get("fields").and_then(|v| {
        v.as_array().map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
    });

    let by_id: std::collections::HashMap<&str, &RepoSnapshot> =
        snaps.iter().map(|s| (s.id.0.as_str(), s)).collect();
    let repos: Vec<Value> = order
        .iter()
        .filter_map(|id| by_id.get(id.0.as_str()).copied())
        .take(limit)
        .map(|snap| project(repo_json(snap, now), fields.as_deref()))
        .collect();

    json!({ "fleet": fleet_summary(&snaps, now), "repos": repos, "meta": meta(ctx, &snaps) })
}

fn get_repo(args: &Value, ctx: &Ctx, now: i64) -> Result<Value, String> {
    let key = repo_arg(args)?;
    let snaps = (ctx.scan)();
    let snap = find_repo(&snaps, &key).ok_or_else(|| format!("no repository matching `{key}`"))?;
    let mut value = repo_json(snap, now);
    // Same GitHub detail the TUI shows on Enter (PRs / contributors / issues /
    // release), so inspecting a repo is consistent across the TUI and MCP.
    if let (Some(url), Some(token)) = (snap.remote_url.as_deref(), ctx.token)
        && let Some(detail) = cohors_github::fetch_repo_detail(token, url)
        && let Value::Object(map) = &mut value
    {
        map.insert(
            "remote_detail".to_string(),
            serde_json::to_value(detail).unwrap_or(Value::Null),
        );
    }
    Ok(value)
}

fn repo_path(args: &Value, ctx: &Ctx) -> Result<Value, String> {
    let key = repo_arg(args)?;
    let snaps = (ctx.scan)();
    let snap = find_repo(&snaps, &key).ok_or_else(|| format!("no repository matching `{key}`"))?;
    match &snap.path {
        Some(path) => Ok(json!({ "path": path })),
        None => Err(format!("repository `{key}` has no local path")),
    }
}

/// `search` — content grep (via the git adapter) or snapshot metadata match,
/// scoped by an optional selector (reads default to the whole fleet).
fn search(args: &Value, ctx: &Ctx, now: i64) -> Result<Value, String> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .filter(|q| !q.is_empty())
        .ok_or("missing required argument `query`")?
        .to_string();
    let kind: SearchKind = args
        .get("kind")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or(SearchKind::Content);
    let max_results = args
        .get("max_results")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(200)
        .max(1);

    let mut selector: Selector = args
        .get("selector")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    if selector.is_empty() {
        selector.all = true;
    }

    let snaps = (ctx.scan)();
    let order = resolve(&snaps, &selector, SortMode::DirtyFirst, now);
    let by_id: std::collections::HashMap<&str, &RepoSnapshot> =
        snaps.iter().map(|s| (s.id.0.as_str(), s)).collect();
    let selected: Vec<&RepoSnapshot> = order
        .iter()
        .filter_map(|id| by_id.get(id.0.as_str()).copied())
        .collect();

    let mut hits: Vec<Value> = Vec::new();
    let mut truncated = false;

    if kind == SearchKind::Content {
        for snap in &selected {
            if hits.len() >= max_results {
                truncated = true;
                break;
            }
            let Some(path) = &snap.path else {
                continue;
            };
            let remaining = max_results - hits.len();
            for hit in cohors_git::search_content(path, &query, remaining + 1) {
                if hits.len() >= max_results {
                    truncated = true;
                    break;
                }
                hits.push(json!({
                    "repo": snap.id.0, "path": hit.path, "line": hit.line, "text": hit.text
                }));
            }
        }
    } else {
        let owned: Vec<RepoSnapshot> = selected.iter().map(|s| (*s).clone()).collect();
        for hit in search_metadata(&owned, &query, kind) {
            if hits.len() >= max_results {
                truncated = true;
                break;
            }
            hits.push(serde_json::to_value(hit).unwrap_or(Value::Null));
        }
    }

    Ok(json!({ "hits": hits, "truncated": truncated, "meta": meta(ctx, &snaps) }))
}

/// Fail-loud diagnostics attached to every read: where we looked, the config in
/// effect, and whether the result is empty or partial — so an agent never reads
/// `total: 0` as "all clear" (the failure mode that turns a misconfigured root
/// into a confident wrong answer).
fn meta(ctx: &Ctx, snaps: &[RepoSnapshot]) -> Value {
    let total = snaps.len();
    let errored = snaps.iter().filter(|s| s.has_error()).count();
    let mut m = json!({
        "roots": ctx.roots,
        "config_path": ctx.config_path,
        "total": total,
        "errored": errored,
    });
    if total == 0 {
        m["note"] = json!(format!(
            "No repositories found under {:?}. Point cohors at your code: set `roots` in {}, pass --root, or run it where your repos live.",
            ctx.roots, ctx.config_path
        ));
    } else if errored > 0 {
        m["note"] = json!(format!(
            "{errored} of {total} repositories could not be read (each carries an `error`); results are partial."
        ));
    }
    m
}

/// Read the required `repo` string argument.
fn repo_arg(args: &Value) -> Result<String, String> {
    args.get("repo")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| "missing required argument `repo`".to_string())
}

/// Find a repo by id, name, or canonical path.
fn find_repo<'a>(snaps: &'a [RepoSnapshot], key: &str) -> Option<&'a RepoSnapshot> {
    snaps.iter().find(|s| {
        s.id.0 == key || s.name == key || s.path.as_ref().is_some_and(|p| p.as_str() == key)
    })
}

/// Serialize a snapshot to the `cohors scan` JSON, adding a per-repo `assessment`
/// (severity + needs-attention) — the one thing the MCP layer adds over `scan`.
fn repo_json(snap: &RepoSnapshot, now: i64) -> Value {
    let mut value = serde_json::to_value(snap).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut value {
        let assessment = assess(snap, now);
        map.insert(
            "assessment".to_string(),
            json!({
                "severity": assessment.severity,
                "needs_attention": assessment.needs_attention(),
            }),
        );
    }
    value
}

/// Keep only the requested top-level fields (id and name always survive).
fn project(value: Value, fields: Option<&[String]>) -> Value {
    let Some(fields) = fields else {
        return value;
    };
    let Value::Object(map) = value else {
        return value;
    };
    let mut out = serde_json::Map::new();
    for (key, val) in map {
        if key == "id" || key == "name" || fields.iter().any(|f| f == &key) {
            out.insert(key, val);
        }
    }
    Value::Object(out)
}

fn parse_sort(sort: Option<&str>) -> SortMode {
    match sort {
        Some("recent") => SortMode::Recent,
        Some("name") => SortMode::Name,
        Some("ahead-behind") => SortMode::AheadBehind,
        _ => SortMode::DirtyFirst,
    }
}

/// Wrap a tool payload as an MCP `tools/call` result (`content` text holds the
/// JSON; `isError` flags a tool-level failure).
fn tool_content(value: &Value, is_error: bool) -> Value {
    let text = match value {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    };
    json!({ "content": [ { "type": "text", "text": text } ], "isError": is_error })
}

fn result_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    fn scan() -> Vec<RepoSnapshot> {
        cohors_core::demo::fleet(NOW)
    }

    /// Drive one request through the dispatcher with the demo fleet.
    fn call(request: Value) -> Value {
        let scan_fn = scan;
        let roots = vec!["/demo".to_string()];
        let ctx = Ctx {
            scan: &scan_fn,
            token: None,
            roots: &roots,
            config_path: "(test)",
            caps: Caps::default(),
            allowlist: &[],
            max_targets: 0,
        };
        handle(&request, &ctx, NOW).expect("request expects a response")
    }

    /// Like [`call`], but with explicit capability tiers.
    fn call_caps(request: Value, caps: Caps) -> Value {
        let scan_fn = scan;
        let roots = vec!["/demo".to_string()];
        let ctx = Ctx {
            scan: &scan_fn,
            token: None,
            roots: &roots,
            config_path: "(test)",
            caps,
            allowlist: &[],
            max_targets: 0,
        };
        handle(&request, &ctx, NOW).expect("request expects a response")
    }

    /// Full control over caps, allowlist, and the action-target cap.
    fn call_ctx(request: Value, caps: Caps, allowlist: &[String], max_targets: usize) -> Value {
        let scan_fn = scan;
        let roots = vec!["/demo".to_string()];
        let ctx = Ctx {
            scan: &scan_fn,
            token: None,
            roots: &roots,
            config_path: "(test)",
            caps,
            allowlist,
            max_targets,
        };
        handle(&request, &ctx, NOW).expect("request expects a response")
    }

    fn payload(resp: &Value) -> Value {
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    fn is_error(resp: &Value) -> bool {
        resp["result"]["isError"] == true
    }

    fn err_text(resp: &Value) -> String {
        resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    const WRITES: Caps = Caps {
        allow_writes: true,
        allow_run: false,
        allow_open: false,
    };
    const RUN: Caps = Caps {
        allow_writes: false,
        allow_run: true,
        allow_open: false,
    };

    fn act(name: &str, arguments: Value) -> Value {
        json!({
            "jsonrpc": "2.0", "id": 99, "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        })
    }

    #[test]
    fn initialize_reports_server_info() {
        let resp = call(json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18" }
        }));
        assert_eq!(resp["result"]["serverInfo"]["name"], "cohors");
        // Echoes the client's protocol version.
        assert_eq!(resp["result"]["protocolVersion"], "2025-06-18");
    }

    #[test]
    fn notifications_get_no_response() {
        let scan_fn = scan;
        let ctx = Ctx {
            scan: &scan_fn,
            token: None,
            roots: &[],
            config_path: "(test)",
            caps: Caps::default(),
            allowlist: &[],
            max_targets: 0,
        };
        let resp = handle(
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            &ctx,
            NOW,
        );
        assert!(resp.is_none());
    }

    #[test]
    fn tools_list_returns_read_catalog() {
        let resp = call(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"list_repos"));
        assert!(names.contains(&"fleet_summary"));
        assert!(names.contains(&"get_repo"));
        assert!(names.contains(&"repo_path"));
    }

    #[test]
    fn list_repos_returns_fleet_and_repos_with_assessment() {
        let resp = call(json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "list_repos", "arguments": {} }
        }));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert!(payload["fleet"]["total"].as_u64().unwrap() >= 1);
        let repos = payload["repos"].as_array().unwrap();
        assert!(!repos.is_empty());
        assert!(repos[0].get("assessment").is_some());
        assert!(repos[0]["assessment"].get("severity").is_some());
    }

    #[test]
    fn list_repos_selector_filters() {
        let resp = call(json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "list_repos", "arguments": { "selector": { "error": true } } }
        }));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        let repos = payload["repos"].as_array().unwrap();
        // The demo fleet includes one unreadable repo.
        assert!(repos.iter().all(|r| r["error"].is_string()));
    }

    #[test]
    fn list_repos_fields_projection() {
        let resp = call(json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": { "name": "list_repos", "arguments": { "fields": ["branch"], "limit": 1 } }
        }));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        let repo = &payload["repos"][0];
        assert!(repo.get("branch").is_some());
        assert!(repo.get("id").is_some()); // always kept
        assert!(repo.get("worktree").is_none()); // projected out
    }

    #[test]
    fn get_repo_unknown_is_tool_error() {
        let resp = call(json!({
            "jsonrpc": "2.0", "id": 6, "method": "tools/call",
            "params": { "name": "get_repo", "arguments": { "repo": "does-not-exist" } }
        }));
        assert_eq!(resp["result"]["isError"], true);
    }

    #[test]
    fn action_tools_are_read_only_errors() {
        let resp = call(json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": { "name": "pull", "arguments": { "selector": { "all": true } } }
        }));
        assert_eq!(resp["result"]["isError"], true);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("read-only"));
    }

    #[test]
    fn unknown_method_is_protocol_error() {
        let resp = call(json!({ "jsonrpc": "2.0", "id": 8, "method": "frobnicate" }));
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn search_is_in_catalog() {
        let resp = call(json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/list" }));
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"search"));
    }

    #[test]
    fn search_requires_a_query() {
        let resp = call(json!({
            "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": { "name": "search", "arguments": { "kind": "name" } }
        }));
        assert_eq!(resp["result"]["isError"], true);
    }

    #[test]
    fn search_metadata_returns_hits_and_truncated_shape() {
        let resp = call(json!({
            "jsonrpc": "2.0", "id": 11, "method": "tools/call",
            "params": { "name": "search", "arguments": { "query": "a", "kind": "name" } }
        }));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert!(payload["hits"].is_array());
        assert!(payload["truncated"].is_boolean());
    }

    #[test]
    fn reads_carry_meta_with_roots_and_config() {
        let resp = call(json!({
            "jsonrpc": "2.0", "id": 12, "method": "tools/call",
            "params": { "name": "list_repos", "arguments": {} }
        }));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["meta"]["roots"][0], "/demo");
        assert_eq!(payload["meta"]["config_path"], "(test)");
        assert!(payload["meta"]["total"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn empty_fleet_explains_itself() {
        // A misconfigured/empty fleet must say *why* it's empty, not look "all clear".
        let empty = || Vec::<RepoSnapshot>::new();
        let roots = vec!["~/projects".to_string()];
        let ctx = Ctx {
            scan: &empty,
            token: None,
            roots: &roots,
            config_path: "/cfg/config.toml",
            caps: Caps::default(),
            allowlist: &[],
            max_targets: 0,
        };
        let resp = handle(
            &json!({
                "jsonrpc": "2.0", "id": 13, "method": "tools/call",
                "params": { "name": "fleet_summary", "arguments": {} }
            }),
            &ctx,
            NOW,
        )
        .unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["meta"]["total"], 0);
        assert_eq!(payload["meta"]["roots"][0], "~/projects");
        let note = payload["meta"]["note"].as_str().unwrap();
        assert!(note.contains("No repositories"));
        assert!(note.contains("/cfg/config.toml"));
    }

    #[test]
    fn write_tool_is_gated_without_allow_writes() {
        let resp = call(act("fetch", json!({ "selector": { "all": true } })));
        assert_eq!(resp["result"]["isError"], true);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("--allow-writes"));
    }

    #[test]
    fn dry_run_previews_targets_without_acting() {
        let resp = call_caps(
            act(
                "fetch",
                json!({ "selector": { "all": true }, "dry_run": true }),
            ),
            WRITES,
        );
        let p = payload(&resp);
        assert_eq!(p["dry_run"], true);
        assert!(p["targets"].as_u64().unwrap() >= 1);
        assert!(p["repos"].is_array());
    }

    #[test]
    fn stash_requires_confirm_even_with_writes() {
        let resp = call_caps(act("stash", json!({ "selector": { "all": true } })), WRITES);
        assert_eq!(resp["result"]["isError"], true);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("confirm"));
    }

    #[test]
    fn run_is_gated_without_allow_run() {
        // Even with writes, `run` needs its own flag.
        let resp = call_caps(
            act(
                "run",
                json!({ "command": "echo hi", "selector": { "all": true }, "confirm": true }),
            ),
            WRITES,
        );
        assert_eq!(resp["result"]["isError"], true);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("--allow-run"));
    }

    #[test]
    fn run_requires_confirm() {
        let resp = call_caps(
            act(
                "run",
                json!({ "command": "echo hi", "selector": { "all": true } }),
            ),
            RUN,
        );
        assert_eq!(resp["result"]["isError"], true);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("confirm"));
    }

    #[test]
    fn empty_selector_action_is_no_targets() {
        let resp = call_caps(act("fetch", json!({})), WRITES);
        let p = payload(&resp);
        assert_eq!(p["targets"], 0);
        assert!(p["note"].as_str().unwrap().contains("empty selector"));
    }

    #[test]
    fn action_tools_appear_in_catalog() {
        let resp = call(json!({ "jsonrpc": "2.0", "id": 100, "method": "tools/list" }));
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        // Parity: every repo-targeting TUI verb has a matching MCP tool, plus the
        // remote read tools.
        for tool in ["fetch", "pull", "stash", "run", "list_prs", "ci_status"] {
            assert!(names.contains(&tool), "missing {tool}");
        }
    }

    #[test]
    fn dry_run_previews_without_confirm_or_gate() {
        // A side-effect-free preview must work on a read-only server with no
        // confirm — for both confirm-gated tools (run, stash).
        for tool in ["run", "stash"] {
            let mut args = json!({ "selector": { "all": true }, "dry_run": true });
            if tool == "run" {
                args["command"] = json!("echo SHOULD_NOT_RUN");
            }
            let resp = call(act(tool, args)); // default caps: read-only, no confirm
            assert_eq!(resp["result"]["isError"], false, "{tool} dry_run errored");
            let p = payload(&resp);
            assert_eq!(p["dry_run"], true, "{tool} should preview");
            assert!(p["targets"].as_u64().unwrap() >= 1);
        }
    }

    #[test]
    fn remote_tools_return_shape_and_note_missing_token() {
        for tool in ["ci_status", "list_prs"] {
            let resp = call(act(tool, json!({})));
            let p = payload(&resp);
            assert!(p["repos"].is_array(), "{tool} repos");
            assert!(p["meta"]["excluded"].is_number(), "{tool} reports excluded");
            assert!(
                p["meta"]["note"].as_str().unwrap().contains("token"),
                "{tool} should note the missing token"
            );
        }
    }

    #[test]
    fn cap_blocks_broad_selector_but_all_bypasses() {
        // `name:*` matches the whole fleet but isn't an explicit {all:true}.
        let over = act(
            "fetch",
            json!({ "selector": { "name": "*" }, "dry_run": true }),
        );
        let resp = call_ctx(over, WRITES, &[], 1);
        assert!(is_error(&resp));
        assert!(err_text(&resp).contains("cap"));

        // {all:true} is the deliberate escape hatch — no cap error.
        let all = act(
            "fetch",
            json!({ "selector": { "all": true }, "dry_run": true }),
        );
        let resp = call_ctx(all, WRITES, &[], 1);
        assert_eq!(payload(&resp)["dry_run"], true);
    }

    #[test]
    fn run_allowlist_blocks_then_permits() {
        let allow = vec!["echo *".to_string()];
        // A non-matching command is refused before executing.
        let bad = act(
            "run",
            json!({ "command": "rm -rf x", "selector": { "all": true }, "confirm": true }),
        );
        let resp = call_ctx(bad, RUN, &allow, 0);
        assert!(is_error(&resp));
        assert!(err_text(&resp).contains("allowlist"));

        // A matching command is allowed through (it then runs; demo paths are
        // synthetic so per-repo runs just fail — the point is it wasn't blocked).
        let ok = act(
            "run",
            json!({ "command": "echo hi", "selector": { "all": true }, "confirm": true }),
        );
        let resp = call_ctx(ok, RUN, &allow, 0);
        assert!(!is_error(&resp));
    }

    #[test]
    fn command_matches_globs() {
        assert!(command_matches("cargo *", "cargo build --release"));
        assert!(command_matches("git *", "git status -s"));
        assert!(!command_matches("git *", "rm -rf /"));
        assert!(command_matches("*", "anything goes"));
        assert!(command_matches("exact", "exact"));
        assert!(!command_matches("exact", "exactly"));
    }
}
