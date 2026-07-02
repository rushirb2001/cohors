//! The `cohors mcp` server — a Model Context Protocol surface over stdio so a
//! coding agent gets the same fleet view the dashboard does (ADR-023, ADR-025).
//!
//! Transport is a hand-rolled, synchronous JSON-RPC 2.0 loop over stdin/stdout
//! (newline-delimited messages, as the MCP stdio transport specifies). This
//! keeps the binary on the project's sync, no-tokio architecture (ADR-012) and
//! adds no new dependency. The tool layer is deliberately transport-agnostic, so
//! swapping in `rmcp` later is contained to [`serve_stdio`].
//!
//! Tools: reads (`list_repos`, `get_repo`, `fleet_summary`, `repo_path`,
//! `search`, and the GitHub-enriched `list_prs`/`ci_status`) are always on;
//! actions (`fetch`, `pull`, `stash`, `run`) sit behind the ADR-025 tiers —
//! `--allow-writes`, `--allow-run`, per-call `confirm`, and `dry_run` (a
//! side-effect-free preview that needs no tier or confirm). Every read carries
//! fail-loud diagnostics (see [`meta`]).
//!
//! This is the 4th adapter, its own crate (ADR-002/023): it reads through the
//! `cohors-fleet` facade and writes through `cohors-actions` — never the raw
//! git/github adapters. The `cohors` binary's `mcp` subcommand calls
//! [`serve_stdio`].

