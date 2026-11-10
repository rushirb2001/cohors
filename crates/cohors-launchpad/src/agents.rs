//! Starting agent runners: the disk and process half.
//!
//! `cohors_core::agents` knows the runners and the prompts but touches
//! nothing. This module does what needs a machine:
//!
//! - [`detect_runners`] — which runner executables are installed.
//! - [`plan_launch`] — turn "set this repo up", "work on this task", "run this
//!   repo's agent" into the exact command to start, writing the prompt file,
//!   remembering which conversation a task belongs to, and logging the launch.
//!
//! Everything a repository supplies (an agent's prompt file, a task title) is
//! treated as untrusted: file paths are confined to the repo's `.principia/`
//! directory, ids must be kebab-case, and nothing repo-authored ever reaches a
//! command line except as the *contents* of a prompt file.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use cohors_core::agents::{
    self, Launch, PromptContext, RUNNERS, Runner, RunnerStatus, Shell, is_session_id,
};
use cohors_core::launchpad::{ActivityEvent, CONTRIB_DIR, LOCAL_FILE, SPEC_VERSION, is_kebab};
use cohors_core::time::iso8601_utc;
use serde_json::{Value, json};

use super::{
    ContributionState, LaunchpadError, ensure_gitignored, io_err, read_board, read_contribution,
    record_activity, refuse_symlink, write_json_atomic,
};

/// Prompt files older than this are deleted the next time one is written.
const PROMPT_TTL_SECS: i64 = 24 * 60 * 60;

// ── Finding runners ──────────────────────────────────────────────────────────

