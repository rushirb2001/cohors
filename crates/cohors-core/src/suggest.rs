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
    /// The same command as arguments to `cohors` (without the program name),
    /// for running it without a shell.
    pub args: Vec<String>,
    /// The repository it concerns, when it concerns one.
    pub repo: Option<String>,
    /// Higher comes first. Comparable across kinds.
    pub score: u32,
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
    }
}

/// `cohors` and `args` as one line to read or paste, each argument quoted for
/// a POSIX shell where it needs it.
pub fn command_line(args: &[String]) -> String {
    std::iter::once("cohors".to_string())
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
        let dirty = o
            .repos
            .iter()
            .find(|r| r.staged + r.modified + r.untracked > 0)
            .expect("the demo fleet has a dirty repo");
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
}
