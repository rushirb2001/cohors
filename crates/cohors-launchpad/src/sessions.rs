//! Finding the conversations coding agents held in each repository, so cohors
//! can suggest which one to resume (ADR-062).
//!
//! Read-only, best-effort, bounded:
//! - **Claude Code** keeps one JSON-lines file per session under
//!   `~/.claude/projects/<folder>/<session-id>.jsonl`, where `<folder>` is the
//!   working directory with every character outside `[A-Za-z0-9]` turned into
//!   `-`. Only folders whose name starts with a repository's escaped path are
//!   opened, so unrelated sessions are never read.
//! - **Codex** keeps `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`, whose
//!   first line (`session_meta`) names the id and working directory. Only the
//!   days inside the horizon are listed.
//!
//! Of each file only the first [`HEAD_BYTES`] and the last [`TAIL_BYTES`] are
//! read: enough for the folder, the branch, the title and the last prompt.
//! Nothing is written, and nothing about a session leaves this machine.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use cohors_core::suggest::{AgentSession, SESSION_HORIZON_SECS};
use serde_json::Value;

/// Bytes read from the start of a session file.
const HEAD_BYTES: u64 = 256 * 1024;
/// Bytes read from the end of a large session file (later titles, prompts).
const TAIL_BYTES: u64 = 96 * 1024;
/// At most this many session files are opened per call, newest first.
const MAX_FILES: usize = 400;
/// A title longer than this is clipped.
const TITLE_CHARS: usize = 100;

/// The agent sessions held in `repos` (each `(id, absolute path)`), newest
/// first, from the stores under `home`. A session belongs to the deepest
/// repository that contains its working directory.
pub fn agent_sessions(repos: &[(String, PathBuf)], home: &Path, now: i64) -> Vec<AgentSession> {
    let repos: Vec<(String, PathBuf)> = repos
        .iter()
        .map(|(id, p)| {
            (
                id.clone(),
                fs::canonicalize(p).unwrap_or_else(|_| p.clone()),
            )
        })
        .collect();
    let since = now - SESSION_HORIZON_SECS;
    let mut files: Vec<(i64, &'static str, PathBuf)> = Vec::new();
    files.extend(
        claude_files(&home.join(".claude").join("projects"), &repos, since)
            .into_iter()
            .map(|(t, p)| (t, "claude-code", p)),
    );
    files.extend(
        codex_files(&home.join(".codex").join("sessions"), since, now)
            .into_iter()
            .map(|(t, p)| (t, "codex", p)),
    );
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.truncate(MAX_FILES);

    let mut out: Vec<AgentSession> = Vec::new();
    for (mtime, runner, path) in files {
        let parsed = match runner {
            "claude-code" => parse_claude(&path),
            _ => parse_codex(&path),
        };
        let Some(found) = parsed else { continue };
        // The id goes into a command line: only real session ids (hex and
        // dashes, as Claude Code and Codex write them) are kept.
        if !cohors_core::agents::is_session_id(&found.id) {
            continue;
        }
        let cwd = PathBuf::from(&found.cwd);
        let cwd = fs::canonicalize(&cwd).unwrap_or(cwd);
        let Some((repo, _)) = repos
            .iter()
            .filter(|(_, root)| cwd.starts_with(root))
            .max_by_key(|(_, root)| root.as_os_str().len())
        else {
            continue;
        };
        if out.iter().any(|s| s.id == found.id && s.runner == runner) {
            continue;
        }
        out.push(AgentSession {
            runner: runner.to_string(),
            id: found.id,
            repo: repo.clone(),
            cwd: cwd.to_string_lossy().into_owned(),
            branch: found.branch,
            title: found.title,
            last_active: mtime,
        });
    }
    out
}

/// Claude Code's folder name for a working directory.
pub fn claude_folder(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Session files under Claude Code's projects folder that may belong to one
/// of `repos` (their folder name starts with a repo's escaped path) and were
/// active since `since`.
fn claude_files(projects: &Path, repos: &[(String, PathBuf)], since: i64) -> Vec<(i64, PathBuf)> {
    let prefixes: Vec<String> = repos.iter().map(|(_, p)| claude_folder(p)).collect();
    let Ok(dirs) = fs::read_dir(projects) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dir in dirs.flatten() {
        let name = dir.file_name().to_string_lossy().into_owned();
        if !prefixes
            .iter()
            .any(|p| name == *p || name.starts_with(&format!("{p}-")))
        {
            continue;
        }
        let Ok(entries) = fs::read_dir(dir.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "jsonl")
                && let Some(t) = mtime(&path)
                && t >= since
            {
                out.push((t, path));
            }
        }
    }
    out
}

/// Codex rollout files from the days inside the horizon.
fn codex_files(sessions: &Path, since: i64, now: i64) -> Vec<(i64, PathBuf)> {
    let mut out = Vec::new();
    let mut day = now;
    // One folder per calendar day: walk back day by day over the horizon.
    while day >= since - 86_400 {
        let (y, m, d) = civil_from_days(day.div_euclid(86_400));
        let folder = sessions
            .join(format!("{y:04}"))
            .join(format!("{m:02}"))
            .join(format!("{d:02}"));
        if let Ok(entries) = fs::read_dir(&folder) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "jsonl")
                    && let Some(t) = mtime(&path)
                    && t >= since
                {
                    out.push((t, path));
                }
            }
        }
        day -= 86_400;
    }
    out
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian (Howard
/// Hinnant's algorithm), so no date crate is needed.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn mtime(path: &Path) -> Option<i64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64)
}

