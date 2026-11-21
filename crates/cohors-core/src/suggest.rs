//! "What should I do now?" — suggestions with the exact command to type.
//!
//! For someone who is unsure where they left off, or who would rather not
//! type: the conversations they had with coding agents in each repository
//! (which one to resume), the fleet's "do next" steps, and the focus board,
//! ranked into one short list. Each [`Suggestion`] carries the command that
//! carries it out, so a person can copy it and an agent can run it.
//!
//! Pure: the sessions are found on disk by the launchpad adapter and passed
//! in; this module only judges and orders them.
//!
//! Below the real work sit the **setup tips** (ADR-065): the parts of cohors a
//! person has not used yet (the web dashboard, the editor extension, Claude
//! Code's MCP server), offered one or two at a time and never above anything
//! that needs doing. What is installed is found by `cohors_fleet::tips`; this
//! module only words the tips and decides which are still worth showing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::attention::Severity;
use crate::overview::{ActionKind, Overview, RepoLine};

/// A conversation with a coding agent, found on this machine and tied to the
/// repository it was held in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSession {
    /// The runner that owns it: `claude-code` or `codex` (the runners that
    /// can reopen a conversation).
    pub runner: String,
    /// The runner's own session id (what `--resume` takes).
    pub id: String,
    /// The repository's id (its path), the deepest repo the session ran in.
    pub repo: String,
    /// The folder the session ran in (the repo or a folder inside it).
    pub cwd: String,
    /// The branch it was on, when the runner recorded one.
    pub branch: Option<String>,
    /// What it was about: the title the user gave it, else the last prompt,
    /// else the first. Repository-authored text: render it through a
    /// control-character filter.
    pub title: String,
    /// When it was last active (Unix seconds).
    pub last_active: i64,
}

/// What a suggestion asks you to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    /// Reopen an agent conversation.
    Resume,
    /// A "do next" step from the overview.
    Act,
    /// An item on the focus board.
    Focus,
    /// Nothing pressing: start something.
    Start,
    /// A part of cohors you have not used yet (a setup tip).
    Setup,
}

/// The program every suggestion except some setup tips runs.
pub const COHORS: &str = "cohors";

fn cohors_program() -> String {
    COHORS.to_string()
}

/// One suggestion: what, why, and the command that does it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    pub kind: SuggestionKind,
    /// What to do, in a few words.
    pub title: String,
    /// Why it is suggested (where, how recent, what is left).
    pub why: String,
    /// The command that carries it out, for reading and copying (each
    /// argument quoted for a POSIX shell where it needs it).
    pub command: String,
    /// The same command as arguments to `program` (without the program
    /// name), for running it without a shell.
    pub args: Vec<String>,
    /// The repository it concerns, when it concerns one.
    pub repo: Option<String>,
    /// Higher comes first. Comparable across kinds.
    pub score: u32,
    /// The program `args` are for: `cohors`, except for a setup tip that is
    /// another tool's command (`code`, `claude`). `cohors next N` runs only
    /// cohors itself and prints any other command for you to run.
    #[serde(default = "cohors_program")]
    pub program: String,
    /// A setup tip's id (`web`, `extension`, `claude`), which `cohors next
    /// --dismiss <id>` takes. `None` for everything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip: Option<String>,
    /// A setup tip's name in a word or two (`Web dashboard`, `VS Code`), for
    /// a line too narrow for the title. `None` for everything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl Suggestion {
    /// Whether carrying it out means running cohors itself.
    pub fn runs_cohors(&self) -> bool {
        self.program == COHORS
    }

    /// Whether it has a command worth copying (a first-run tip about the
    /// dashboard's keys has none: its "command" is the keys).
    pub fn copyable(&self) -> bool {
        self.program != KEYS
    }

    /// What a tip asks you to do, in words: its command, or for a tip about
    /// the dashboard's keys `press /` (nothing when the tip's own sentence
    /// already says it, as the one about clicking does).
    pub fn action(&self) -> String {
        if self.copyable() {
            self.command.clone()
        } else if self.command.is_empty() {
            String::new()
        } else {
            format!("press {}", self.command)
        }
    }

    /// Whether it is one of the first-run tips ([`LEARN_TIPS`]).
    pub fn is_first_run_tip(&self) -> bool {
        self.tip
            .as_deref()
            .is_some_and(|id| id.starts_with(LEARN_PREFIX))
    }
}

/// Sessions older than this are not suggested: a month-old conversation is
/// history, not something to pick up.
pub const SESSION_HORIZON_SECS: i64 = 30 * 24 * 3600;

/// How many sessions a suggestion list offers at most.
const SESSIONS_SUGGESTED: usize = 3;