/// Find `bin` in the directories of a PATH-style string. On Windows the usual
/// executable extensions are tried too.
pub fn find_on_path(bin: &str, path_var: &std::ffi::OsStr) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        ["", ".exe", ".cmd", ".bat"]
            .iter()
            .map(|ext| format!("{bin}{ext}"))
            .collect()
    } else {
        vec![bin.to_string()]
    };
    std::env::split_paths(path_var)
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// The PATH a login shell would have. A process started by an editor or a
/// launcher often has a shorter PATH than the user's terminal, and would miss
/// anything a shell profile adds — which is where most runners are installed.
/// Best-effort: `None` on Windows, or when the shell does not answer in time.
fn login_shell_path() -> Option<std::ffi::OsString> {
    if cfg!(windows) {
        return None;
    }
    let shell = std::env::var_os("SHELL").filter(|s| !s.is_empty())?;
    let mut child = std::process::Command::new(shell)
        .args(["-lc", "printf %s \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    // A profile that waits for input must not hang cohors: give it 3 seconds.
    for _ in 0..30 {
        match child.try_wait() {
            Ok(Some(_)) => {
                let out = child.wait_with_output().ok()?;
                let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                return (!path.is_empty()).then(|| path.into());
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

/// Which runners are installed. Every runner is listed; one that is missing
/// says why, so a front-end can show it disabled instead of hiding it.
pub fn detect_runners() -> Vec<RunnerStatus> {
    let process_path = std::env::var_os("PATH").unwrap_or_default();
    let find = |r: &Runner, path: &std::ffi::OsStr| {
        find_on_path(r.bin, path).map(|p| p.display().to_string())
    };
    let mut found: Vec<Option<String>> = RUNNERS.iter().map(|r| find(r, &process_path)).collect();
    // Only ask a login shell (which costs a process) when something is missing.
    if found.iter().any(Option::is_none)
        && let Some(login) = login_shell_path()
    {
        for (slot, runner) in found.iter_mut().zip(&RUNNERS) {
            if slot.is_none() {
                *slot = find(runner, &login);
            }
        }
    }
    RUNNERS
        .iter()
        .zip(found)
        .map(|(runner, path)| RunnerStatus::new(runner, path))
        .collect()
}

// ── Task ↔ conversation links ────────────────────────────────────────────────

/// The conversations this machine has linked to a repo's tasks, from the
/// gitignored `.principia/local.json` (`{"threads": {"<task id>": "<session id>"}}`).
/// A session id is machine-specific, which is why it is never committed.
pub fn read_threads(root: &Path) -> BTreeMap<String, String> {
    let Ok(body) = fs::read_to_string(root.join(LOCAL_FILE)) else {
        return BTreeMap::new();
    };
    serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|doc| doc.get("threads").cloned())
        .and_then(|threads| serde_json::from_value(threads).ok())
        .unwrap_or_default()
}

/// Remember that `task_id` is being worked on in conversation `session`.
/// Other keys already in `local.json` are kept.
pub fn write_thread(
    root: &Path,
    task_id: &str,
    session: &str,
    now_secs: i64,
) -> Result<(), LaunchpadError> {
    let path = root.join(LOCAL_FILE);
    let mut doc = fs::read_to_string(&path)
        .ok()
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    doc["specVersion"] = json!(SPEC_VERSION);
    doc["updated"] = json!(iso8601_utc(now_secs));
    if !doc["threads"].is_object() {
        doc["threads"] = json!({});
    }
    doc["threads"][task_id] = json!(session);

    let dir = root.join(CONTRIB_DIR);
    refuse_symlink(&dir)?;
    fs::create_dir_all(&dir).map_err(io_err("creating", &dir))?;
    refuse_symlink(&path)?;
    write_json_atomic(&path, &doc)?;
    ensure_gitignored(root)
}

// ── Prompt files ─────────────────────────────────────────────────────────────

/// Write `body` to a fresh prompt file under the launchpad home and return its
/// path. A file (rather than an argument) keeps the command line short and
/// identical across runners. Old prompt files are cleared as a side effect.
pub fn write_prompt_file(
    home: &Path,
    tag: &str,
    body: &str,
    now_secs: i64,
) -> Result<PathBuf, LaunchpadError> {
    let dir = home.join("prompts");
    fs::create_dir_all(&dir).map_err(io_err("creating", &dir))?;
    prune_prompts(&dir, now_secs);
    // Repo-supplied text must never reach a file name: keep letters, digits,
    // dash and underscore only.
    let safe: String = tag
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(60)
        .collect();
    let safe = if safe.is_empty() { "prompt" } else { &safe };
    let path = dir.join(format!("{safe}-{now_secs}-{}.md", std::process::id()));
    refuse_symlink(&path)?;
    fs::write(&path, body).map_err(io_err("writing", &path))?;
    Ok(path)
}

/// Delete prompt files older than [`PROMPT_TTL_SECS`]. Best-effort.
fn prune_prompts(dir: &Path, now_secs: i64) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let age = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| now_secs - d.as_secs() as i64);
        if age.is_some_and(|age| age > PROMPT_TTL_SECS) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Resolve a repo-declared agent file, refusing anything outside the repo's
/// `.principia/` directory. The declaration comes from a repository we may not
/// have written, so `..` segments and symlinks must not lead out of it.
fn resolve_agent_file(root: &Path, rel: &str) -> Result<PathBuf, LaunchpadError> {
    let refuse = || {
        LaunchpadError::Invalid(format!(
            "refusing agent file `{rel}`: it must be a file inside {CONTRIB_DIR}/"
        ))
    };
    let base = fs::canonicalize(root.join(CONTRIB_DIR)).map_err(|_| refuse())?;
    let target = fs::canonicalize(root.join(rel))
        .map_err(|_| LaunchpadError::Invalid(format!("agent file is missing: {rel}")))?;
    if target.starts_with(&base) && target.is_file() {
        Ok(target)
    } else {
        Err(refuse())
    }
}

/// A new session id, formatted like a version-4 UUID. Built from the standard
/// library's per-process random hashing keys plus the clock and process id, so
/// no extra dependency is needed; it only has to be unique, not secret.
pub fn new_session_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = |salt: u64| {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(salt);
        hasher.write_u32(std::process::id());
        if let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            hasher.write_u128(now.as_nanos());
        }
        hasher.finish()
    };
    let (a, b) = (part(1), part(2));
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (a >> 32) as u32,
        (a >> 16) as u16,
        (a as u16) & 0x0fff,
        ((b >> 48) as u16 & 0x3fff) | 0x8000,
        b & 0xffff_ffff_ffff
    )
}

// ── Planning a launch ────────────────────────────────────────────────────────

/// What to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchKind {
    /// One of the prompts cohors ships (`setup-repo`, `plan-day`, …).
    Prompt(String),
    /// An agent the repo declares in its `.principia/repo.json`.
    Agent(String),
    /// A task from the repo's `.principia/repo.json` (or a focus-board item).
    Task(String),
    /// Just open a session in the repo.
    Start,
    /// Re-enter a session by id.
    Resume(String),
}