#![forbid(unsafe_code)]

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
pub fn serve_stdio(
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
                    "instructions": INSTRUCTIONS,
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

/// Server-level guidance surfaced to the model on `initialize` (the MCP
/// `instructions` field). This is the highest-leverage place to make an agent
/// reach for cohors instead of shelling out to `git`/`find`.
const INSTRUCTIONS: &str = "cohors is the user's git repository fleet — every repo under their configured roots — with live local status (branch, ahead/behind, dirty worktree, stashes, last commit) and optional GitHub enrichment (CI, PRs). Whenever the user asks about \"my repos\", \"my projects\", the \"fleet\", what is dirty / unpushed / behind / needs attention, where a repo lives, or to find or change code across repos, use these cohors tools — do NOT shell out to find, cd, or per-repo git, and do not ask the user where their repos are. Start with fleet_summary (cheapest) or list_repos. Reads are always available; the write tools (fetch/pull/push/commit/stash) and run are gated and will tell you how to enable them. Target repos with the shared selector predicate, e.g. {\"dirty\": true}, {\"behind\": true}, {\"name\": \"pay*\"}, or {\"all\": true}.";

/// The catalog returned by `tools/list`. JSON-Schema'd inputs; `repo` accepts an
/// id, name, or path. Descriptions lead with *when* to use the tool, so an agent
/// matches user intent to a tool instead of falling back to raw git.
fn tool_catalog() -> Value {
    let selector_schema = json!({
        "type": "object",
        "description": "Predicate that selects repos across the fleet; set fields to AND them, omit a field for no constraint (ADR-024). Scope: all (bool — the whole fleet), ids[str], name (glob, e.g. \"pay*\"), path_glob, root, group (a config-defined cluster name, e.g. \"payments\"). Local state: dirty, ahead (alias unpushed), behind, diverged, no_upstream, has_stash, detached, error, branch (exact name), attention (\"any\"|\"notice\"|\"warn\"|\"risk\"). Remote (needs a GitHub token): ci (\"passing\"|\"failing\"|\"pending\"), min_prs (int). Combine: any_of[selector] (OR), not (selector). The empty selector {} matches NOTHING by design — pass {\"all\": true} for the whole fleet.",
        "additionalProperties": true
    });
    // Read tools are hand-written (their schemas are bespoke); the action tools
    // below are appended from the registry.
    let mut tools = json!([
        {
            "name": "list_repos",
            "description": "The primary \"what's going on across my repos\" call: returns every repo's git status — branch, ahead/behind, dirty worktree, stashes, last commit, and an attention assessment — plus a fleet summary, in one request. Use this (or fleet_summary) whenever the user asks about their repos, projects, or fleet, or what is dirty/unpushed/behind/needs attention; do NOT enumerate repos yourself with find or per-repo git. Omit selector for the whole fleet, or narrow it (e.g. {\"dirty\": true}). Use fields to shrink each repo to just the keys you need.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": selector_schema.clone(),
                    "sort": { "type": "string", "enum": ["dirty-first", "recent", "name", "ahead-behind"], "description": "Order of the returned repos (default dirty-first)." },
                    "fields": { "type": "array", "items": { "type": "string" }, "description": "Project each repo to just these top-level keys (id and name are always kept) to keep the response small. Valid keys: branch, upstream, worktree, last_commit, activity, groups, stash_count, stash_latest, remote, assessment, path, error. Note: ahead/behind live inside `upstream`, and staged/modified/untracked inside `worktree` — request those parent keys, not the leaf names." },
                    "limit": { "type": "integer", "minimum": 1, "description": "Return at most this many repos (after sorting)." }
                }
            }
        },
        {
            "name": "get_repo",
            "description": "Full status of ONE repository, found by id, name, or path — branch, upstream ahead/behind, worktree counts, stashes, recent activity, and last commit. Adds remote_detail (open PRs, contributors, open issues, latest release) when the repo has a GitHub remote and a token. Use when the user names a single repo; for several repos call list_repos with a selector instead.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": { "type": "string", "description": "Repo id, name/alias, or path." } },
                "required": ["repo"]
            }
        },
        {
            "name": "fleet_summary",
            "description": "The cheapest \"is anything on fire?\" call: fleet-wide counts only — total, needs-attention, unpushed, behind, dirty, stashed, errors. Use it first to gauge overall state, then drill in with list_repos. Takes no arguments.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "search",
            "description": "Find code or repos ACROSS the whole fleet without cd-ing into each one. kind=content greps file contents (ripgrep/git grep/fallback, fixed-string — the default); kind=path|name|branch matches repo metadata. An optional selector limits which repos are searched. This is the entry point for cross-repo audits and refactors — use it instead of running grep/rg in each repo yourself.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "What to look for (fixed string for content; substring/glob for path/name/branch)." },
                    "kind": { "type": "string", "enum": ["content", "path", "name", "branch"], "description": "What to match (default content)." },
                    "selector": selector_schema.clone(),
                    "max_results": { "type": "integer", "minimum": 1, "description": "Cap on total hits (default 200)." }
                },
                "required": ["query"]
            }
        },
        {
            "name": "changes",
            "description": "What is actually uncommitted in each selected repo: the changed-file list (each path with its git porcelain status) and, with include_patch:true, a size-capped unified diff of the working tree. Use this to summarize or review uncommitted work — e.g. \"what's the work sitting in hybrid-flow?\" — instead of cd-ing in and running git status / git diff. Clean repos are omitted. Omit the selector to cover every dirty repo, or scope it (e.g. {\"name\": \"hybrid-flow\"}) to one.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": selector_schema.clone(),
                    "include_patch": { "type": "boolean", "description": "Also return a unified diff of the working tree per repo (default false; can be large)." },
                    "max_bytes": { "type": "integer", "minimum": 1, "description": "Cap each repo's patch in bytes (default 20000); the patch is flagged `truncated` when cut." }
                }
            }
        },
        {
            "name": "repo_path",
            "description": "Resolve a repo (by id, name, or path) to its absolute path so you can operate in it directly. Prefer this over searching the filesystem for where a repo lives.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": { "type": "string", "description": "Repo id, name/alias, or path." } },
                "required": ["repo"]
            }
        },
        {
            "name": "list_prs",
            "description": "Open pull requests per repo (GitHub-enriched). Use for \"any open PRs across my repos / waiting on me?\". Needs a token (gh auth or GITHUB_TOKEN) — meta says so when absent — and repos without a GitHub remote are omitted. An optional selector scopes the set.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": selector_schema.clone(),
                    "state": { "type": "string", "enum": ["open", "all"], "description": "Which PRs to count (default open)." }
                }
            }
        },
        {
            "name": "ci_status",
            "description": "CI / checks status per repo (GitHub-enriched: passing | failing | pending). Use for \"which repos have red/failing CI?\"; to then act on them, pass {\"ci\": \"failing\"} as a selector to another tool. Needs a token; repos without a GitHub remote are omitted. An optional selector scopes the set.",
            "inputSchema": {
                "type": "object",
                "properties": { "selector": selector_schema.clone() }
            }
        },
    ]);
    // The action half (fetch/pull/push/commit/stash/run) is generated from the
    // shared registry (ADR: one registry drives all surfaces), so a verb added in
    // `cohors-actions` shows up here automatically and the parity test enforces it.
    if let Value::Array(arr) = &mut tools {
        for def in cohors_actions::registry() {
            arr.push(action_tool(def, &selector_schema));
        }
    }
    tools
}