/// The arguments to `cohors` that start a "do next" step (the shortest
/// command that does).
pub fn action_args(kind: ActionKind) -> &'static [&'static str] {
    use ActionKind as K;
    match kind {
        K::Unreadable => &["scan", "--select", "error"],
        K::Resolve => &["scan", "--select", "conflicted"],
        // Diverged branches are left out: a plain push would be rejected.
        K::Push => &["push", "--select", "ahead,!diverged"],
        K::Reconcile => &["scan", "--select", "diverged"],
        K::Publish => &["push", "--select", "unpublished"],
        K::ReviewStash => &["scan", "--select", "stash"],
        // Diverged branches are left out: a fast-forward pull cannot apply.
        K::Pull => &["pull", "--select", "behind,!diverged"],
        K::Commit => &["scan", "--select", "dirty"],
        K::Fetch => &["fetch", "--select", "stale-fetch"],
        K::StartTask => &["launchpad"],
        K::Worktrees => &["scan", "--select", "worktree-work"],
    }
}

/// `cohors` and `args` as one line to read or paste, each argument quoted for
/// a POSIX shell where it needs it.
pub fn command_line(args: &[String]) -> String {
    program_command_line(COHORS, args)
}

/// `program` and `args` as one line to read or paste, every word quoted for a
/// POSIX shell where it needs it (a program path with spaces included).
pub fn program_command_line(program: &str, args: &[String]) -> String {
    std::iter::once(shell_word(program))
        .chain(args.iter().map(|a| shell_word(a)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The command line that starts a "do next" step.
pub fn action_command(kind: ActionKind) -> String {
    let args: Vec<String> = action_args(kind).iter().map(|a| a.to_string()).collect();
    command_line(&args)
}

/// How likely `session` is the one to pick up, or `None` when it is too old.
/// Recency leads; a session in the repo you are in, in a repo that still has
/// uncommitted work made after its last commit, or on the branch that is
/// checked out now, rises.
pub fn session_score(
    session: &AgentSession,
    line: Option<&RepoLine>,
    here: Option<&str>,
    now: i64,
) -> Option<u32> {
    let age = (now - session.last_active).max(0);
    if age > SESSION_HORIZON_SECS {
        return None;
    }
    const HOUR: i64 = 3600;
    let mut score = match age {
        a if a < HOUR => 100,
        a if a < 6 * HOUR => 85,
        a if a < 24 * HOUR => 70,
        a if a < 3 * 24 * HOUR => 50,
        a if a < 7 * 24 * HOUR => 35,
        _ => 15,
    };
    // The repo you are in leads: within a day it outranks any other repo's
    // session, however fresh.
    if here == Some(session.repo.as_str()) {
        score += 60;
    }
    if let Some(line) = line {
        let dirty = line.staged + line.modified + line.untracked > 0;
        let after_last_commit = line
            .commit
            .as_ref()
            .is_none_or(|c| c.timestamp <= session.last_active);
        if dirty && after_last_commit {
            score += 30; // work was left behind after this conversation
        }
        if session.branch.as_deref() == Some(line.branch.as_str()) {
            score += 10;
        }
    }
    Some(score)
}

/// The sessions worth resuming, most likely first, each with its score.
/// With `here` (a repo id), only that repository's sessions.
pub fn rank_sessions<'a>(
    sessions: &'a [AgentSession],
    overview: &Overview,
    here: Option<&str>,
    only_here: bool,
    now: i64,
) -> Vec<(&'a AgentSession, u32)> {
    let mut ranked: Vec<(&AgentSession, u32)> = sessions
        .iter()
        .filter(|s| !only_here || here == Some(s.repo.as_str()))
        .filter_map(|s| {
            let line = overview.repos.iter().find(|r| r.id == s.repo);
            session_score(s, line, here, now).map(|score| (s, score))
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(b.0.last_active.cmp(&a.0.last_active))
            .then(a.0.id.cmp(&b.0.id))
    });
    ranked
}

/// The arguments that reopen `session` (cohors finds the runner by the id).
pub fn resume_args(session: &AgentSession, repo_name: &str) -> Vec<String> {
    vec![
        "agent".into(),
        "--resume".into(),
        session.id.clone(),
        "-r".into(),
        repo_name.to_string(),
    ]
}

/// The command line that reopens `session`.
pub fn resume_command(session: &AgentSession, repo_name: &str) -> String {
    command_line(&resume_args(session, repo_name))
}

/// Quote `word` for a POSIX shell when it needs it.
fn shell_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./@:+".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// The short list: sessions to resume, "do next" steps and focus items,
/// ranked together, at most `limit`. `here` is the repo you are in, if any.
pub fn suggest(
    overview: &Overview,
    sessions: &[AgentSession],
    here: Option<&str>,
    now: i64,
    limit: usize,
) -> Vec<Suggestion> {
    let name_of = |id: &str| {
        overview
            .repos
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| id.rsplit('/').next().unwrap_or(id).to_string())
    };
    let mut out: Vec<Suggestion> = Vec::new();

    for (session, score) in rank_sessions(sessions, overview, here, false, now)
        .into_iter()
        .take(SESSIONS_SUGGESTED)
    {
        let name = name_of(&session.repo);
        let mut why = vec![
            name.clone(),
            session.runner.clone(),
            crate::time::relative(session.last_active, now),
        ];
        if let Some(line) = overview.repos.iter().find(|r| r.id == session.repo) {
            let dirty = line.staged + line.modified + line.untracked;
            if dirty > 0 {
                why.push(format!(
                    "{dirty} uncommitted file{}",
                    if dirty == 1 { "" } else { "s" }
                ));
            }
        }
        out.push(Suggestion {
            kind: SuggestionKind::Resume,
            title: format!("Resume \u{201c}{}\u{201d}", session.title),
            why: why.join(" \u{b7} "),
            command: resume_command(session, &name),
            args: resume_args(session, &name),
            repo: Some(session.repo.clone()),
            score,
            program: cohors_program(),
            tip: None,
            label: None,
        });
    }

    for action in &overview.actions {
        let weight = match action.severity {
            Severity::Risk => 95,
            Severity::Warn => 75,
            Severity::Notice => 45,
            Severity::Info => 25,
            Severity::Ok => 10,
        };
        // A step in the repo you are in matters a little more to you now.
        let bonus = if here.is_some_and(|h| action.repos.iter().any(|r| r == h)) {
            10
        } else {
            0
        };
        out.push(Suggestion {
            kind: SuggestionKind::Act,
            title: action.title.clone(),
            why: action.why.clone(),
            command: action_command(action.kind),
            args: action_args(action.kind)
                .iter()
                .map(|a| a.to_string())
                .collect(),
            repo: (action.repos.len() == 1).then(|| action.repos[0].clone()),
            score: weight + bonus,
            program: cohors_program(),
            tip: None,
            label: None,
        });
    }

    for item in &overview.focus {
        let Some(repo) = &item.repo else { continue };
        // A focus item that names a task can start an agent on it directly.
        let task = item
            .reference
            .as_deref()
            .filter(|r| is_kebab_id(r))
            .unwrap_or(item.id.as_str());
        let args: Vec<String> = if is_kebab_id(task) {
            vec!["agent".into(), task.to_string(), "-r".into(), repo.clone()]
        } else {
            vec!["launchpad".into()]
        };
        let command = command_line(&args);
        out.push(Suggestion {
            kind: SuggestionKind::Focus,
            title: format!("Work on \u{201c}{}\u{201d}", item.title),
            why: format!("on your focus board \u{b7} {repo}"),
            command,
            args,
            repo: Some(repo.clone()),
            score: 55,
            program: cohors_program(),
            tip: None,
            label: None,
        });
    }

    if out.is_empty() {
        out.push(Suggestion {
            kind: SuggestionKind::Start,
            title: "Start an agent session".to_string(),
            why: "nothing is waiting on you".to_string(),
            command: "cohors agent".to_string(),
            args: vec!["agent".into()],
            repo: here.map(str::to_string),
            score: 1,
            program: cohors_program(),
            tip: None,
            label: None,
        });
    }

    // Stable: by score, then the order they were added (sessions, steps, focus).
    out.sort_by_key(|s| std::cmp::Reverse(s.score));
    out.truncate(limit.max(1));
    out
}

/// A task or prompt id: lowercase letters, digits and single dashes.
fn is_kebab_id(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

// ── Setup tips ───────────────────────────────────────────────────────────────

/// The cohors editor extension's id on the VS Code Marketplace and Open VSX.
pub const EXTENSION_ID: &str = "rushirb2001.cohors";

/// Tip: open the web dashboard (`cohors web`).
pub const TIP_WEB: &str = "web";
/// Tip: install the editor extension.
pub const TIP_EXTENSION: &str = "extension";
/// Tip: register cohors as one of Claude Code's MCP servers.
pub const TIP_CLAUDE: &str = "claude";
/// Every setup tip, in the order they are offered.
pub const TIP_IDS: [&str; 3] = [TIP_WEB, TIP_EXTENSION, TIP_CLAUDE];

/// A tip shown on this many separate days with nothing done about it is not
/// shown again. Five: a working week of seeing it is enough to decide, and
/// after that it would only be in the way. Days, not showings, because the
/// dashboard, the start screen, `cohors next` and the web page all show it,
/// and one busy afternoon should not use it up.
pub const TIP_SHOWN_DAYS: u32 = 5;

/// The most tips any surface shows at once: one or two quiet lines, never a
/// list of chores.
pub const TIPS_AT_ONCE: usize = 2;

/// What the setup tips need to know about this machine. Found by an adapter
/// (`cohors_fleet::tips`); nothing here touches the disk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SetupFacts {
    /// `cohors web` has served its page here at least once.
    pub web_opened: bool,
    /// The editors that take the cohors extension and are installed, the
    /// one to suggest first.
    pub editors: Vec<EditorFacts>,
    /// Claude Code, when it is installed.
    pub claude: Option<ClaudeFacts>,
    /// The cohors to register with Claude Code: an absolute path, the one on
    /// PATH when it is this same program (it survives an upgrade).
    pub cohors_path: Option<String>,
}

/// An installed editor that can take the cohors extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorFacts {
    /// As people say it: `VS Code`, `Cursor`, `Windsurf`.
    pub name: String,
    /// Its command line program (`code`), or the full path to the one inside
    /// the app when it is not on PATH.
    pub cli: String,
    /// The cohors extension is installed in it.
    pub has_extension: bool,
}