/// The repository a launch happens in.
#[derive(Debug, Clone, Copy)]
pub struct RepoTarget<'a> {
    pub name: &'a str,
    pub root: &'a Path,
    pub branch: &'a str,
}

/// Everything [`plan_launch`] needs.
#[derive(Debug, Clone)]
pub struct LaunchRequest<'a> {
    pub kind: LaunchKind,
    /// A specific runner id, or `None` for the first installed one.
    pub runner: Option<&'a str>,
    /// Required for everything except a fleet-wide prompt.
    pub repo: Option<RepoTarget<'a>>,
    /// `(name, path)` of every repo, listed in fleet-wide prompts.
    pub fleet: &'a [(String, String)],
    /// Where to start when there is no repo (a fleet-wide prompt).
    pub fallback_cwd: &'a Path,
}

/// The exact thing to run, in both forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// One line for a terminal (the prompt is read from `prompt_file`).
    pub command: String,
    /// The executable to spawn directly…
    pub program: String,
    /// …and its arguments, with the prompt text inline. No shell involved.
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub runner: String,
    /// A name for the terminal tab, e.g. `Claude Code: setup`.
    pub label: String,
    pub prompt_file: Option<PathBuf>,
    /// Whether the conversation was started under a session id cohors chose,
    /// and so can be found again.
    pub linked: bool,
    pub session: Option<String>,
}

impl LaunchPlan {
    /// The plan as the JSON `cohors agent command` prints.
    pub fn to_json(&self) -> Value {
        json!({
            "command": self.command,
            "cwd": self.cwd,
            "runner": self.runner,
            "label": self.label,
            "prompt_file": self.prompt_file,
            "linked": self.linked,
            "session": self.session,
        })
    }
}

fn invalid(message: impl Into<String>) -> LaunchpadError {
    LaunchpadError::Invalid(message.into())
}

/// Choose the runner: the one asked for (which must be installed), else the
/// first installed one the agent supports (`allowed` empty = any).
fn pick_runner<'a>(
    wanted: Option<&str>,
    allowed: &[String],
    statuses: &'a [RunnerStatus],
) -> Result<(&'static Runner, &'a RunnerStatus), LaunchpadError> {
    let supports = |id: &str| allowed.is_empty() || allowed.iter().any(|a| a == id);
    let status = match wanted {
        Some(id) => {
            let status = statuses
                .iter()
                .find(|s| s.id == id)
                .ok_or_else(|| invalid(format!("unknown runner `{id}`")))?;
            if !status.available {
                return Err(invalid(format!(
                    "{} is unavailable: {}",
                    status.label,
                    status.reason.as_deref().unwrap_or("not installed")
                )));
            }
            if !supports(id) {
                return Err(invalid(format!(
                    "this agent does not support {} (it supports: {})",
                    status.label,
                    allowed.join(", ")
                )));
            }
            status
        }
        None => statuses
            .iter()
            .find(|s| s.available && supports(&s.id))
            .ok_or_else(|| {
                let looked_for: Vec<&str> = statuses
                    .iter()
                    .filter(|s| supports(&s.id))
                    .map(|s| s.bin.as_str())
                    .collect();
                invalid(format!(
                    "no supported agent runner is installed (looked for: {})",
                    looked_for.join(", ")
                ))
            })?,
    };
    let runner = agents::runner(&status.id).ok_or_else(|| invalid("runner table out of date"))?;
    Ok((runner, status))
}