/// What a session file tells us.
struct Found {
    id: String,
    cwd: String,
    branch: Option<String>,
    title: String,
}

/// The complete JSON lines of a file's head and (when large) its tail.
fn lines_of(path: &Path) -> Vec<Value> {
    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut chunks: Vec<String> = Vec::new();
    let mut head = Vec::new();
    if (&mut file).take(HEAD_BYTES).read_to_end(&mut head).is_ok() {
        chunks.push(String::from_utf8_lossy(&head).into_owned());
    }
    if len > HEAD_BYTES + TAIL_BYTES && file.seek(SeekFrom::End(-(TAIL_BYTES as i64))).is_ok() {
        let mut tail = Vec::new();
        if file.take(TAIL_BYTES).read_to_end(&mut tail).is_ok() {
            // The first line of the tail is cut: drop it.
            let text = String::from_utf8_lossy(&tail).into_owned();
            if let Some((_, rest)) = text.split_once('\n') {
                chunks.push(rest.to_string());
            }
        }
    }
    chunks
        .iter()
        .flat_map(|c| c.lines())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

/// One line of text: control characters and newlines out, clipped.
fn tidy(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if flat.chars().count() > TITLE_CHARS {
        let mut cut: String = flat.chars().take(TITLE_CHARS - 1).collect();
        cut.push('…');
        cut
    } else {
        flat
    }
}

/// Text a person typed: a string, or the text parts of a content array.
/// Markup the tool injects (`<command-name>…`, `<environment_context>…`) is
/// not a prompt.
fn prompt_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| {
                let kind = p.get("type").and_then(Value::as_str)?;
                matches!(kind, "text" | "input_text")
                    .then(|| p.get("text").and_then(Value::as_str))
                    .flatten()
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty() && !text.starts_with('<')).then(|| tidy(text))
}