/// Claude Code, as installed here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeFacts {
    /// Its command line program (`claude`, or the full path when it is not
    /// on PATH).
    pub cli: String,
    /// Whether cohors is already one of its MCP servers, or its plugin is
    /// installed. `None` when that could not be told reliably, and then no
    /// tip is offered: a tip that might be wrong is worse than none.
    pub has_cohors: Option<bool>,
}

/// What a tip is about, by id, for a message about a tip that is not on
/// screen (`cohors next --dismiss`). `None` for an unknown id.
pub fn tip_label(id: &str) -> Option<&'static str> {
    Some(match id {
        TIP_WEB => "open your fleet in the browser",
        TIP_EXTENSION => "install the cohors extension for your editor",
        TIP_CLAUDE => "let Claude Code see your fleet",
        _ => return LEARN_TIPS.iter().find(|t| t.0 == id).map(|t| t.1),
    })
}

// ── First-run tips ───────────────────────────────────────────────────────────

/// The `program` of a tip that names keys to press in the dashboard rather
/// than a command to run: it is read, never copied or run.
pub const KEYS: &str = "keys";

/// Every first-run tip's id starts with this.
pub const LEARN_PREFIX: &str = "learn-";

/// A first-run tip is shown on one day, then the next one takes its place:
/// about a month of one new thing a day, and `x` in the dashboard moves on
/// at once for anyone who wants them faster.
pub const LEARN_SHOWN_DAYS: u32 = 1;