/// Work out exactly what to run for `request`.
///
/// Side effects, all under `home` or the repo's gitignored `local.json`: the
/// prompt file is written, a task started under a chosen session id has that
/// link recorded, and a `launch` event is appended to the activity log (which
/// is best-effort — a full disk must not stop someone starting work).
pub fn plan_launch(
    home: &Path,
    request: &LaunchRequest<'_>,
    statuses: &[RunnerStatus],
    now_secs: i64,
) -> Result<LaunchPlan, LaunchpadError> {
    let need_repo = || {
        request
            .repo
            .ok_or_else(|| invalid("this needs a repository: pass --repo <path>"))
    };
    let ctx_of = |repo: &RepoTarget<'_>, task_title: &str| {
        (
            repo.name.to_string(),
            repo.root.display().to_string(),
            repo.branch.to_string(),
            task_title.to_string(),
        )
    };

    // Each arm settles: which runners are allowed, the working directory, what
    // the terminal is called, the prompt (if any), how to launch, what to log.
    struct Step {
        allowed: Vec<String>,
        cwd: PathBuf,
        what: String,
        tag: String,
        prompt: Option<String>,
        resume: Option<String>,
        /// The task to link a new conversation to.
        link_task: Option<String>,
        log_as: String,
    }

    let step = match &request.kind {
        LaunchKind::Prompt(id) => {
            let prompt = agents::prompt(id).ok_or_else(|| {
                let known: Vec<&str> = agents::PROMPTS.iter().map(|p| p.id).collect();
                invalid(format!(
                    "unknown prompt `{id}` (known: {})",
                    known.join(", ")
                ))
            })?;
            let (cwd, ctx) = if prompt.needs_repo {
                let repo = need_repo()?;
                (repo.root.to_path_buf(), ctx_of(&repo, ""))
            } else {
                (request.fallback_cwd.to_path_buf(), Default::default())
            };
            let board = home.join("board.json").display().to_string();
            let body = agents::render_prompt(
                prompt,
                &PromptContext {
                    repo_name: &ctx.0,
                    repo_root: &ctx.1,
                    branch: &ctx.2,
                    task_title: &ctx.3,
                },
                request.fleet,
                &board,
            );
            Step {
                allowed: Vec::new(),
                cwd,
                what: prompt.title.to_string(),
                tag: id.clone(),
                prompt: Some(body),
                resume: None,
                link_task: None,
                log_as: id.clone(),
            }
        }
        LaunchKind::Agent(agent_id) => {
            let repo = need_repo()?;
            let (allowed, what, body) = declared_agent(&repo, agent_id, None)?;
            Step {
                allowed,
                cwd: repo.root.to_path_buf(),
                what,
                tag: format!("agent-{agent_id}"),
                prompt: Some(body),
                resume: None,
                link_task: None,
                log_as: agent_id.clone(),
            }
        }
        LaunchKind::Task(task_id) => {
            let repo = need_repo()?;
            if !is_kebab(task_id) {
                return Err(invalid(format!(
                    "refusing task id `{task_id}`: it must be kebab-case"
                )));
            }
            let contribution = match read_contribution(repo.root) {
                ContributionState::Ok(c) => Some(*c),
                _ => None,
            };
            let board = read_board(home);
            let repo_task = contribution
                .as_ref()
                .and_then(|c| c.tasks.iter().find(|t| &t.id == task_id).cloned());
            // A focus-board item can stand in for a task the repo never wrote down.
            let focus = board.focus.iter().find(|f| &f.id == task_id);
            let task = repo_task
                .or_else(|| {
                    focus.map(|f| cohors_core::launchpad::Task {
                        id: f.id.clone(),
                        title: f.title.clone(),
                        agent: f.agent.clone(),
                        ..Default::default()
                    })
                })
                .ok_or_else(|| invalid(format!("no task `{task_id}` in {}", repo.name)))?;

            let ships_agent = |id: &str| {
                contribution
                    .as_ref()
                    .is_some_and(|c| c.agents.iter().any(|a| a.id == id))
            };
            let log_as = format!("task:{task_id}");
            match task.agent.as_deref().filter(|a| ships_agent(a)) {
                // The repo ships an agent for this task: run it, with the task attached.
                Some(agent_id) => {
                    let (allowed, _, body) = declared_agent(&repo, agent_id, Some(&task))?;
                    Step {
                        allowed,
                        cwd: repo.root.to_path_buf(),
                        what: task.title.clone(),
                        tag: format!("task-{task_id}"),
                        prompt: Some(body),
                        resume: None,
                        link_task: None,
                        log_as,
                    }
                }
                None => {
                    let ctx = ctx_of(&repo, &task.title);
                    let body = agents::task_prompt(
                        &PromptContext {
                            repo_name: &ctx.0,
                            repo_root: &ctx.1,
                            branch: &ctx.2,
                            task_title: &ctx.3,
                        },
                        &task,
                    );
                    // Already linked to a conversation? Go back to it instead of
                    // starting a cold one with none of the context.
                    let existing = focus
                        .and_then(|f| f.thread.clone())
                        .or_else(|| read_threads(repo.root).get(task_id).cloned())
                        .filter(|id| is_session_id(id));
                    Step {
                        allowed: Vec::new(),
                        cwd: repo.root.to_path_buf(),
                        what: task.title.clone(),
                        tag: format!("task-{task_id}"),
                        prompt: Some(body),
                        resume: existing,
                        link_task: Some(task_id.clone()),
                        log_as,
                    }
                }
            }
        }
        LaunchKind::Start => {
            let repo = need_repo()?;
            Step {
                allowed: Vec::new(),
                cwd: repo.root.to_path_buf(),
                what: repo.name.to_string(),
                tag: String::new(),
                prompt: None,
                resume: None,
                link_task: None,
                log_as: "new-session".to_string(),
            }
        }
        LaunchKind::Resume(session) => {
            let repo = need_repo()?;
            if !is_session_id(session) {
                return Err(invalid("that does not look like a session id"));
            }
            Step {
                allowed: Vec::new(),
                cwd: repo.root.to_path_buf(),
                what: "resume".to_string(),
                tag: String::new(),
                prompt: None,
                resume: Some(session.clone()),
                link_task: None,
                log_as: "resume".to_string(),
            }
        }
    };

    let (runner, status) = pick_runner(request.runner, &step.allowed, statuses)?;
    let program = status
        .path
        .clone()
        .unwrap_or_else(|| runner.bin.to_string());
    let shell = Shell::native();

    // Decide how to enter the session.
    let explicit_resume = matches!(request.kind, LaunchKind::Resume(_));
    let (launch_session, resuming) = match &step.resume {
        Some(session) if runner.can_resume() => (Some(session.clone()), true),
        Some(_) if explicit_resume => {
            return Err(invalid(format!(
                "{} cannot reopen a session by id",
                runner.label
            )));
        }
        // A linked task on a runner that cannot resume starts fresh.
        _ => (None, false),
    };

    let mut plan = LaunchPlan {
        command: runner.bin.to_string(),
        program,
        args: Vec::new(),
        cwd: step.cwd,
        runner: runner.id.to_string(),
        label: format!("{}: {}", runner.label, step.what),
        prompt_file: None,
        linked: false,
        session: None,
    };

    if resuming {
        let session = launch_session.unwrap_or_default();
        let launch = Launch::Resume { session: &session };
        plan.command = runner
            .shell_command(shell, launch, "")
            .unwrap_or_else(|| runner.bin.to_string());
        plan.args = runner.argv(launch, "").unwrap_or_default();
        plan.linked = true;
        plan.session = Some(session);
    } else if let Some(body) = &step.prompt {
        let file = write_prompt_file(home, &step.tag, body, now_secs)?;
        let file_text = file.display().to_string();
        // Start under a session id we choose when the runner allows it and
        // there is a task to link, so the conversation can be found again.
        let chosen = step
            .link_task
            .as_ref()
            .filter(|_| runner.can_link())
            .map(|_| new_session_id());
        let launch = match &chosen {
            Some(session) => Launch::Linked { session },
            None => Launch::Start,
        };
        plan.command = runner
            .shell_command(shell, launch, &file_text)
            .ok_or_else(|| invalid("runner cannot start a prompt"))?;
        plan.args = runner.argv(launch, body).unwrap_or_default();
        plan.prompt_file = Some(file);
        if let (Some(session), Some(task_id), Some(repo)) = (&chosen, &step.link_task, request.repo)
        {
            // Recording the link is best-effort: the launch still works without it.
            if let Err(e) = write_thread(repo.root, task_id, session, now_secs) {
                tracing::warn!("could not record the task's conversation: {e}");
            } else {
                plan.linked = true;
            }
            plan.session = chosen;
        }
    }

    let event = ActivityEvent {
        phase: "launch".to_string(),
        runner: runner.id.to_string(),
        repo: plan.cwd.display().to_string(),
        branch: request
            .repo
            .map(|r| r.branch.to_string())
            .unwrap_or_default(),
        agent: step.log_as,
        source: "cohors".to_string(),
        ..Default::default()
    };
    if let Err(e) = record_activity(home, event, now_secs) {
        tracing::warn!("could not record the launch: {e}");
    }
    Ok(plan)
}