fn parse_claude(path: &Path) -> Option<Found> {
    let id = path.file_stem()?.to_string_lossy().into_owned();
    let lines = lines_of(path);
    let (mut cwd, mut branch, mut custom, mut last, mut first) =
        (None, None, None::<String>, None::<String>, None::<String>);
    for line in &lines {
        if cwd.is_none() {
            cwd = line.get("cwd").and_then(Value::as_str).map(str::to_string);
        }
        if let Some(b) = line.get("gitBranch").and_then(Value::as_str)
            && !b.is_empty()
            && b != "HEAD"
        {
            branch = Some(b.to_string());
        }
        match line.get("type").and_then(Value::as_str) {
            Some("custom-title") => {
                custom = line.get("customTitle").and_then(Value::as_str).map(tidy);
            }
            Some("last-prompt") => {
                last = line
                    .get("lastPrompt")
                    .and_then(Value::as_str)
                    .filter(|p| !p.trim_start().starts_with('<'))
                    .map(tidy);
            }
            Some("user") if first.is_none() => {
                let meta = line.get("isMeta").and_then(Value::as_bool).unwrap_or(false);
                if !meta {
                    first = line
                        .get("message")
                        .and_then(|m| m.get("content"))
                        .and_then(prompt_text);
                }
            }
            _ => {}
        }
    }
    let title = custom
        .filter(|t| !t.is_empty())
        .or(last.filter(|t| !t.is_empty()))
        .or(first)?;
    Some(Found {
        id,
        cwd: cwd?,
        branch,
        title,
    })
}

