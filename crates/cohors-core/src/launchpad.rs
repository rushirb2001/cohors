//! The launchpad contract — what a repository says about *itself*.
//!
//! The rest of the core answers "what state is each repo in?". This module
//! answers "what can I *do* in each repo, and what work is waiting there?". A
//! repository opts in by committing a `.principia/repo.json` describing its
//! **flows** (things to run), **agents** (prompt files it ships), and **tasks**
//! (short-lived, evidence-backed work). A user-level **board** carries focus
//! across repos, and an append-only **activity** log records agent sessions.
//!
//! This is the Principia contract, spec v1, implemented natively so one binary
//! serves both the fleet and the launchpad (ADR-052). The on-disk format is
//! unchanged, so repos already carrying a `.principia/` directory keep working.
//!
//! Everything here is pure: models, validation, and flow detection take data
//! in and return data out. Reading and writing the files is the job of the
//! `cohors-launchpad` adapter crate.
//!
//! ## Why validation works on raw JSON
//!
//! The typed structs below are for *reading* (they are lenient: every field has
//! a default). Validation instead walks a `serde_json::Value`, because an agent
//! that wrote a bad document needs the **whole list** of problems in one reply,
//! not the first type error serde happens to hit.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The contract version this build reads and writes.
pub const SPEC_VERSION: u64 = 1;
/// The per-repo directory a contribution lives in (committed).
pub const CONTRIB_DIR: &str = ".principia";
/// The committed contribution file, relative to the repo root.
pub const CONTRIB_FILE: &str = ".principia/repo.json";
/// The machine-local file that must never be committed.
pub const LOCAL_FILE: &str = ".principia/local.json";

/// Agent runners a repo-declared agent may claim to support.
pub const RUNNERS: [&str; 4] = ["claude-code", "codex", "gemini-cli", "agy"];
/// Allowed `flows[].kind` values.
pub const FLOW_KINDS: [&str; 5] = ["server", "task", "test", "build", "tool"];
/// Allowed task / focus `status` values.
pub const STATUSES: [&str; 4] = ["todo", "doing", "blocked", "done"];
/// Allowed `tasks[].priority` values.
pub const PRIORITIES: [&str; 3] = ["high", "normal", "low"];
/// Allowed activity `phase` values.
pub const PHASES: [&str; 4] = ["start", "end", "launch", "note"];

// ── Models (lenient, for reading) ────────────────────────────────────────────

/// A link shown beside a repo (docs, dashboards, trackers).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Link {
    pub label: String,
    pub url: String,
}

/// Something the user can start in a repo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Flow {
    pub id: String,
    pub label: String,
    /// Shell command, run from the repo root (or `cwd`). Never absolute.
    pub run: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u32>,
    #[serde(skip_serializing_if = "is_false")]
    pub background: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub primary: bool,
    /// `"detected"` for flows inferred from manifests; absent for declared ones.
    /// Detected flows live in memory only and are never written to disk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// A prompt file the repo ships for recurring, repo-specific work.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentDecl {
    pub id: String,
    pub label: String,
    /// Path under `.principia/`, relative to the repo root.
    pub file: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub supports: Vec<String>,
}

/// A short-lived piece of work the repo knows about.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

impl Task {
    /// The status, defaulting to `todo` when the file omits it.
    pub fn status(&self) -> &str {
        self.status.as_deref().unwrap_or("todo")
    }

    /// True for anything not yet `done`.
    pub fn is_open(&self) -> bool {
        self.status() != "done"
    }

    /// Sort key: lower is more urgent (`doing` → `blocked` → `todo` → `done`,
    /// then high → normal → low priority).
    pub fn rank(&self) -> (u8, u8) {
        let status = match self.status() {
            "doing" => 0,
            "blocked" => 1,
            "todo" => 2,
            _ => 3,
        };
        let priority = match self.priority.as_deref().unwrap_or("normal") {
            "high" => 0,
            "low" => 2,
            _ => 1,
        };
        (status, priority)
    }
}

/// A repository's committed `.principia/repo.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Contribution {
    pub spec_version: u64,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub group: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    pub flows: Vec<Flow>,
    pub agents: Vec<AgentDecl>,
    pub tasks: Vec<Task>,
    /// ISO-8601 UTC timestamp of the last write.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    /// Commit the author read when writing this, so staleness can be shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_from: Option<String>,
}

