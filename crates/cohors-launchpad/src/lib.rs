//! cohors-launchpad — the disk half of the launchpad contract.
//!
//! `cohors_core::launchpad` defines the contract (models, validation, flow
//! detection) without touching the filesystem. This crate does the I/O around
//! it:
//!
//! - **Per repo:** read `.principia/repo.json`, probe build manifests for
//!   zero-config flows, and write a validated contribution.
//! - **Per user:** the focus board (`board.json`) and the append-only activity
//!   log (`history/<day>.jsonl`) under the launchpad home (`~/.principia`).
//!
//! Three rules hold everywhere:
//!
//! 1. **Validate, then write.** An invalid document writes nothing and returns
//!    every error at once.
//! 2. **Writes are atomic.** A file is written to a sibling temp file and
//!    renamed into place, so a crash never leaves half a JSON document.
//! 3. **No panics on user data.** A missing, unreadable, or malformed file is a
//!    value (`Missing` / `Invalid`), never a crash — one bad repo must not take
//!    the launchpad down.

#![forbid(unsafe_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use cohors_core::launchpad::{
    ActivityEvent, Board, CONTRIB_DIR, CONTRIB_FILE, Contribution, LOCAL_FILE, LaunchpadEntry,
    Manifests, PHASES, SPEC_VERSION, Validation, detect_flows, validate_board,
    validate_contribution,
};
use cohors_core::time::iso8601_utc;
use serde_json::{Value, json};

/// An I/O failure, carrying the path it happened on.
#[derive(Debug, thiserror::Error)]
pub enum LaunchpadError {
    #[error("{action} {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0}")]
    Invalid(String),
}

fn io_err(action: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> LaunchpadError {
    let path = path.to_path_buf();
    move |source| LaunchpadError::Io {
        action,
        path,
        source,
    }
}

/// What a repo's `.principia/repo.json` turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContributionState {
    /// No contribution committed — the repo has not opted in.
    Missing,
    /// A file exists but cannot be used; the string says why.
    Invalid(String),
    /// Boxed so the enum stays small: the other variants are a word or two.
    Ok(Box<Contribution>),
}

/// The result of a validated write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOutcome {
    pub written: bool,
    pub validation: Validation,
    /// Where the document was (or would be) written.
    pub path: PathBuf,
}

impl WriteOutcome {
    /// The JSON shape every surface returns for a write.
    pub fn to_json(&self) -> Value {
        json!({
            "written": self.written,
            "errors": self.validation.errors,
            "warnings": self.validation.warnings,
            "path": self.path.display().to_string(),
        })
    }
}

// ── Per-repo: read ───────────────────────────────────────────────────────────

/// Read and parse a repo's committed contribution.
pub fn read_contribution(root: &Path) -> ContributionState {
    let file = root.join(CONTRIB_FILE);
    let body = match fs::read_to_string(&file) {
        Ok(body) => body,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ContributionState::Missing,
        Err(e) => return ContributionState::Invalid(format!("repo.json is unreadable: {e}")),
    };
    let doc: Value = match serde_json::from_str(&body) {
        Ok(doc) => doc,
        Err(e) => return ContributionState::Invalid(format!("repo.json is not valid JSON: {e}")),
    };
    match doc.get("specVersion").and_then(Value::as_u64) {
        Some(SPEC_VERSION) => {}
        other => {
            return ContributionState::Invalid(format!(
                "unsupported specVersion {}",
                other.map_or("(missing)".to_string(), |v| v.to_string())
            ));
        }
    }
    match serde_json::from_value::<Contribution>(doc) {
        Ok(contribution) => ContributionState::Ok(Box::new(contribution)),
        Err(e) => ContributionState::Invalid(format!("repo.json has the wrong shape: {e}")),
    }
}

/// Validate the contribution committed in `root` against the contract,
/// including that every declared agent file exists. `None` when the repo has
/// no contribution at all.
pub fn validate_repo(root: &Path) -> Option<Validation> {
    let body = match fs::read_to_string(root.join(CONTRIB_FILE)) {
        Ok(body) => body,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            return Some(Validation {
                errors: vec![format!("repo.json is unreadable: {e}")],
                warnings: Vec::new(),
            });
        }
    };
    Some(match serde_json::from_str::<Value>(&body) {
        Ok(doc) => validate_contribution(&doc, &|rel| root.join(rel).is_file()),
        Err(e) => Validation {
            errors: vec![format!("repo.json is not valid JSON: {e}")],
            warnings: Vec::new(),
        },
    })
}