fn parse_codex(path: &Path) -> Option<Found> {
    let lines = lines_of(path);
    let meta = lines
        .iter()
        .find(|l| l.get("type").and_then(Value::as_str) == Some("session_meta"))?
        .get("payload")?;
    let id = meta.get("id").and_then(Value::as_str)?.to_string();
    let cwd = meta.get("cwd").and_then(Value::as_str)?.to_string();
    let branch = meta
        .get("git")
        .and_then(|g| g.get("branch"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut prompts = lines.iter().filter_map(|l| {
        let payload = l.get("payload")?;
        match (
            l.get("type").and_then(Value::as_str),
            payload.get("type").and_then(Value::as_str),
        ) {
            (Some("event_msg"), Some("user_message")) => payload
                .get("message")
                .and_then(Value::as_str)
                .map(|s| Value::String(s.into())),
            (Some("response_item"), Some("message"))
                if payload.get("role").and_then(Value::as_str) == Some("user") =>
            {
                payload.get("content").cloned()
            }
            _ => None,
        }
        .and_then(|c| prompt_text(&c))
    });
    let first = prompts.next();
    let last = prompts.next_back();
    Some(Found {
        id,
        cwd,
        branch,
        title: last.or(first)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn write(path: &Path, lines: &[Value]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        fs::write(path, body.join("\n") + "\n").unwrap();
    }

    #[test]
    fn claude_sessions_are_found_titled_and_tied_to_the_deepest_repo() {
        let home = tempfile::tempdir().unwrap();
        let code = home.path().join("code");
        let api = code.join("projects").join("api");
        fs::create_dir_all(&api).unwrap();
        let (code, api) = (
            fs::canonicalize(&code).unwrap(),
            fs::canonicalize(&api).unwrap(),
        );
        let projects = home.path().join(".claude").join("projects");
        // A session in the nested repo, renamed by the user.
        write(
            &projects
                .join(claude_folder(&api))
                .join("aaaa1111-2222-4333-8444-555566667777.jsonl"),
            &[
                serde_json::json!({"type":"user","cwd": api, "gitBranch":"feat/x","message":{"content":"<command-name>/clear</command-name>"}}),
                serde_json::json!({"type":"user","cwd": api, "gitBranch":"feat/x","message":{"content":[{"type":"text","text":"Fix the login\nredirect"}]}}),
                serde_json::json!({"type":"custom-title","customTitle":"Login redirect"}),
            ],
        );
        // A session in the outer repo with only a last prompt.
        write(
            &projects
                .join(claude_folder(&code))
                .join("bbbb1111-2222-4333-8444-555566667777.jsonl"),
            &[
                serde_json::json!({"type":"user","cwd": code, "message":{"content":"tidy notes"}}),
                serde_json::json!({"type":"last-prompt","lastPrompt":"update the roadmap"}),
            ],
        );
        // An unrelated project is never opened (no repo prefix).
        write(
            &projects.join("-elsewhere").join("cccc.jsonl"),
            &[serde_json::json!({"type":"user","cwd":"/elsewhere","message":{"content":"x"}})],
        );
        let repos = vec![
            ("code".to_string(), code.clone()),
            ("api".to_string(), api.clone()),
        ];
        let now = mtime(
            &projects
                .join(claude_folder(&api))
                .join("aaaa1111-2222-4333-8444-555566667777.jsonl"),
        )
        .unwrap();
        let mut found = agent_sessions(&repos, home.path(), now);
        found.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].id, "aaaa1111-2222-4333-8444-555566667777");
        assert_eq!(found[0].repo, "api", "the deepest repo wins");
        assert_eq!(found[0].title, "Login redirect");
        assert_eq!(found[0].branch.as_deref(), Some("feat/x"));
        assert_eq!(found[1].repo, "code");
        assert_eq!(found[1].title, "update the roadmap");
        assert_eq!(found[1].runner, "claude-code");
    }

    #[test]
    fn codex_sessions_come_from_the_dated_folders() {
        let home = tempfile::tempdir().unwrap();
        let repo = home.path().join("app");
        fs::create_dir_all(&repo).unwrap();
        let repo = fs::canonicalize(&repo).unwrap();
        let now = mtime(home.path()).unwrap_or(NOW);
        let (y, m, d) = civil_from_days(now.div_euclid(86_400));
        let file = home.path().join(format!(
            ".codex/sessions/{y:04}/{m:02}/{d:02}/rollout-x.jsonl"
        ));
        write(
            &file,
            &[
                serde_json::json!({"type":"session_meta","payload":{"id":"0c0dfeed-1234-4abc-8def-0123456789ab","cwd":repo,"git":{"branch":"main"}}}),
                serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>…"}]}}),
                serde_json::json!({"type":"event_msg","payload":{"type":"user_message","message":"add retries"}}),
            ],
        );
        let found = agent_sessions(&[("app".into(), repo)], home.path(), now + 5);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].runner, "codex");
        assert_eq!(found[0].id, "0c0dfeed-1234-4abc-8def-0123456789ab");
        assert_eq!(found[0].title, "add retries");
        assert_eq!(found[0].branch.as_deref(), Some("main"));
    }

    /// A session file whose id is not a session id (it would land in a
    /// command line) is skipped.
    #[test]
    fn a_session_with_a_strange_id_is_skipped() {
        let home = tempfile::tempdir().unwrap();
        let repo = home.path().join("app");
        fs::create_dir_all(&repo).unwrap();
        let repo = fs::canonicalize(&repo).unwrap();
        let folder = home
            .path()
            .join(".claude/projects")
            .join(claude_folder(&repo));
        write(
            &folder.join("x;rm -rf ~.jsonl"),
            &[serde_json::json!({"type":"user","cwd": repo,"message":{"content":"hi"}})],
        );
        let now = mtime(&folder.join("x;rm -rf ~.jsonl")).unwrap();
        assert!(agent_sessions(&[("app".into(), repo)], home.path(), now).is_empty());
    }

    #[test]
    fn old_sessions_and_missing_stores_are_quietly_empty() {
        let home = tempfile::tempdir().unwrap();
        assert!(
            agent_sessions(&[("r".into(), home.path().to_path_buf())], home.path(), NOW).is_empty()
        );
    }

    #[test]
    fn dates_convert_like_a_calendar() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_513), (2026, 3, 1));
    }

    #[test]
    fn titles_are_one_clipped_line_without_control_characters() {
        let long = "a".repeat(300);
        assert_eq!(tidy(&long).chars().count(), TITLE_CHARS);
        assert_eq!(tidy("x\x1b[2J\ny"), "x [2J y");
    }
}