/// The first-run tips, most useful first: `(id, what it does, the keys or
/// the command, program)`. The dashboard's keys come first (that is where
/// a new user is), then the commands worth knowing at a terminal. Every key
/// and flag named here is real; a test in `cohors` checks the keys against
/// the dashboard's keymap.
pub const LEARN_TIPS: &[(&str, &str, &str, &str)] = &[
    (
        "learn-cards",
        "Jump to a card with its number, or click it",
        "1-5",
        KEYS,
    ),
    (
        "learn-mouse",
        "Click a row to select it, click it again to open it",
        "",
        KEYS,
    ),
    (
        "learn-filter",
        "Filter the repositories by name as you type",
        "/",
        KEYS,
    ),
    (
        "learn-detail",
        "Open the selected repository's full detail",
        "enter",
        KEYS,
    ),
    (
        "learn-standup",
        "See what you committed, by day, across every repository",
        "s",
        KEYS,
    ),
    (
        "learn-standup-copy",
        "Copy your standup as markdown, ready to paste",
        "Y",
        KEYS,
    ),
    (
        "learn-board",
        "Open the task board of every repository",
        "t",
        KEYS,
    ),
    (
        "learn-mark",
        "Mark several repositories, then act on all of them at once",
        "space",
        KEYS,
    ),
    (
        "learn-fetch-all",
        "Fetch every repository at once",
        "F",
        KEYS,
    ),
    (
        "learn-pull",
        "Pull the selected repositories, fast-forward only",
        "p",
        KEYS,
    ),
    (
        "learn-push",
        "Push the selected repositories, publishing new branches",
        "P",
        KEYS,
    ),
    (
        "learn-commit",
        "Commit: cohors asks for the message and confirms first",
        "c",
        KEYS,
    ),
    (
        "learn-stash",
        "Stash the changes in the selected repositories, after asking",
        "S",
        KEYS,
    ),
    (
        "learn-open",
        "Open the selected repository in your editor or another app",
        "o",
        KEYS,
    ),
    (
        "learn-run",
        "Run a shell command in every selected repository",
        "!",
        KEYS,
    ),
    (
        "learn-agent",
        "Start a coding agent in the selected repository",
        "a",
        KEYS,
    ),
    (
        "learn-copy-path",
        "Copy the selected repository's path",
        "y",
        KEYS,
    ),
    (
        "learn-changes",
        "See the changed files and their diff, in the detail view",
        "d",
        KEYS,
    ),
    (
        "learn-command",
        "Sort, narrow or jump with a command, such as :sort name",
        ":",
        KEYS,
    ),
    (
        "learn-lazygit",
        "Open lazygit in the selected repository, when it is installed",
        "L",
        KEYS,
    ),
    (
        "learn-rescan",
        "Rescan now; the dashboard also follows changes by itself",
        "r",
        KEYS,
    ),
    ("learn-help", "See every key", "?", KEYS),
    (
        "learn-next",
        "Ask what to do next, and which agent session to resume",
        "cohors next",
        COHORS,
    ),
    (
        "learn-standup-cli",
        "Your commits of the last week, across every repository",
        "cohors standup --since 7d",
        COHORS,
    ),
    (
        "learn-search",
        "Search every repository at once",
        "cohors search TODO",
        COHORS,
    ),
    (
        "learn-path",
        "Go to a repository by name from any shell",
        "cd \"$(cohors path NAME)\"",
        COHORS,
    ),
    (
        "learn-dry-run",
        "See which repositories a pull would touch, without pulling",
        "cohors pull --select behind --dry-run",
        COHORS,
    ),
    (
        "learn-deep",
        "Find repositories nested inside other repositories",
        "cohors discover --deep",
        COHORS,
    ),
    (
        "learn-watch",
        "Follow changes across your repositories as they happen",
        "cohors watch",
        COHORS,
    ),
    (
        "learn-doctor",
        "Check which cohors you run and whether it updates itself",
        "cohors doctor",
        COHORS,
    ),
];