/// Load a repo-declared agent: who may run it, what to call it, and its prompt
/// with placeholders filled (and `task` attached when started from one).
fn declared_agent(
    repo: &RepoTarget<'_>,
    agent_id: &str,
    task: Option<&cohors_core::launchpad::Task>,
) -> Result<(Vec<String>, String, String), LaunchpadError> {
    if !is_kebab(agent_id) {
        return Err(invalid(format!(
            "refusing agent id `{agent_id}`: it must be kebab-case"
        )));
    }
    let ContributionState::Ok(contribution) = read_contribution(repo.root) else {
        return Err(invalid(format!(
            "{} has no valid {CONTRIB_DIR}/repo.json",
            repo.name
        )));
    };
    let agent = contribution
        .agents
        .iter()
        .find(|a| a.id == agent_id)
        .ok_or_else(|| invalid(format!("no agent `{agent_id}` in {}", repo.name)))?;
    let file = resolve_agent_file(repo.root, &agent.file)?;
    let body = fs::read_to_string(&file).map_err(io_err("reading", &file))?;
    let root = repo.root.display().to_string();
    let ctx = PromptContext {
        repo_name: repo.name,
        repo_root: &root,
        branch: repo.branch,
        task_title: task.map(|t| t.title.as_str()).unwrap_or(""),
    };
    let label = if agent.label.is_empty() {
        agent_id.to_string()
    } else {
        agent.label.clone()
    };
    Ok((
        agent.supports.clone(),
        label,
        agents::agent_prompt(&body, &ctx, task),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    /// Every runner "installed" at a fake path, so plans are deterministic.
    fn all_installed() -> Vec<RunnerStatus> {
        RUNNERS
            .iter()
            .map(|r| RunnerStatus::new(r, Some(format!("/bin/{}", r.bin))))
            .collect()
    }

    fn only(id: &str) -> Vec<RunnerStatus> {
        RUNNERS
            .iter()
            .map(|r| RunnerStatus::new(r, (r.id == id).then(|| format!("/bin/{}", r.bin))))
            .collect()
    }

    fn write(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn request<'a>(kind: LaunchKind, repo: Option<RepoTarget<'a>>) -> LaunchRequest<'a> {
        LaunchRequest {
            kind,
            runner: None,
            repo,
            fleet: &[],
            fallback_cwd: Path::new("/"),
        }
    }

    fn target(root: &Path) -> RepoTarget<'_> {
        RepoTarget {
            name: "app",
            root,
            branch: "main",
        }
    }

    #[test]
    fn finds_executables_on_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir
            .path()
            .join(if cfg!(windows) { "tool.exe" } else { "tool" });
        fs::write(&bin, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Not executable yet ⇒ not found.
            assert!(find_on_path("tool", dir.path().as_os_str()).is_none());
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(find_on_path("tool", dir.path().as_os_str()), Some(bin));
        assert!(find_on_path("absent", dir.path().as_os_str()).is_none());
    }

    #[test]
    fn detection_lists_every_runner_with_a_reason_when_missing() {
        let statuses = detect_runners();
        assert_eq!(statuses.len(), RUNNERS.len());
        for s in &statuses {
            assert_eq!(s.available, s.reason.is_none());
            assert_eq!(s.available, s.path.is_some());
        }
    }

    #[test]
    fn a_shipped_prompt_becomes_a_command_and_is_logged() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let plan = plan_launch(
            home.path(),
            &request(
                LaunchKind::Prompt("setup-repo".to_string()),
                Some(target(repo.path())),
            ),
            &all_installed(),
            NOW,
        )
        .unwrap();

        assert_eq!(plan.runner, "claude-code");
        assert_eq!(plan.label, "Claude Code: Set up this repo");
        assert_eq!(plan.cwd, repo.path());
        assert_eq!(plan.program, "/bin/claude");
        let file = plan.prompt_file.clone().unwrap();
        assert!(file.starts_with(home.path().join("prompts")));
        let body = fs::read_to_string(&file).unwrap();
        assert!(body.contains("cohors launchpad"));
        assert!(body.contains(&repo.path().display().to_string()));
        // Terminal form reads the file; direct form carries the text.
        assert!(plan.command.starts_with("claude "));
        assert!(plan.command.contains(&file.display().to_string()));
        assert_eq!(plan.args, [body]);
        assert!(!plan.linked && plan.session.is_none());

        let log = super::super::recent_activity(home.path(), 5);
        assert_eq!(log.len(), 1);
        assert_eq!(
            (log[0].phase.as_str(), log[0].agent.as_str()),
            ("launch", "setup-repo")
        );
        assert_eq!(log[0].source, "cohors");
    }

    #[test]
    fn a_fleet_prompt_needs_no_repo_and_lists_the_fleet() {
        let home = tempfile::tempdir().unwrap();
        let fleet = [("app".to_string(), "/code/app".to_string())];
        let mut req = request(LaunchKind::Prompt("plan-day".to_string()), None);
        req.fleet = &fleet;
        req.fallback_cwd = home.path();
        let plan = plan_launch(home.path(), &req, &all_installed(), NOW).unwrap();
        assert_eq!(plan.cwd, home.path());
        let body = fs::read_to_string(plan.prompt_file.unwrap()).unwrap();
        assert!(body.contains("app  (/code/app)"));
        assert!(body.contains("board.json"));

        // A repo prompt without a repo is refused, plainly.
        let err = plan_launch(
            home.path(),
            &request(LaunchKind::Prompt("setup-repo".to_string()), None),
            &all_installed(),
            NOW,
        )
        .unwrap_err();
        assert!(err.to_string().contains("--repo"));
        let err = plan_launch(
            home.path(),
            &request(LaunchKind::Prompt("nope".to_string()), None),
            &all_installed(),
            NOW,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown prompt"));
    }

    #[test]
    fn runner_choice_respects_what_is_installed() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let mut req = request(LaunchKind::Start, Some(target(repo.path())));

        // Nothing installed: say what was looked for.
        let none: Vec<RunnerStatus> = RUNNERS.iter().map(|r| RunnerStatus::new(r, None)).collect();
        let err = plan_launch(home.path(), &req, &none, NOW).unwrap_err();
        assert!(
            err.to_string()
                .contains("no supported agent runner is installed")
        );

        // The first installed one is used when none is named.
        let plan = plan_launch(home.path(), &req, &only("codex"), NOW).unwrap();
        assert_eq!(
            (plan.runner.as_str(), plan.command.as_str()),
            ("codex", "codex")
        );
        assert!(plan.args.is_empty() && plan.prompt_file.is_none());

        // A named runner that is missing is an error naming the reason.
        req.runner = Some("agy");
        let err = plan_launch(home.path(), &req, &only("codex"), NOW).unwrap_err();
        assert!(err.to_string().contains("agy is unavailable"));
        req.runner = Some("emacs");
        assert!(plan_launch(home.path(), &req, &only("codex"), NOW).is_err());
    }

    const CONTRIBUTION: &str = r#"{
        "specVersion": 1, "name": "app", "flows": [],
        "agents": [{"id": "triage", "label": "Triage", "file": ".principia/agents/triage.md",
                    "supports": ["codex"]}],
        "tasks": [{"id": "ship", "title": "Ship it", "notes": "Behind the flag."},
                  {"id": "sort", "title": "Sort it", "agent": "triage"}]
    }"#;

    #[test]
    fn a_declared_agent_is_confined_and_honours_supports() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        write(repo.path(), ".principia/repo.json", CONTRIBUTION);
        write(
            repo.path(),
            ".principia/agents/triage.md",
            "Triage {{REPO_NAME}} on {{BRANCH}}.",
        );
        let agent = || {
            request(
                LaunchKind::Agent("triage".to_string()),
                Some(target(repo.path())),
            )
        };

        // Only codex is supported, so it is picked even though claude is first.
        let plan = plan_launch(home.path(), &agent(), &all_installed(), NOW).unwrap();
        assert_eq!(plan.runner, "codex");
        assert_eq!(plan.label, "Codex: Triage");
        assert_eq!(plan.args, ["Triage app on main."]);

        // Asking for an unsupported runner is refused.
        let mut req = agent();
        req.runner = Some("claude-code");
        let err = plan_launch(home.path(), &req, &all_installed(), NOW).unwrap_err();
        assert!(err.to_string().contains("does not support"));

        // An agent file that escapes .principia/ is refused.
        write(repo.path(), "secret.md", "nope");
        write(
            repo.path(),
            ".principia/repo.json",
            &CONTRIBUTION.replace(".principia/agents/triage.md", ".principia/../secret.md"),
        );
        let err = plan_launch(home.path(), &agent(), &all_installed(), NOW).unwrap_err();
        assert!(err.to_string().contains("refusing agent file"), "{err}");

        // Ids are kebab-case or nothing.
        let bad = request(
            LaunchKind::Agent("../x".to_string()),
            Some(target(repo.path())),
        );
        assert!(plan_launch(home.path(), &bad, &all_installed(), NOW).is_err());
    }

    #[test]
    fn a_task_starts_linked_then_resumes_its_conversation() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        write(repo.path(), ".principia/repo.json", CONTRIBUTION);
        let task = || {
            request(
                LaunchKind::Task("ship".to_string()),
                Some(target(repo.path())),
            )
        };

        // First launch: a new conversation under an id we chose, and the link
        // is written to the gitignored local.json.
        let first = plan_launch(home.path(), &task(), &all_installed(), NOW).unwrap();
        let session = first.session.clone().expect("a chosen session id");
        assert!(first.linked && is_session_id(&session));
        assert!(
            first
                .command
                .starts_with(&format!("claude --session-id '{session}' "))
        );
        assert_eq!(read_threads(repo.path()).get("ship"), Some(&session));
        let ignore = fs::read_to_string(repo.path().join(".gitignore")).unwrap();
        assert!(ignore.contains(".principia/local.json"));
        let body = fs::read_to_string(first.prompt_file.unwrap()).unwrap();
        assert!(body.contains("Task: Ship it") && body.contains("Behind the flag."));

        // Second launch: go back to that conversation.
        let again = plan_launch(home.path(), &task(), &all_installed(), NOW).unwrap();
        assert_eq!(again.command, format!("claude --resume '{session}'"));
        assert_eq!(again.args, ["--resume".to_string(), session.clone()]);
        assert!(again.prompt_file.is_none());

        // A runner that cannot resume starts fresh rather than failing.
        let fresh = plan_launch(home.path(), &task(), &only("gemini-cli"), NOW).unwrap();
        assert!(fresh.command.starts_with("gemini --session-id "));

        // A task that names a shipped agent runs that agent with the task attached.
        write(repo.path(), ".principia/agents/triage.md", "Triage.");
        let sorted = plan_launch(
            home.path(),
            &request(
                LaunchKind::Task("sort".to_string()),
                Some(target(repo.path())),
            ),
            &all_installed(),
            NOW,
        )
        .unwrap();
        assert_eq!(sorted.runner, "codex");
        assert!(sorted.args[0].ends_with("Specific task: Sort it"));

        let missing = request(
            LaunchKind::Task("absent".to_string()),
            Some(target(repo.path())),
        );
        assert!(plan_launch(home.path(), &missing, &all_installed(), NOW).is_err());
    }

    #[test]
    fn resume_needs_a_real_id_and_a_capable_runner() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let resume = |id: &str| {
            request(
                LaunchKind::Resume(id.to_string()),
                Some(target(repo.path())),
            )
        };
        let plan =
            plan_launch(home.path(), &resume("abcdef12-3456"), &all_installed(), NOW).unwrap();
        assert_eq!(plan.command, "claude --resume 'abcdef12-3456'");
        assert!(plan_launch(home.path(), &resume("x; rm -rf ~"), &all_installed(), NOW).is_err());
        let err = plan_launch(
            home.path(),
            &resume("abcdef12-3456"),
            &only("gemini-cli"),
            NOW,
        )
        .unwrap_err();
        assert!(err.to_string().contains("cannot reopen a session"));
    }

    #[test]
    fn threads_keep_other_local_keys_and_old_prompts_are_pruned() {
        let repo = tempfile::tempdir().unwrap();
        write(
            repo.path(),
            ".principia/local.json",
            r#"{"device":"pixel","threads":{"a":"1"}}"#,
        );
        write_thread(repo.path(), "b", "abcdef12", NOW).unwrap();
        let doc: Value =
            serde_json::from_str(&fs::read_to_string(repo.path().join(LOCAL_FILE)).unwrap())
                .unwrap();
        assert_eq!(doc["device"], "pixel");
        assert_eq!(doc["threads"], json!({"a": "1", "b": "abcdef12"}));

        let home = tempfile::tempdir().unwrap();
        let file = write_prompt_file(home.path(), "../../etc/passwd", "x", NOW).unwrap();
        assert_eq!(file.parent().unwrap(), home.path().join("prompts"));
        assert!(
            file.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("etcpasswd-")
        );
        // A file "written" far in the future's past is cleared on the next write.
        let later = NOW + 10 * PROMPT_TTL_SECS + 4_000_000_000;
        write_prompt_file(home.path(), "next", "y", later).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn session_ids_look_like_uuids_and_differ() {
        let (a, b) = (new_session_id(), new_session_id());
        assert_ne!(a, b);
        assert!(is_session_id(&a));
        assert_eq!(a.len(), 36);
        assert_eq!(a.as_bytes()[14], b'4');
    }
}