/// One repo as the launchpad sees it: its contribution (if any) plus whatever
/// could be inferred with zero config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct LaunchpadEntry {
    pub id: String,
    pub name: String,
    /// True when a valid `.principia/repo.json` is committed.
    pub configured: bool,
    /// Why an existing contribution could not be used (bad JSON, wrong version).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contribution: Option<Contribution>,
    /// Flows inferred from manifests (npm/cargo/uv/make/gradle). Never on disk.
    pub detected_flows: Vec<Flow>,
}

impl LaunchpadEntry {
    /// Tasks not yet done, most urgent first.
    pub fn open_tasks(&self) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self
            .contribution
            .iter()
            .flat_map(|c| c.tasks.iter())
            .filter(|t| t.is_open())
            .collect();
        tasks.sort_by_key(|t| t.rank());
        tasks
    }
}

/// One focus item on the cross-repo board.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FocusItem {
    pub id: String,
    pub title: String,
    /// Matches a contribution's `name` — never a path, so a board syncs across
    /// machines where checkouts live in different places.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// A task id inside that repo.
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
}

/// The user-level focus board (`~/.principia/board.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Board {
    pub spec_version: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    pub focus: Vec<FocusItem>,
}

/// One line of the activity log (`~/.principia/history/<day>.jsonl`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActivityEvent {
    pub ts: String,
    pub phase: String,
    pub runner: String,
    pub repo: String,
    pub branch: String,
    pub agent: String,
    pub source: String,
}

fn is_false(b: &bool) -> bool {
    !*b
}

// ── Validation ───────────────────────────────────────────────────────────────

/// The outcome of validating a document: `errors` block a write, `warnings`
/// do not.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Validation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Validation {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// `^[a-z0-9]+(-[a-z0-9]+)*$`, written out by hand to avoid a regex dependency
/// in the core.
pub fn is_kebab(id: &str) -> bool {
    !id.is_empty()
        && id.split('-').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// True when a path/command starts absolute or home-relative: `/x`, `~/x`, or a
/// Windows drive (`C:`). Committed files must contain none of these.
pub fn is_absolute_like(s: &str) -> bool {
    let s = s.trim_start();
    let mut chars = s.chars();
    match chars.next() {
        Some('/') | Some('~') => true,
        Some(c) if c.is_ascii_alphabetic() => chars.next() == Some(':'),
        _ => false,
    }
}

/// Non-empty string field, or `None`.
fn text<'a>(item: &'a Value, key: &str) -> Option<&'a str> {
    item.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

/// The array under `key`, or an empty slice. A present-but-not-an-array value
/// is reported as an error.
fn list<'a>(doc: &'a Value, key: &str, errors: &mut Vec<String>) -> &'a [Value] {
    match doc.get(key) {
        None | Some(Value::Null) => &[],
        Some(Value::Array(items)) => items,
        Some(_) => {
            errors.push(format!("{key} must be an array"));
            &[]
        }
    }
}

/// Every `id` in `items` is present, kebab-case, and unique.
fn check_ids(items: &[Value], what: &str, errors: &mut Vec<String>) {
    let mut seen: Vec<&str> = Vec::new();
    for item in items {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            errors.push(format!("{what}: every entry needs a string id"));
            continue;
        };
        if !is_kebab(id) {
            errors.push(format!("{what}: id \"{id}\" must be kebab-case"));
        }
        if seen.contains(&id) {
            errors.push(format!("{what}: duplicate id \"{id}\""));
        }
        seen.push(id);
    }
}

fn label_of(item: &Value) -> &str {
    item.get("id").and_then(Value::as_str).unwrap_or("?")
}

fn check_enum(item: &Value, key: &str, allowed: &[&str], at: &str, errors: &mut Vec<String>) {
    if let Some(value) = item.get(key).filter(|v| !v.is_null()) {
        match value.as_str() {
            Some(s) if allowed.contains(&s) => {}
            _ => errors.push(format!("{at}: {key} must be one of {}", allowed.join(", "))),
        }
    }
}