/// The first-run tips as suggestions, in [`LEARN_TIPS`] order, each scored
/// below every setup tip.
pub fn learn_tips() -> Vec<Suggestion> {
    LEARN_TIPS
        .iter()
        .map(|&(id, title, command, program)| Suggestion {
            // A tip, like the setup tips: shown on the tip line, never in a
            // list of work.
            kind: SuggestionKind::Setup,
            title: title.to_string(),
            why: String::new(),
            command: command.to_string(),
            // Never run: a tip is read, or its command copied.
            args: Vec::new(),
            repo: None,
            score: 0,
            program: program.to_string(),
            tip: Some(id.to_string()),
            label: None,
        })
        .collect()
}

/// On how many days tip `id` is shown before it retires.
fn shown_days_for(id: &str) -> u32 {
    if id.starts_with(LEARN_PREFIX) {
        LEARN_SHOWN_DAYS
    } else {
        TIP_SHOWN_DAYS
    }
}

fn setup_tip(
    id: &str,
    label: &str,
    title: String,
    why: String,
    program: &str,
    args: Vec<String>,
) -> Suggestion {
    Suggestion {
        kind: SuggestionKind::Setup,
        title,
        why,
        command: program_command_line(program, &args),
        args,
        repo: None,
        // Below every other suggestion, whatever it is (the lowest of them,
        // "start an agent session", is 1): a tip never outranks real work.
        score: 0,
        program: program.to_string(),
        tip: Some(id.to_string()),
        label: Some(label.to_string()),
    }
}

/// Every setup tip that applies to this machine, in the order they are worth
/// doing: open the web dashboard, install the editor extension, let Claude
/// Code see the fleet. One that is done is not there.
pub fn setup_tips(facts: &SetupFacts) -> Vec<Suggestion> {
    let mut tips = Vec::new();
    if !facts.web_opened {
        tips.push(setup_tip(
            TIP_WEB,
            "Web dashboard",
            "Open your fleet in the browser".to_string(),
            "the same dashboard as a web page, kept live, with pull requests and CI".to_string(),
            COHORS,
            vec!["web".to_string()],
        ));
    }
    // One editor is enough: someone with the extension in any of them knows
    // it exists, and that is all a tip is for.
    if !facts.editors.iter().any(|e| e.has_extension)
        && let Some(editor) = facts.editors.first()
    {
        tips.push(setup_tip(
            TIP_EXTENSION,
            &editor.name,
            format!("Install the cohors extension for {}", editor.name),
            format!(
                "{} is installed: the extension shows your fleet in its sidebar",
                editor.name
            ),
            &editor.cli,
            vec!["--install-extension".to_string(), EXTENSION_ID.to_string()],
        ));
    }
    if let Some(claude) = &facts.claude
        && claude.has_cohors == Some(false)
        && let Some(cohors) = &facts.cohors_path
    {
        // User scope: the fleet is the same from every folder, and the
        // default scope would register it for the current folder only.
        let args = [
            "mcp", "add", "--scope", "user", "cohors", "--", cohors, "mcp",
        ];
        tips.push(setup_tip(
            TIP_CLAUDE,
            "Claude Code",
            "Let Claude Code see your fleet".to_string(),
            "Claude Code is installed, and cohors is not one of its MCP servers yet".to_string(),
            &claude.cli,
            args.iter().map(|a| a.to_string()).collect(),
        ));
    }
    tips
}

/// Which day `now` (Unix seconds) falls on, counted from 1970 in UTC.
fn day_of(now: i64) -> i64 {
    now.div_euclid(24 * 3600)
}

/// What became of each tip: dismissed, or on how many days it was shown.
/// Kept by the adapter in cohors' cache folder (`tips.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TipLedger {
    pub tips: BTreeMap<String, TipRecord>,
}

/// One tip's history.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TipRecord {
    /// Hidden for good (`cohors next --dismiss`, or `x` in the dashboard).
    pub dismissed: bool,
    /// On how many separate days it was shown.
    pub days: u32,
    /// The last of those days (days since 1970, UTC).
    pub last_day: i64,
}