/// Cap on detected Makefile targets, so a huge Makefile cannot flood a row.
const MAX_MAKE_TARGETS: usize = 12;

/// Look at a repo's build manifests. Cheap: a handful of `stat`s plus, at most,
/// reading `package.json` and `Makefile`.
pub fn probe_manifests(root: &Path) -> Manifests {
    let has = |name: &str| root.join(name).exists();

    let package_scripts = fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .and_then(|doc| {
            doc.get("scripts")
                .and_then(Value::as_object)
                .map(|scripts| scripts.keys().cloned().collect::<Vec<_>>())
        })
        .unwrap_or_default();
    let package_manager = if package_scripts.is_empty() {
        None
    } else if has("pnpm-lock.yaml") {
        Some("pnpm")
    } else if has("yarn.lock") {
        Some("yarn")
    } else if has("bun.lock") || has("bun.lockb") {
        Some("bun")
    } else {
        Some("npm")
    }
    .map(String::from);

    let make_targets = fs::read_to_string(root.join("Makefile"))
        .map(|body| parse_make_targets(&body))
        .unwrap_or_default();

    Manifests {
        package_scripts,
        package_manager,
        cargo: has("Cargo.toml"),
        python: has("pyproject.toml") || has("uv.lock"),
        make_targets,
        gradle: has("settings.gradle") || has("settings.gradle.kts"),
    }
}

/// Target names from a Makefile: lines shaped `name:` at column zero, skipping
/// special targets (`.PHONY`), pattern rules (`%.o`), and assignments (`x := y`).
fn parse_make_targets(body: &str) -> Vec<String> {
    let mut targets: Vec<String> = Vec::new();
    for line in body.lines() {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let is_name = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !is_name || rest.starts_with('=') || targets.iter().any(|t| t == name) {
            continue;
        }
        targets.push(name.to_string());
        if targets.len() == MAX_MAKE_TARGETS {
            break;
        }
    }
    targets
}

/// Everything the launchpad knows about one repo.
pub fn entry(id: &str, name: &str, root: &Path) -> LaunchpadEntry {
    let (configured, invalid, contribution) = match read_contribution(root) {
        ContributionState::Missing => (false, None, None),
        ContributionState::Invalid(why) => (false, Some(why), None),
        ContributionState::Ok(c) => (true, None, Some(*c)),
    };
    LaunchpadEntry {
        id: id.to_string(),
        // The repo's own declared name wins over the directory name.
        name: contribution
            .as_ref()
            .map(|c| c.name.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| name.to_string()),
        configured,
        invalid,
        contribution,
        detected_flows: detect_flows(&probe_manifests(root)),
    }
}

// ── Per-repo: write ──────────────────────────────────────────────────────────

/// Validate and write a repo's contribution, replacing the whole document.
///
/// `doc` is the document as supplied (an agent's tool arguments); `specVersion`
/// and `updated` are stamped here. With `dry_run`, validation runs and nothing
/// is written. Also makes sure `.principia/local.json` is gitignored.
pub fn write_contribution(
    root: &Path,
    doc: &Value,
    now_secs: i64,
    dry_run: bool,
) -> Result<WriteOutcome, LaunchpadError> {
    let path = root.join(CONTRIB_FILE);
    let Some(fields) = doc.as_object() else {
        return Err(LaunchpadError::Invalid(
            "the contribution must be a JSON object".to_string(),
        ));
    };
    let mut stamped = fields.clone();
    stamped.insert("specVersion".to_string(), json!(SPEC_VERSION));
    stamped.insert("updated".to_string(), json!(iso8601_utc(now_secs)));
    let stamped = Value::Object(stamped);

    let validation = validate_contribution(&stamped, &|rel| root.join(rel).is_file());
    if !validation.is_ok() || dry_run {
        return Ok(WriteOutcome {
            written: false,
            validation,
            path,
        });
    }

    let dir = root.join(CONTRIB_DIR);
    fs::create_dir_all(&dir).map_err(io_err("creating", &dir))?;
    write_json_atomic(&path, &stamped)?;
    ensure_gitignored(root)?;
    Ok(WriteOutcome {
        written: true,
        validation,
        path,
    })
}