/// Validate a `.principia/repo.json` document against spec v1.
///
/// `file_exists` answers whether a repo-relative path exists, so the "agent
/// file must exist" rule can be checked without this crate touching the disk.
/// Pass `&|_| true` to skip that check (e.g. validating a document in the
/// abstract).
pub fn validate_contribution(doc: &Value, file_exists: &dyn Fn(&str) -> bool) -> Validation {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    if !doc.is_object() {
        errors.push("the document must be a JSON object".to_string());
        return Validation { errors, warnings };
    }
    if doc.get("specVersion").and_then(Value::as_u64) != Some(SPEC_VERSION) {
        errors.push(format!(
            "specVersion must be {SPEC_VERSION}, got {}",
            doc.get("specVersion").unwrap_or(&Value::Null)
        ));
    }
    if text(doc, "name").is_none() {
        errors.push("name is required".to_string());
    }
    if let Some(summary) = doc.get("summary").and_then(Value::as_str) {
        let len = summary.chars().count();
        if len > 80 {
            errors.push(format!("summary is {len} chars, max 80"));
        }
    }

    let flows = list(doc, "flows", &mut errors);
    check_ids(flows, "flows", &mut errors);
    let mut primaries = 0;
    for flow in flows {
        let at = format!("flows[{}]", label_of(flow));
        match text(flow, "run") {
            None => errors.push(format!("{at}: run is required")),
            Some(run) if is_absolute_like(run) => {
                errors.push(format!("{at}: run must not start with an absolute path"))
            }
            Some(_) => {}
        }
        if text(flow, "label").is_none() {
            errors.push(format!("{at}: label is required"));
        }
        if let Some(cwd) = flow.get("cwd").and_then(Value::as_str)
            && (is_absolute_like(cwd) || cwd.split(['/', '\\']).any(|seg| seg == ".."))
        {
            errors.push(format!("{at}: cwd must be relative to the repo root"));
        }
        check_enum(flow, "kind", &FLOW_KINDS, &at, &mut errors);
        if flow.get("primary").and_then(Value::as_bool) == Some(true) {
            primaries += 1;
        }
    }
    if primaries > 1 {
        errors.push(format!("only one flow may be primary, found {primaries}"));
    }

    let agents = list(doc, "agents", &mut errors);
    check_ids(agents, "agents", &mut errors);
    for agent in agents {
        let at = format!("agents[{}]", label_of(agent));
        if let Some(supports) = agent.get("supports").and_then(Value::as_array) {
            for runner in supports {
                match runner.as_str() {
                    Some(r) if RUNNERS.contains(&r) => {}
                    _ => errors.push(format!("{at}: unknown runner {runner}")),
                }
            }
        }
        if text(agent, "label").is_none() {
            errors.push(format!("{at}: label is required"));
        }
        let Some(file) = text(agent, "file") else {
            errors.push(format!("{at}: file is required"));
            continue;
        };
        if is_absolute_like(file) {
            errors.push(format!("{at}: file must be relative to the repo root"));
        } else if file.split(['/', '\\']).any(|seg| seg == "..") {
            errors.push(format!("{at}: file must not contain \"..\""));
        } else if !(file.starts_with(".principia/") || file.starts_with(".principia\\")) {
            errors.push(format!("{at}: file must live under .principia/"));
        } else if !file_exists(file) {
            errors.push(format!("{at}: file not found: {file}"));
        }
    }

    let tasks = list(doc, "tasks", &mut errors);
    check_ids(tasks, "tasks", &mut errors);
    for task in tasks {
        let at = format!("tasks[{}]", label_of(task));
        if text(task, "title").is_none() {
            errors.push(format!("{at}: title is required"));
        }
        check_enum(task, "status", &STATUSES, &at, &mut errors);
        check_enum(task, "priority", &PRIORITIES, &at, &mut errors);
        if let Some(notes) = task.get("notes").and_then(Value::as_str)
            && notes.chars().count() > 400
        {
            errors.push(format!("{at}: notes exceed 400 chars"));
        }
        if let Some(agent) = task.get("agent").and_then(Value::as_str) {
            if !is_kebab(agent) {
                errors.push(format!("{at}: agent \"{agent}\" must be kebab-case"));
            } else if !agents
                .iter()
                .any(|a| a.get("id").and_then(Value::as_str) == Some(agent))
            {
                warnings.push(format!(
                    "{at}: agent \"{agent}\" does not match any declared agents[] id"
                ));
            }
        }
    }

    if doc.get("updated").is_none() {
        warnings.push("no updated timestamp".to_string());
    }
    Validation { errors, warnings }
}