impl TipLedger {
    /// Whether tip `id` is no longer shown: dismissed, or shown on
    /// [`TIP_SHOWN_DAYS`] days already. On the last of those days it is still
    /// shown, so it does not vanish halfway through a day.
    pub fn hides(&self, id: &str, now: i64) -> bool {
        self.tips.get(id).is_some_and(|record| {
            record.dismissed
                || (record.days >= shown_days_for(id) && record.last_day != day_of(now))
        })
    }

    /// Count `id` as shown today. Returns whether anything changed (the
    /// first showing of the day), so the file is written once a day at most.
    pub fn note_shown(&mut self, id: &str, now: i64) -> bool {
        let today = day_of(now);
        let record = self.tips.entry(id.to_string()).or_default();
        if record.days > 0 && record.last_day == today {
            return false;
        }
        record.days = record.days.saturating_add(1);
        record.last_day = today;
        true
    }

    /// Hide `id` for good. Returns whether it was showing until now.
    pub fn dismiss(&mut self, id: &str) -> bool {
        let record = self.tips.entry(id.to_string()).or_default();
        !std::mem::replace(&mut record.dismissed, true)
    }
}

/// The tips from `tips` that `ledger` still lets through, at most `max`, in
/// their order.
pub fn tips_to_show(
    tips: Vec<Suggestion>,
    ledger: &TipLedger,
    now: i64,
    max: usize,
) -> Vec<Suggestion> {
    tips.into_iter()
        .filter(|t| t.tip.as_deref().is_some_and(|id| !ledger.hides(id, now)))
        .take(max)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;
    const HOUR: i64 = 3600;

    fn session(id: &str, repo: &str, age: i64) -> AgentSession {
        AgentSession {
            runner: "claude-code".into(),
            id: id.into(),
            repo: repo.into(),
            cwd: repo.into(),
            branch: Some("main".into()),
            title: format!("work {id}"),
            last_active: NOW - age,
        }
    }

    fn fleet() -> Overview {
        let snaps = crate::demo::fleet(NOW);
        crate::overview::overview(&snaps, &[], &Default::default(), NOW)
    }

    #[test]
    fn recent_sessions_in_this_repo_come_first_and_old_ones_drop() {
        let o = fleet();
        let here = o.repos[0].id.clone();
        let other = o.repos[1].id.clone();
        let sessions = vec![
            session("old", &here, 40 * 24 * HOUR),
            session("elsewhere-fresh", &other, HOUR / 2),
            session("here-yesterday", &here, 20 * HOUR),
        ];
        let ranked = rank_sessions(&sessions, &o, Some(&here), false, NOW);
        let ids: Vec<&str> = ranked.iter().map(|(s, _)| s.id.as_str()).collect();
        assert!(!ids.contains(&"old"), "past the horizon: {ids:?}");
        assert_eq!(
            ids[0], "here-yesterday",
            "the repo you are in wins: {ids:?}"
        );
        let only_here = rank_sessions(&sessions, &o, Some(&here), true, NOW);
        assert_eq!(only_here.len(), 1);
    }

    #[test]
    fn work_left_after_a_session_raises_it() {
        let o = fleet();
        // Dirty, and last committed before the session ended: what is
        // uncommitted there was left after it. (Whichever repo the fleet
        // happens to list first may have been committed to since.)
        let dirty = o
            .repos
            .iter()
            .find(|r| {
                r.staged + r.modified + r.untracked > 0
                    && r.commit
                        .as_ref()
                        .is_some_and(|c| c.timestamp <= NOW - 2 * HOUR)
            })
            .expect("the demo fleet has a dirty repo committed to before the session");
        let clean = o
            .repos
            .iter()
            .find(|r| r.staged + r.modified + r.untracked == 0 && r.id != dirty.id)
            .unwrap();
        let a = session("a", &dirty.id, 2 * HOUR);
        let b = session("b", &clean.id, 2 * HOUR);
        assert!(
            session_score(&a, Some(dirty), None, NOW) > session_score(&b, Some(clean), None, NOW)
        );
    }

    #[test]
    fn suggestions_mix_sessions_and_steps_with_a_command_each() {
        let o = fleet();
        let here = o.repos[0].id.clone();
        let sessions = vec![session("3f2a9c10-aaaa-4bbb", &here, HOUR / 4)];
        let list = suggest(&o, &sessions, Some(&here), NOW, 6);
        assert!(list.len() <= 6);
        assert!(list.iter().all(|s| s.command.starts_with("cohors ")));
        let resume = list
            .iter()
            .find(|s| s.kind == SuggestionKind::Resume)
            .expect("a fresh session here is suggested");
        assert!(
            resume
                .command
                .starts_with("cohors agent --resume 3f2a9c10-aaaa-4bbb -r ")
        );
        assert_eq!(
            &resume.args[..3],
            ["agent", "--resume", "3f2a9c10-aaaa-4bbb"]
        );
        // The display line and the argument list say the same thing.
        assert!(list.iter().all(|s| command_line(&s.args) == s.command));
        assert!(resume.title.contains("work 3f2a9c10-aaaa-4bbb"));
        assert!(list.iter().any(|s| s.kind == SuggestionKind::Act));
        // Sorted by score.
        assert!(list.windows(2).all(|w| w[0].score >= w[1].score));
    }

    #[test]
    fn an_empty_fleet_still_suggests_something() {
        let o = crate::overview::overview(&[], &[], &Default::default(), NOW);
        let list = suggest(&o, &[], None, NOW, 5);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].command, "cohors agent");
    }

    #[test]
    fn repo_names_are_quoted_for_the_shell_when_needed() {
        let s = session("abc", "/r", 0);
        assert_eq!(
            resume_command(&s, "pay-api"),
            "cohors agent --resume abc -r pay-api"
        );
        assert_eq!(
            resume_command(&s, "it's mine"),
            "cohors agent --resume abc -r 'it'\\''s mine'"
        );
        assert_eq!(
            action_command(ActionKind::Push),
            "cohors push --select 'ahead,!diverged'"
        );
    }

    /// A machine with VS Code and Claude Code, where nothing is set up yet.
    fn fresh_machine() -> SetupFacts {
        SetupFacts {
            web_opened: false,
            editors: vec![
                EditorFacts {
                    name: "VS Code".into(),
                    cli: "code".into(),
                    has_extension: false,
                },
                EditorFacts {
                    name: "Cursor".into(),
                    cli: "/Applications/Cursor.app/Contents/Resources/app/bin/cursor".into(),
                    has_extension: false,
                },
            ],
            claude: Some(ClaudeFacts {
                cli: "claude".into(),
                has_cohors: Some(false),
            }),
            cohors_path: Some("/opt/homebrew/bin/cohors".into()),
        }
    }

    #[test]
    fn every_tip_applies_to_a_fresh_machine_in_order_with_its_own_program() {
        let tips = setup_tips(&fresh_machine());
        let ids: Vec<&str> = tips.iter().filter_map(|t| t.tip.as_deref()).collect();
        assert_eq!(ids, TIP_IDS);
        assert!(tips.iter().all(|t| t.kind == SuggestionKind::Setup));
        // The display line and the argument list say the same thing.
        assert!(
            tips.iter()
                .all(|t| program_command_line(&t.program, &t.args) == t.command)
        );
        assert_eq!(tips[0].command, "cohors web");
        assert!(tips[0].runs_cohors());
        assert_eq!(
            tips[1].command,
            "code --install-extension rushirb2001.cohors"
        );
        assert!(!tips[1].runs_cohors(), "the extension tip runs `code`");
        assert!(
            tips[1].title.contains("VS Code"),
            "the first editor is named"
        );
        let labels: Vec<Option<&str>> = tips.iter().map(|t| t.label.as_deref()).collect();
        assert_eq!(
            labels,
            [Some("Web dashboard"), Some("VS Code"), Some("Claude Code")]
        );
        assert_eq!(
            tips[2].command,
            "claude mcp add --scope user cohors -- /opt/homebrew/bin/cohors mcp"
        );
        assert!(!tips[2].runs_cohors());
    }

    #[test]
    fn a_tip_that_is_done_or_unsure_is_not_offered() {
        let mut facts = fresh_machine();
        facts.web_opened = true;
        // The extension in any one editor is enough.
        facts.editors[1].has_extension = true;
        // Claude Code's registration could not be read: no tip, not a guess.
        facts.claude = Some(ClaudeFacts {
            cli: "claude".into(),
            has_cohors: None,
        });
        assert!(setup_tips(&facts).is_empty());

        let mut facts = fresh_machine();
        facts.editors.clear();
        facts.claude = None;
        let ids: Vec<Option<String>> = setup_tips(&facts).into_iter().map(|t| t.tip).collect();
        assert_eq!(ids, [Some(TIP_WEB.to_string())]);
    }

    #[test]
    fn a_path_with_spaces_is_quoted_in_the_command() {
        let mut facts = fresh_machine();
        facts.editors[0].cli =
            "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code".into();
        let tip = setup_tips(&facts).remove(1);
        assert_eq!(
            tip.command,
            "'/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code' \
             --install-extension rushirb2001.cohors"
        );
        assert_eq!(tip.args, ["--install-extension", EXTENSION_ID]);
    }

    #[test]
    fn tips_rank_below_everything_else() {
        let o = crate::overview::overview(&[], &[], &Default::default(), NOW);
        let start = suggest(&o, &[], None, NOW, 5);
        let lowest = start.iter().map(|s| s.score).min().unwrap();
        assert!(
            setup_tips(&fresh_machine())
                .iter()
                .all(|t| t.score < lowest)
        );
    }

    #[test]
    fn a_dismissed_tip_is_gone_and_a_tip_retires_after_its_days() {
        const DAY: i64 = 24 * HOUR;
        let tips = setup_tips(&fresh_machine());
        let mut ledger = TipLedger::default();
        assert_eq!(tips_to_show(tips.clone(), &ledger, NOW, 2).len(), 2);

        assert!(ledger.dismiss(TIP_WEB));
        assert!(!ledger.dismiss(TIP_WEB), "dismissing twice changes nothing");
        let shown = tips_to_show(tips.clone(), &ledger, NOW, 2);
        let ids: Vec<&str> = shown.iter().filter_map(|t| t.tip.as_deref()).collect();
        assert_eq!(ids, [TIP_EXTENSION, TIP_CLAUDE]);

        // Shown many times on one day counts once.
        assert!(ledger.note_shown(TIP_EXTENSION, NOW));
        assert!(!ledger.note_shown(TIP_EXTENSION, NOW + HOUR));
        assert_eq!(ledger.tips[TIP_EXTENSION].days, 1);
        // On its last day it stays up all day, then it is gone.
        for day in 1..i64::from(TIP_SHOWN_DAYS) {
            assert!(!ledger.hides(TIP_EXTENSION, NOW + day * DAY));
            ledger.note_shown(TIP_EXTENSION, NOW + day * DAY);
        }
        let last = NOW + (i64::from(TIP_SHOWN_DAYS) - 1) * DAY;
        assert!(!ledger.hides(TIP_EXTENSION, last + HOUR / 2));
        assert!(ledger.hides(TIP_EXTENSION, last + DAY));
        assert!(
            !ledger.hides(TIP_CLAUDE, last + DAY),
            "each tip counts alone"
        );
    }

    #[test]
    fn there_are_thirty_first_run_tips_each_its_own_and_short() {
        let tips = learn_tips();
        assert_eq!(tips.len(), 30);
        let mut ids: Vec<&str> = tips.iter().filter_map(|t| t.tip.as_deref()).collect();
        assert!(ids.iter().all(|id| id.starts_with(LEARN_PREFIX)));
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), tips.len(), "every id is unique");
        for tip in &tips {
            assert!(tip.is_first_run_tip() && tip.score == 0, "{tip:?}");
            // One line of a dashboard: the sentence and its keys or command.
            assert!(tip.title.chars().count() <= 64, "too long: {}", tip.title);
            assert_eq!(
                tip_label(tip.tip.as_deref().unwrap()),
                Some(tip.title.as_str())
            );
            assert!(tip.args.is_empty(), "a tip is never run");
            assert_eq!(tip.copyable(), tip.program == COHORS, "{tip:?}");
        }
        assert!(!setup_tips(&fresh_machine())[0].is_first_run_tip());
    }

    #[test]
    fn a_first_run_tip_is_shown_on_one_day_and_then_makes_way() {
        const DAY: i64 = 24 * HOUR;
        let mut ledger = TipLedger::default();
        let first = |ledger: &TipLedger, now| {
            tips_to_show(learn_tips(), ledger, now, 1)[0]
                .tip
                .clone()
                .unwrap()
        };
        assert_eq!(first(&ledger, NOW), "learn-cards");
        ledger.note_shown("learn-cards", NOW);
        assert_eq!(first(&ledger, NOW + HOUR), "learn-cards", "all day long");
        assert_eq!(
            first(&ledger, NOW + DAY),
            "learn-mouse",
            "the next day, the next"
        );
        // `x` moves on at once.
        ledger.dismiss("learn-mouse");
        assert_eq!(first(&ledger, NOW + DAY), "learn-filter");
    }

    #[test]
    fn the_ledger_reads_back_what_it_wrote_and_tolerates_less() {
        let mut ledger = TipLedger::default();
        ledger.dismiss(TIP_CLAUDE);
        ledger.note_shown(TIP_WEB, NOW);
        let json = serde_json::to_string(&ledger).unwrap();
        assert_eq!(serde_json::from_str::<TipLedger>(&json).unwrap(), ledger);
        // A file from another version: missing fields default, unknown ones are ignored.
        let older: TipLedger =
            serde_json::from_str(r#"{"tips":{"web":{"days":2}},"later":1}"#).unwrap();
        assert_eq!(older.tips["web"].days, 2);
        assert!(!older.tips["web"].dismissed);
    }

    #[test]
    fn a_suggestion_from_an_older_cohors_runs_cohors() {
        let json = r#"{"kind":"act","title":"t","why":"w","command":"cohors scan","args":["scan"],"repo":null,"score":3}"#;
        let s: Suggestion = serde_json::from_str(json).unwrap();
        assert!(s.runs_cohors() && s.tip.is_none());
        // A suggestion that is not a tip says nothing about tips in JSON.
        let out = serde_json::to_value(&s).unwrap();
        assert!(out.get("tip").is_none() && out["program"] == "cohors");
        assert_eq!(tip_label(TIP_WEB), Some("open your fleet in the browser"));
        assert_eq!(tip_label("nope"), None);
    }
}