/// Build one action tool's catalog entry from its [`cohors_actions::ActionDef`]:
/// the name and description come from the registry; the per-verb argument schema
/// (which differ — `commit` needs a message, `run` a command + timeout) is here.
fn action_tool(def: &cohors_actions::ActionDef, selector_schema: &Value) -> Value {
    let dry_run = json!({ "type": "boolean", "description": "Preview the resolved target set without acting." });
    let confirm = json!({ "type": "boolean", "description": "Must be true to actually act; preview first with dry_run." });

    let mut props = serde_json::Map::new();
    let mut required = vec![json!("selector")];
    // `run`'s command comes before the selector in the existing schema; keep that.
    if def.verb == "run" {
        props.insert(
            "command".into(),
            json!({ "type": "string", "description": "Shell command run in each selected repo's directory." }),
        );
        required.insert(0, json!("command"));
    }
    props.insert("selector".into(), selector_schema.clone());
    if def.verb == "pull" {
        props.insert(
            "mode".into(),
            json!({ "type": "string", "enum": ["ff-only"], "description": "Only fast-forward is supported (the default)." }),
        );
    }
    if def.verb == "commit" {
        props.insert(
            "message".into(),
            json!({ "type": "string", "description": "Commit message, applied to every selected repo (required)." }),
        );
        required.push(json!("message"));
    }
    if def.needs_confirm {
        props.insert("confirm".into(), confirm);
    }
    props.insert("dry_run".into(), dry_run);
    if def.verb == "run" {
        props.insert(
            "timeout_secs".into(),
            json!({ "type": "integer", "minimum": 1, "description": "Per-repo timeout in seconds (default 120)." }),
        );
    }

    json!({
        "name": def.verb,
        "description": def.summary,
        "inputSchema": { "type": "object", "properties": Value::Object(props), "required": required }
    })
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
        "changes" => Ok(changes(args, ctx, now)),
        "search" => search(args, ctx, now),
        "list_prs" => Ok(list_prs(args, ctx, now)),
        "ci_status" => Ok(ci_status(args, ctx, now)),
        // Every registry verb routes through the one shared dispatcher — the
        // verb→primitive mapping and the tier/confirm gates live in
        // `cohors-actions`, not per surface (ADR: one registry, one dispatch).
        "fetch" | "pull" | "push" | "commit" | "stash" | "run" => cohors_actions::dispatch(
            name,
            args,
            &(ctx.scan)(),
            cohors_actions::Caps {
                allow_writes: ctx.caps.allow_writes,
                allow_run: ctx.caps.allow_run,
            },
            ctx.allowlist,
            ctx.max_targets,
            now,
            "cohors mcp",
        ),
        "open" => Err("`open` is not available in this build (local-desktop tool).".to_string()),
        other => Err(format!("unknown tool `{other}`")),
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
    cohors_fleet::enrich(&mut snaps, ctx.token);
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
    cohors_fleet::enrich(&mut snaps, ctx.token);
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
        && let Some(detail) = cohors_fleet::fetch_repo_detail(token, url)
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

/// `changes` — per-repo uncommitted file list (+ optional capped patch), scoped
/// by an optional selector. Reads default to every *dirty* repo (a clean repo
/// has nothing to show); path-less/errored repos are skipped, as are repos that
/// turn out to have no changes.
fn changes(args: &Value, ctx: &Ctx, now: i64) -> Value {
    let snaps = (ctx.scan)();

    let mut selector: Selector = args
        .get("selector")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    // No selector ⇒ every repo with uncommitted changes (the useful default).
    if selector.is_empty() {
        selector.dirty = true;
    }

    let include_patch = args
        .get("include_patch")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max_bytes = args
        .get("max_bytes")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(20_000)
        .max(1);

    let order = resolve(&snaps, &selector, SortMode::DirtyFirst, now);
    let by_id: std::collections::HashMap<&str, &RepoSnapshot> =
        snaps.iter().map(|s| (s.id.0.as_str(), s)).collect();

    let repos: Vec<Value> = order
        .iter()
        .filter_map(|id| by_id.get(id.0.as_str()).copied())
        .filter_map(|snap| {
            let path = snap.path.as_deref()?; // skip path-less repos
            let ch = cohors_fleet::repo_changes(path, include_patch, max_bytes);
            if ch.files.is_empty() {
                return None; // nothing uncommitted — omit the repo entirely
            }
            let mut obj = serde_json::Map::new();
            obj.insert("repo".into(), json!(snap.name));
            obj.insert("id".into(), json!(snap.id.0));
            obj.insert(
                "files".into(),
                serde_json::to_value(&ch.files).unwrap_or(Value::Null),
            );
            if let Some(patch) = ch.patch {
                obj.insert("patch".into(), json!(patch));
                obj.insert("truncated".into(), json!(ch.truncated));
            }
            Some(Value::Object(obj))
        })
        .collect();

    json!({ "repos": repos, "meta": meta(ctx, &snaps) })
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
            for hit in cohors_fleet::search_content(path, &query, remaining + 1) {
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
    let has_token = ctx.token.is_some();
    let mut m = json!({
        "roots": ctx.roots,
        "config_path": ctx.config_path,
        "total": total,
        "errored": errored,
        // Visible on every call so the agent always knows whether the remote half
        // (CI / PRs) is live, instead of silently seeing fewer rows.
        "github_token": has_token,
    });
    if !has_token {
        m["github_note"] = json!(
            "No GitHub token — CI and PR data are unavailable (remote results will be empty). Run `gh auth login` (or set $GITHUB_TOKEN) and reconnect the server to enable them."
        );
    }
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
                // The concrete reasons (most urgent first), so the agent reads
                // "branch never pushed" / "↑2 unpushed · aging" instead of having
                // to re-derive the why from the raw fields.
                "reasons": assessment.reasons.iter().map(|r| r.label()).collect::<Vec<_>>(),
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

    /// Structural parity, MCP half (replaces the old hardcoded list): every action
    /// in the shared registry must have an MCP catalog tool whose description,
    /// confirm arg, and tier-gate match the `ActionDef`. Adding a verb to
    /// `cohors_actions::registry()` without generating its tool here fails this
    /// test. The TUI half is enforced symmetrically in `cohors-tui` (app::tests).
    #[test]
    fn registry_drives_catalog_parity() {
        let resp = call(json!({ "jsonrpc": "2.0", "id": 100, "method": "tools/list" }));
        let tools = resp["result"]["tools"].as_array().unwrap();
        let find = |name: &str| tools.iter().find(|t| t["name"] == name);

        for def in cohors_actions::registry() {
            // A tool exists for this verb, generated from the def.
            let tool =
                find(def.verb).unwrap_or_else(|| panic!("MCP catalog missing `{}`", def.verb));
            assert_eq!(
                tool["description"], def.summary,
                "`{}` catalog description drifted from the registry",
                def.verb
            );
            // needs_confirm ⇔ a `confirm` argument is offered.
            let has_confirm = tool["inputSchema"]["properties"].get("confirm").is_some();
            assert_eq!(
                has_confirm, def.needs_confirm,
                "`{}` confirm arg disagrees with the registry",
                def.verb
            );
            // The tier picks which gate the description must name.
            let desc = tool["description"].as_str().unwrap();
            match def.tier {
                cohors_actions::Tier::Write => assert!(
                    desc.contains("--allow-writes"),
                    "`{}` is Write tier but doesn't name --allow-writes",
                    def.verb
                ),
                cohors_actions::Tier::Run => assert!(
                    desc.contains("--allow-run"),
                    "`{}` is Run tier but doesn't name --allow-run",
                    def.verb
                ),
                other => panic!("action `{}` has unexpected tier {other:?}", def.verb),
            }
        }

        // The remote read tools are not registry-driven; keep asserting they ship.
        assert!(find("list_prs").is_some());
        assert!(find("ci_status").is_some());
    }

    #[test]
    fn push_is_gated_without_writes() {
        let resp = call(act("push", json!({ "selector": { "all": true } })));
        assert!(is_error(&resp));
        assert!(err_text(&resp).contains("--allow-writes"));
    }

    #[test]
    fn commit_gates_writes_then_confirm_then_message() {
        // Read-only server: blocked on the write tier first.
        let resp = call(act(
            "commit",
            json!({ "selector": { "all": true }, "message": "x" }),
        ));
        assert!(is_error(&resp));
        assert!(err_text(&resp).contains("--allow-writes"));

        // Writes on, but no confirm: still blocked.
        let resp = call_caps(
            act(
                "commit",
                json!({ "selector": { "all": true }, "message": "x" }),
            ),
            WRITES,
        );
        assert!(is_error(&resp));
        assert!(err_text(&resp).contains("confirm"));

        // Writes + confirm but no message: a clear argument error.
        let resp = call_caps(
            act(
                "commit",
                json!({ "selector": { "all": true }, "confirm": true }),
            ),
            WRITES,
        );
        assert!(is_error(&resp));
        assert!(err_text(&resp).contains("message"));
    }

    #[test]
    fn commit_dry_run_previews_on_read_only_server() {
        // Side-effect-free preview works with no tier and no confirm.
        let resp = call(act(
            "commit",
            json!({ "selector": { "all": true }, "message": "x", "dry_run": true }),
        ));
        assert!(!is_error(&resp));
        let p = payload(&resp);
        assert_eq!(p["dry_run"], true);
        assert!(p["targets"].as_u64().unwrap() >= 1);
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
}