/// Append `.principia/local.json` to the repo's `.gitignore` if it is not
/// already listed. Machine-local values must never be committed.
fn ensure_gitignored(root: &Path) -> Result<(), LaunchpadError> {
    let path = root.join(".gitignore");
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io_err("reading", &path)(e)),
    };
    if body.lines().any(|line| line.trim() == LOCAL_FILE) {
        return Ok(());
    }
    let separator = if body.is_empty() || body.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    fs::write(&path, format!("{body}{separator}{LOCAL_FILE}\n")).map_err(io_err("writing", &path))
}

/// Write pretty JSON to a temp sibling, then rename over the target.
fn write_json_atomic(path: &Path, value: &Value) -> Result<(), LaunchpadError> {
    let mut body = serde_json::to_vec_pretty(value)
        .map_err(|e| LaunchpadError::Invalid(format!("serializing JSON: {e}")))?;
    body.push(b'\n');
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body).map_err(io_err("writing", &tmp))?;
    fs::rename(&tmp, path).map_err(io_err("replacing", path))
}

// ── Per-user: home, board, activity ──────────────────────────────────────────

/// The launchpad home: `$PRINCIPIA_HOME`, else `~/.principia`. `None` when no
/// home directory can be determined (the caller reports it; nothing panics).
pub fn launchpad_home() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("PRINCIPIA_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join(".principia"))
}

/// Read the focus board. A missing or malformed board reads as empty — the
/// board is optional, and a bad file must not hide the rest of the launchpad.
pub fn read_board(home: &Path) -> Board {
    fs::read_to_string(home.join("board.json"))
        .ok()
        .and_then(|body| serde_json::from_str::<Board>(&body).ok())
        .unwrap_or(Board {
            spec_version: SPEC_VERSION,
            ..Board::default()
        })
}

/// Validate and write the focus board, replacing it. `focus` is the array of
/// focus items as supplied.
pub fn write_board(
    home: &Path,
    focus: &Value,
    now_secs: i64,
    dry_run: bool,
) -> Result<WriteOutcome, LaunchpadError> {
    let path = home.join("board.json");
    let doc = json!({
        "specVersion": SPEC_VERSION,
        "updated": iso8601_utc(now_secs),
        "focus": focus,
    });
    let validation = validate_board(&doc);
    if !validation.is_ok() || dry_run {
        return Ok(WriteOutcome {
            written: false,
            validation,
            path,
        });
    }
    fs::create_dir_all(home).map_err(io_err("creating", home))?;
    write_json_atomic(&path, &doc)?;
    Ok(WriteOutcome {
        written: true,
        validation,
        path,
    })
}

/// Append one event to today's activity log and return it.
pub fn record_activity(
    home: &Path,
    mut event: ActivityEvent,
    now_secs: i64,
) -> Result<ActivityEvent, LaunchpadError> {
    if !PHASES.contains(&event.phase.as_str()) {
        return Err(LaunchpadError::Invalid(format!(
            "phase must be one of {}",
            PHASES.join(", ")
        )));
    }
    event.ts = iso8601_utc(now_secs);
    if event.runner.is_empty() {
        event.runner = "unknown".to_string();
    }
    let dir = home.join("history");
    fs::create_dir_all(&dir).map_err(io_err("creating", &dir))?;
    // One file per UTC day: the date is the first 10 chars of the timestamp.
    let path = dir.join(format!("{}.jsonl", &event.ts[..10]));
    let mut line = serde_json::to_string(&event)
        .map_err(|e| LaunchpadError::Invalid(format!("serializing event: {e}")))?;
    line.push('\n');
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(line.as_bytes()))
        .map_err(io_err("appending to", &path))?;
    Ok(event)
}