/// Validate a board document (`~/.principia/board.json`).
pub fn validate_board(doc: &Value) -> Validation {
    let mut errors = Vec::new();
    if !doc.is_object() {
        errors.push("the document must be a JSON object".to_string());
        return Validation {
            errors,
            warnings: Vec::new(),
        };
    }
    if doc.get("specVersion").and_then(Value::as_u64) != Some(SPEC_VERSION) {
        errors.push(format!("specVersion must be {SPEC_VERSION}"));
    }
    let focus = list(doc, "focus", &mut errors);
    check_ids(focus, "focus", &mut errors);
    for item in focus {
        let at = format!("focus[{}]", label_of(item));
        if text(item, "title").is_none() {
            errors.push(format!("{at}: title is required"));
        }
        check_enum(item, "status", &STATUSES, &at, &mut errors);
        if let Some(repo) = item.get("repo").and_then(Value::as_str)
            && is_absolute_like(repo)
        {
            errors.push(format!("{at}: repo must be a repo name, never a path"));
        }
    }
    Validation {
        errors,
        warnings: Vec::new(),
    }
}

/// The contract, as data an agent can read instead of guessing the schema.
pub fn contract() -> Value {
    json!({
        "specVersion": SPEC_VERSION,
        "file": CONTRIB_FILE,
        "rules": [
            "name is required; summary is at most 80 characters",
            "every id (flows/agents/tasks) is unique and kebab-case",
            "flows[].run and flows[].cwd must be relative, never absolute",
            "at most one flow may be primary",
            "agents[].file must live under .principia/ and must exist",
            format!("agents[].supports may only contain: {}", RUNNERS.join(", ")),
            format!("flows[].kind is one of: {}", FLOW_KINDS.join(", ")),
            format!("tasks[].status is one of: {}", STATUSES.join(", ")),
            format!("tasks[].priority is one of: {}", PRIORITIES.join(", ")),
            "tasks[].notes is at most 400 characters",
            "tasks[].agent, if set, is kebab-case and should name one of this repo's own agents[] ids",
            "no absolute paths, secrets, or machine-specific values anywhere in this file — it is committed",
        ],
        "judgement": [
            "Detection already covers npm/cargo/uv/Makefile/gradle with zero config; declare a flow only when it adds a label, a port, an ordering, a composite command or a cwd that detection cannot infer",
            "An empty flows array is a correct answer, and better than flows that restate package.json",
            "Add an agent only for recurring, repo-specific work; most repositories need none",
            "Tasks come from evidence (uncommitted work, unpushed commits, real TODOs, failing tests), never from invention",
            "Never set status done without a commit, a merged PR, or the user saying so",
        ],
        "machineLocal": {
            "file": LOCAL_FILE,
            "gitignored": true,
            "purpose": "Values true only on this machine (e.g. task → agent-session links). Never committed.",
        },
        "note": "write_repo_contribution validates against exactly these rules before writing anything. A failed validation writes nothing; a successful one REPLACES the whole document — read the current one first.",
    })
}

// ── Zero-config flow detection ───────────────────────────────────────────────

/// What a repo's build manifests say, gathered by an adapter. Pure input for
/// [`detect_flows`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifests {
    /// Script names from `package.json`.
    pub package_scripts: Vec<String>,
    /// `npm` | `pnpm` | `yarn` | `bun`, inferred from the lockfile.
    pub package_manager: Option<String>,
    pub cargo: bool,
    /// `pyproject.toml` or `uv.lock` present.
    pub python: bool,
    pub make_targets: Vec<String>,
    pub gradle: bool,
}

