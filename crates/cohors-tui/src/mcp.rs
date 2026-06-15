//! The `cohors mcp` server — a Model Context Protocol surface over stdio so a
//! coding agent gets the same fleet view the dashboard does (ADR-023, ADR-025).
//!
//! Transport is a hand-rolled, synchronous JSON-RPC 2.0 loop over stdin/stdout
//! (newline-delimited messages, as the MCP stdio transport specifies). This
//! keeps the binary on the project's sync, no-tokio architecture (ADR-012) and
//! adds no new dependency. The tool layer is deliberately transport-agnostic, so
//! swapping in `rmcp` later is contained to [`run`].
//!
//! This slice ships the **read tools** (`list_repos`, `get_repo`,
//! `fleet_summary`, `repo_path`). Gated write/run tools and the remote/search
//! tools follow; until then they are simply absent from `tools/list`.

use std::io::{BufRead, Write};

use anyhow::Result;
use cohors_core::{RepoSnapshot, Selector, SortMode, assess, fleet_summary, resolve};
use serde_json::{Value, json};

/// The MCP protocol version we implement. We echo the client's requested
/// version when it sends one; this is the fallback.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Which tiers of tools are enabled, chosen by the human at launch (ADR-025).
/// Read tools are always on; everything else is opt-in. (No action tools exist
/// yet — these are threaded through for the next slice.)
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)] // read by the action-tool gating in the next slice.
pub struct Caps {
    pub allow_writes: bool,
    pub allow_run: bool,
    pub allow_open: bool,
}

/// A source of fleet snapshots — injected so tests can supply a fixture fleet
/// and the binary can supply a real scan.
type ScanFn<'a> = dyn Fn() -> Vec<RepoSnapshot> + 'a;

/// Run the stdio server loop until stdin closes. Each line is one JSON-RPC
/// message; each request gets exactly one response line, notifications none.
pub fn run(scan: &ScanFn<'_>, caps: Caps) -> Result<()> {
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
            Ok(request) => handle(&request, scan, caps, now_secs()),
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
fn handle(request: &Value, scan: &ScanFn<'_>, caps: Caps, now: i64) -> Option<Value> {
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
            match call_tool(name, &args, scan, caps, now) {
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
                    "selector": selector_schema,
                    "sort": { "type": "string", "enum": ["dirty-first", "recent", "name", "ahead-behind"] },
                    "fields": { "type": "array", "items": { "type": "string" }, "description": "Project each repo to these top-level fields (id and name always kept)." },
                    "limit": { "type": "integer", "minimum": 1 }
                }
            }
        },
        {
            "name": "get_repo",
            "description": "Get one repository's full status by id, name, or path.",
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
            "name": "repo_path",
            "description": "Resolve a repository (by id, name, or path) to its absolute path, so the agent can operate in it directly.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": { "type": "string" } },
                "required": ["repo"]
            }
        }
    ])
}

/// Execute a read tool, returning the JSON payload to embed (or an error message
/// for an `isError` tool result). Unknown tools and bad arguments are tool-level
/// errors, not protocol errors.
fn call_tool(
    name: &str,
    args: &Value,
    scan: &ScanFn<'_>,
    _caps: Caps,
    now: i64,
) -> Result<Value, String> {
    match name {
        "list_repos" => Ok(list_repos(args, scan, now)),
        "get_repo" => get_repo(args, scan, now),
        "fleet_summary" => {
            let snaps = scan();
            serde_json::to_value(fleet_summary(&snaps, now))
                .map_err(|e| format!("serializing fleet summary: {e}"))
        }
        "repo_path" => repo_path(args, scan),
        "fetch" | "pull" | "stash" | "run" | "open" => Err(format!(
            "`{name}` is not available: this build of the cohors MCP server is read-only."
        )),
        other => Err(format!("unknown tool `{other}`")),
    }
}

fn list_repos(args: &Value, scan: &ScanFn<'_>, now: i64) -> Value {
    let snaps = scan();

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

    json!({ "fleet": fleet_summary(&snaps, now), "repos": repos })
}

fn get_repo(args: &Value, scan: &ScanFn<'_>, now: i64) -> Result<Value, String> {
    let key = repo_arg(args)?;
    let snaps = scan();
    find_repo(&snaps, &key)
        .map(|snap| repo_json(snap, now))
        .ok_or_else(|| format!("no repository matching `{key}`"))
}

fn repo_path(args: &Value, scan: &ScanFn<'_>) -> Result<Value, String> {
    let key = repo_arg(args)?;
    let snaps = scan();
    let snap = find_repo(&snaps, &key).ok_or_else(|| format!("no repository matching `{key}`"))?;
    match &snap.path {
        Some(path) => Ok(json!({ "path": path })),
        None => Err(format!("repository `{key}` has no local path")),
    }
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
        handle(&request, &scan_fn, Caps::default(), NOW).expect("request expects a response")
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
        let resp = handle(
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            &(scan as fn() -> Vec<RepoSnapshot>),
            Caps::default(),
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
}