/// The most recent activity events, newest first. Unparseable lines are
/// skipped rather than failing the read.
pub fn recent_activity(home: &Path, limit: usize) -> Vec<ActivityEvent> {
    let Ok(dir) = fs::read_dir(home.join("history")) else {
        return Vec::new();
    };
    let mut days: Vec<PathBuf> = dir
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "jsonl"))
        .collect();
    // File names are ISO dates, so a reverse lexical sort is newest-day first.
    days.sort();
    days.reverse();

    let mut events = Vec::new();
    for day in days {
        let Ok(body) = fs::read_to_string(&day) else {
            continue;
        };
        events.extend(
            body.lines()
                .rev()
                .filter_map(|line| serde_json::from_str::<ActivityEvent>(line).ok()),
        );
        if events.len() >= limit {
            break;
        }
    }
    events.truncate(limit);
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000; // 2023-11-14T22:13:20Z

    fn repo() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn write(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn missing_invalid_and_ok_contributions() {
        let dir = repo();
        assert_eq!(read_contribution(dir.path()), ContributionState::Missing);

        write(dir.path(), CONTRIB_FILE, "{ not json");
        assert!(
            matches!(read_contribution(dir.path()), ContributionState::Invalid(why) if why.contains("not valid JSON"))
        );

        write(
            dir.path(),
            CONTRIB_FILE,
            r#"{"specVersion": 9, "name": "x"}"#,
        );
        assert!(
            matches!(read_contribution(dir.path()), ContributionState::Invalid(why) if why.contains("unsupported specVersion 9"))
        );

        write(
            dir.path(),
            CONTRIB_FILE,
            r#"{"specVersion": 1, "name": "x", "tasks": "oops"}"#,
        );
        assert!(
            matches!(read_contribution(dir.path()), ContributionState::Invalid(why) if why.contains("wrong shape"))
        );

        write(
            dir.path(),
            CONTRIB_FILE,
            r#"{"specVersion": 1, "name": "svc"}"#,
        );
        assert!(
            matches!(read_contribution(dir.path()), ContributionState::Ok(c) if c.name == "svc")
        );
    }

    #[test]
    fn write_stamps_validates_and_gitignores() {
        let dir = repo();
        write(dir.path(), ".gitignore", "target/");
        let doc = json!({ "name": "svc", "summary": "A service", "flows": [] });

        let out = write_contribution(dir.path(), &doc, NOW, false).unwrap();
        assert!(out.written && out.validation.is_ok());

        let on_disk: Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join(CONTRIB_FILE)).unwrap())
                .unwrap();
        assert_eq!(on_disk["specVersion"], 1);
        assert_eq!(on_disk["updated"], "2023-11-14T22:13:20Z");
        assert_eq!(
            fs::read_to_string(dir.path().join(".gitignore")).unwrap(),
            "target/\n.principia/local.json\n"
        );
        // A second write must not duplicate the ignore line or leave a temp file.
        write_contribution(dir.path(), &doc, NOW, false).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(".gitignore"))
                .unwrap()
                .matches(LOCAL_FILE)
                .count(),
            1
        );
        assert!(!dir.path().join(".principia/repo.json.tmp").exists());
    }

    #[test]
    fn invalid_or_dry_run_writes_nothing() {
        let dir = repo();
        let bad = json!({ "name": "svc", "flows": [{ "id": "x", "label": "X", "run": "/abs" }] });
        let out = write_contribution(dir.path(), &bad, NOW, false).unwrap();
        assert!(!out.written);
        assert!(out.validation.errors[0].contains("absolute path"));
        assert!(!dir.path().join(CONTRIB_DIR).exists());

        let out = write_contribution(dir.path(), &json!({ "name": "svc" }), NOW, true).unwrap();
        assert!(!out.written && out.validation.is_ok());
        assert!(!dir.path().join(CONTRIB_DIR).exists());

        assert!(write_contribution(dir.path(), &json!([]), NOW, false).is_err());
    }

    #[test]
    fn agent_files_must_exist_on_disk() {
        let dir = repo();
        let doc = json!({ "name": "svc", "agents": [
            { "id": "triage", "label": "Triage", "file": ".principia/agents/triage.md" }
        ]});
        let out = write_contribution(dir.path(), &doc, NOW, false).unwrap();
        assert!(out.validation.errors[0].contains("file not found"));

        write(dir.path(), ".principia/agents/triage.md", "prompt");
        assert!(
            write_contribution(dir.path(), &doc, NOW, false)
                .unwrap()
                .written
        );
        assert_eq!(validate_repo(dir.path()), Some(Validation::default()));
    }

    #[test]
    fn validate_repo_reports_missing_and_broken() {
        let dir = repo();
        assert_eq!(validate_repo(dir.path()), None);
        write(dir.path(), CONTRIB_FILE, "nope");
        assert!(!validate_repo(dir.path()).unwrap().is_ok());
    }

    #[test]
    fn manifests_are_probed_and_detected() {
        let dir = repo();
        write(
            dir.path(),
            "package.json",
            r#"{"scripts": {"dev": "vite", "test": "vitest"}}"#,
        );
        write(dir.path(), "pnpm-lock.yaml", "");
        write(dir.path(), "Cargo.toml", "");
        write(
            dir.path(),
            "Makefile",
            ".PHONY: build\nCC := gcc\nbuild: deps\n\tcc x\n%.o: %.c\ntest:\nbuild:\n",
        );
        let m = probe_manifests(dir.path());
        assert_eq!(m.package_manager.as_deref(), Some("pnpm"));
        assert_eq!(m.make_targets, ["build", "test"]);
        assert!(m.cargo && !m.python && !m.gradle);

        let e = entry("id-1", "dir-name", dir.path());
        assert!(!e.configured && e.invalid.is_none());
        assert_eq!(e.name, "dir-name");
        assert!(e.detected_flows.iter().any(|f| f.run == "pnpm run dev"));

        // Malformed package.json is ignored, not fatal.
        write(dir.path(), "package.json", "{");
        assert!(probe_manifests(dir.path()).package_scripts.is_empty());
    }

    #[test]
    fn entry_prefers_the_declared_name() {
        let dir = repo();
        write(
            dir.path(),
            CONTRIB_FILE,
            r#"{"specVersion": 1, "name": "Payments API"}"#,
        );
        let e = entry("id-1", "pay", dir.path());
        assert!(e.configured);
        assert_eq!(e.name, "Payments API");
    }

    #[test]
    fn board_round_trip_and_rejection() {
        let home = repo();
        assert!(read_board(home.path()).focus.is_empty());

        let focus =
            json!([{ "id": "f-1", "title": "Ship retries", "repo": "pay", "status": "doing" }]);
        assert!(
            write_board(home.path(), &focus, NOW, true)
                .map(|o| !o.written)
                .unwrap()
        );
        assert!(read_board(home.path()).focus.is_empty());

        assert!(
            write_board(home.path(), &focus, NOW, false)
                .unwrap()
                .written
        );
        let board = read_board(home.path());
        assert_eq!(board.focus[0].title, "Ship retries");
        assert_eq!(board.updated.as_deref(), Some("2023-11-14T22:13:20Z"));

        let bad = json!([{ "id": "Bad Id", "title": "" }]);
        let out = write_board(home.path(), &bad, NOW, false).unwrap();
        assert!(!out.written && out.validation.errors.len() == 2);
        assert_eq!(
            read_board(home.path()).focus.len(),
            1,
            "the good board survives"
        );

        // A corrupt board reads as empty rather than erroring.
        fs::write(home.path().join("board.json"), "garbage").unwrap();
        assert!(read_board(home.path()).focus.is_empty());
    }

    #[test]
    fn activity_appends_and_reads_newest_first() {
        let home = repo();
        assert!(recent_activity(home.path(), 10).is_empty());
        let event = |phase: &str, repo: &str| ActivityEvent {
            phase: phase.into(),
            runner: "claude-code".into(),
            repo: repo.into(),
            ..ActivityEvent::default()
        };
        record_activity(home.path(), event("start", "a"), NOW).unwrap();
        record_activity(home.path(), event("end", "a"), NOW + 60).unwrap();
        record_activity(home.path(), event("start", "b"), NOW + 86_400).unwrap();
        // A junk line in the log is skipped.
        let day = home.path().join("history/2023-11-14.jsonl");
        fs::OpenOptions::new()
            .append(true)
            .open(&day)
            .unwrap()
            .write_all(b"junk\n")
            .unwrap();

        let recent = recent_activity(home.path(), 10);
        let seen: Vec<(&str, &str)> = recent
            .iter()
            .map(|e| (e.repo.as_str(), e.phase.as_str()))
            .collect();
        assert_eq!(seen, [("b", "start"), ("a", "end"), ("a", "start")]);
        assert_eq!(recent_activity(home.path(), 1).len(), 1);

        assert!(record_activity(home.path(), event("bogus", "a"), NOW).is_err());
    }
}
