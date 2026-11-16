//! Suggestions for every surface: the agent sessions found on this machine,
//! the overview's "do next" steps and the focus board, ranked into one list
//! with a command each (ADR-062). The CLI (`cohors next`, `cohors agent
//! --resume`) and the MCP `next` tool read it here, so a person and an agent
//! are told the same thing.

use std::path::{Path, PathBuf};

use cohors_core::suggest::{AgentSession, Suggestion};
use cohors_core::{Overview, RepoSnapshot};

/// The coding-agent conversations held in the fleet's repositories, newest
/// first. `home` is the user's home folder (where the agents keep them).
pub fn agent_sessions(snapshots: &[RepoSnapshot], home: &Path, now: i64) -> Vec<AgentSession> {
    let repos: Vec<(String, PathBuf)> = snapshots
        .iter()
        .filter_map(|s| {
            s.path
                .as_ref()
                .map(|p| (s.id.0.clone(), p.as_std_path().to_path_buf()))
        })
        .collect();
    cohors_launchpad::sessions::agent_sessions(&repos, home, now)
}

/// The repository `dir` is in (the deepest one that contains it), by id.
pub fn repo_containing(snapshots: &[RepoSnapshot], dir: &Path) -> Option<String> {
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    snapshots
        .iter()
        .filter_map(|s| {
            let root = s.path.as_ref()?.as_std_path();
            let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
            dir.starts_with(&root).then(|| (s.id.0.clone(), root))
        })
        .max_by_key(|(_, root)| root.as_os_str().len())
        .map(|(id, _)| id)
}

/// Everything a suggestion list is built from, and the list itself.
#[derive(Debug, Clone)]
pub struct Suggestions {
    pub overview: Overview,
    pub sessions: Vec<AgentSession>,
    pub list: Vec<Suggestion>,
}

/// The ranked suggestions for `snapshots`, at most `limit`, judged from the
/// repo `here` (if any). `user_home` is where the agents keep their sessions;
/// `launchpad_home` is cohors' own (`~/.cohors`) for the focus board.
pub fn suggestions(
    snapshots: &[RepoSnapshot],
    here: Option<&str>,
    user_home: Option<&Path>,
    launchpad_home: Option<&Path>,
    now: i64,
    limit: usize,
) -> Suggestions {
    let overview = crate::overview(snapshots, launchpad_home, now);
    let sessions = user_home
        .map(|home| agent_sessions(snapshots, home, now))
        .unwrap_or_default();
    let list = cohors_core::suggest::suggest(&overview, &sessions, here, now, limit);
    Suggestions {
        overview,
        sessions,
        list,
    }
}