/// Infer runnable flows from manifests, so a repo is useful on the launchpad
/// before anyone configures it. Every result is marked `source: "detected"`.
pub fn detect_flows(m: &Manifests) -> Vec<Flow> {
    let mut flows = Vec::new();
    let mut push = |id: String, label: String, run: String, kind: &str| {
        flows.push(Flow {
            id,
            label,
            run,
            kind: Some(kind.to_string()),
            source: Some("detected".to_string()),
            ..Flow::default()
        });
    };

    let pm = m.package_manager.as_deref().unwrap_or("npm");
    for (script, kind) in [
        ("dev", "server"),
        ("start", "server"),
        ("build", "build"),
        ("test", "test"),
        ("lint", "task"),
    ] {
        if m.package_scripts.iter().any(|s| s == script) {
            push(
                format!("{pm}-{script}"),
                format!("{pm} run {script}"),
                format!("{pm} run {script}"),
                kind,
            );
        }
    }
    if m.cargo {
        for (cmd, kind) in [
            ("check", "task"),
            ("build", "build"),
            ("test", "test"),
            ("run", "task"),
        ] {
            push(
                format!("cargo-{cmd}"),
                format!("cargo {cmd}"),
                format!("cargo {cmd}"),
                kind,
            );
        }
    }
    if m.python {
        push("uv-sync".into(), "uv sync".into(), "uv sync".into(), "task");
        push(
            "uv-pytest".into(),
            "uv run pytest".into(),
            "uv run pytest".into(),
            "test",
        );
    }
    for target in &m.make_targets {
        // Make targets are free-form; keep only those that form a valid id.
        let id = format!(
            "make-{}",
            target.to_ascii_lowercase().replace(['_', '.'], "-")
        );
        if is_kebab(&id) {
            push(
                id,
                format!("make {target}"),
                format!("make {target}"),
                "task",
            );
        }
    }
    if m.gradle {
        push(
            "gradle-assemble-debug".into(),
            "gradlew assembleDebug".into(),
            "./gradlew assembleDebug".into(),
            "build",
        );
    }
    flows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Value {
        json!({
            "specVersion": 1,
            "name": "pay-api",
            "summary": "Payments service",
            "updated": "2026-01-01T00:00:00Z",
            "flows": [{ "id": "dev", "label": "Dev server", "run": "npm run dev", "primary": true }],
            "agents": [{ "id": "triage", "label": "Triage", "file": ".principia/agents/triage.md",
                         "supports": ["claude-code"] }],
            "tasks": [{ "id": "retry-webhooks", "title": "Retry webhooks", "status": "todo",
                        "priority": "high", "agent": "triage" }],
        })
    }

    fn errors(doc: &Value) -> Vec<String> {
        validate_contribution(doc, &|_| true).errors
    }

    #[test]
    fn a_valid_document_passes_cleanly() {
        let v = validate_contribution(&valid(), &|_| true);
        assert_eq!(v, Validation::default());
    }

    #[test]
    fn minimal_document_is_valid_with_a_staleness_warning() {
        let v = validate_contribution(&json!({ "specVersion": 1, "name": "x" }), &|_| true);
        assert!(v.is_ok());
        assert_eq!(v.warnings, vec!["no updated timestamp"]);
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let doc = json!({
            "specVersion": 2,
            "summary": "x".repeat(81),
            "flows": [
                { "id": "Dev", "label": "a", "run": "/usr/bin/x", "primary": true },
                { "id": "b", "run": "ok", "cwd": "../up", "kind": "nope", "primary": true },
                { "id": "b", "label": "dup", "run": "ok" },
            ],
            "tasks": [{ "id": "t", "status": "finished", "priority": "urgent" }],
        });
        let e = errors(&doc);
        for needle in [
            "specVersion must be 1",
            "name is required",
            "summary is 81 chars",
            "id \"Dev\" must be kebab-case",
            "run must not start with an absolute path",
            "flows[b]: label is required",
            "cwd must be relative",
            "kind must be one of",
            "duplicate id \"b\"",
            "only one flow may be primary, found 2",
            "tasks[t]: title is required",
            "status must be one of",
            "priority must be one of",
        ] {
            assert!(
                e.iter().any(|m| m.contains(needle)),
                "missing: {needle}\n{e:#?}"
            );
        }
    }

    #[test]
    fn agent_files_are_confined_and_must_exist() {
        let with_file = |file: &str| {
            json!({ "specVersion": 1, "name": "x",
                    "agents": [{ "id": "a", "label": "A", "file": file }] })
        };
        assert!(errors(&with_file("/etc/passwd"))[0].contains("relative to the repo root"));
        assert!(errors(&with_file(".principia/../x.md"))[0].contains("\"..\""));
        assert!(errors(&with_file("docs/a.md"))[0].contains("under .principia/"));
        let missing = validate_contribution(&with_file(".principia/agents/a.md"), &|_| false);
        assert!(missing.errors[0].contains("file not found"));
    }

    #[test]
    fn unknown_runner_and_dangling_task_agent() {
        let doc = json!({
            "specVersion": 1, "name": "x", "updated": "t",
            "agents": [{ "id": "a", "label": "A", "file": ".principia/a.md", "supports": ["cursor"] }],
            "tasks": [{ "id": "t", "title": "T", "agent": "ghost" }],
        });
        let v = validate_contribution(&doc, &|_| true);
        assert!(v.errors[0].contains("unknown runner"));
        assert!(v.warnings[0].contains("does not match any declared agents"));
    }

    #[test]
    fn non_object_and_non_array_shapes_do_not_panic() {
        assert!(!validate_contribution(&json!([1, 2]), &|_| true).is_ok());
        let v = validate_contribution(
            &json!({ "specVersion": 1, "name": "x", "flows": "nope", "tasks": [3] }),
            &|_| true,
        );
        assert!(v.errors.iter().any(|m| m == "flows must be an array"));
        assert!(v.errors.iter().any(|m| m.contains("needs a string id")));
    }

    #[test]
    fn kebab_and_absolute_helpers() {
        assert!(is_kebab("a-b2-c"));
        for bad in ["", "A", "a--b", "-a", "a-", "a_b"] {
            assert!(!is_kebab(bad), "{bad}");
        }
        for abs in ["/x", "~/x", "C:\\x", " /x"] {
            assert!(is_absolute_like(abs), "{abs}");
        }
        for rel in ["npm run dev", "./x", "cargo test", "c"] {
            assert!(!is_absolute_like(rel), "{rel}");
        }
    }

    #[test]
    fn board_validation() {
        assert!(validate_board(&json!({ "specVersion": 1, "focus": [] })).is_ok());
        let v = validate_board(&json!({
            "specVersion": 1,
            "focus": [{ "id": "f-1", "status": "later", "repo": "/abs/path" }],
        }));
        assert_eq!(v.errors.len(), 3, "{:#?}", v.errors);
    }

    #[test]
    fn contribution_round_trips_and_reads_leniently() {
        let c: Contribution = serde_json::from_value(valid()).unwrap();
        assert_eq!(c.name, "pay-api");
        assert!(c.flows[0].primary);
        assert_eq!(c.tasks[0].rank(), (2, 0));
        // camelCase on the wire, and unset optionals stay off the wire.
        let back = serde_json::to_value(&c).unwrap();
        assert_eq!(back["specVersion"], 1);
        assert!(back["flows"][0].get("cwd").is_none());
        // A sparse file still reads.
        let sparse: Contribution = serde_json::from_value(json!({ "name": "x" })).unwrap();
        assert!(sparse.tasks.is_empty());
    }

    #[test]
    fn open_tasks_are_ranked_and_drop_done() {
        let task = |id: &str, status: &str, priority: &str| Task {
            id: id.into(),
            title: id.into(),
            status: Some(status.into()),
            priority: Some(priority.into()),
            ..Task::default()
        };
        let entry = LaunchpadEntry {
            contribution: Some(Contribution {
                tasks: vec![
                    task("low-todo", "todo", "low"),
                    task("finished", "done", "high"),
                    task("high-todo", "todo", "high"),
                    task("active", "doing", "normal"),
                ],
                ..Contribution::default()
            }),
            ..LaunchpadEntry::default()
        };
        let ids: Vec<&str> = entry.open_tasks().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["active", "high-todo", "low-todo"]);
    }

    #[test]
    fn detection_covers_each_stack() {
        let flows = detect_flows(&Manifests {
            package_scripts: vec!["dev".into(), "test".into(), "deploy".into()],
            package_manager: Some("pnpm".into()),
            cargo: true,
            python: true,
            make_targets: vec!["build_all".into(), "weird target".into()],
            gradle: true,
        });
        let ids: Vec<&str> = flows.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "pnpm-dev",
                "pnpm-test",
                "cargo-check",
                "cargo-build",
                "cargo-test",
                "cargo-run",
                "uv-sync",
                "uv-pytest",
                "make-build-all",
                "gradle-assemble-debug",
            ]
        );
        assert!(
            flows
                .iter()
                .all(|f| f.source.as_deref() == Some("detected"))
        );
        assert!(
            flows
                .iter()
                .all(|f| is_kebab(&f.id) && !is_absolute_like(&f.run))
        );
        assert!(detect_flows(&Manifests::default()).is_empty());
    }
}
