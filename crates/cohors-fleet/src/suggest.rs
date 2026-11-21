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
    // Every folder a session of a repository can have run in: its checkout,
    // and each linked worktree it lists. A worktree kept outside the
    // repository's folder is not under it, so without its own entry a
    // session that ran there matched no repository and was dropped.
    let mut repos: Vec<(String, PathBuf)> = Vec::new();
    for snapshot in snapshots {
        if let Some(path) = &snapshot.path {
            repos.push((snapshot.id.0.clone(), path.as_std_path().to_path_buf()));
        }
        for worktree in snapshot.worktrees.iter().filter(|w| !w.missing) {
            repos.push((
                snapshot.id.0.clone(),
                worktree.path.as_std_path().to_path_buf(),
            ));
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use cohors_core::{LinkedWorktree, RepoId};

    /// A Claude Code session file for a conversation held in `cwd`.
    fn session_in(home: &Path, cwd: &Path, id: &str) {
        let folder = home
            .join(".claude/projects")
            .join(cohors_launchpad::sessions::claude_folder(cwd));
        std::fs::create_dir_all(&folder).unwrap();
        let line = serde_json::json!({
            "type": "user", "cwd": cwd, "message": {"content": "carry on"}
        });
        std::fs::write(folder.join(format!("{id}.jsonl")), format!("{line}\n")).unwrap();
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    /// A worktree kept outside its repository's folder is not under it, so a
    /// session that ran there matched no repository and was dropped. It
    /// belongs to that worktree's repository.
    #[test]
    fn a_session_in_a_worktree_outside_its_repo_belongs_to_that_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let (home, shop, hotfix) = (root.join("home"), root.join("shop"), root.join("hotfix"));
        for dir in [&home, &shop, &hotfix] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let utf8 = |p: &Path| camino::Utf8PathBuf::from_path_buf(p.to_path_buf()).unwrap();
        let mut repo = RepoSnapshot::errored(RepoId("shop".into()), "shop", Some(utf8(&shop)), "");
        repo.error = None;
        repo.worktrees = vec![LinkedWorktree {
            name: "hotfix".into(),
            path: utf8(&hotfix),
            branch: cohors_core::Branch::Named("hotfix".into()),
            status: Default::default(),
            unpushed: 0,
            unpushed_unique: 0,
            last_commit: None,
            touched: None,
            locked: false,
            missing: false,
        }];
        session_in(&home, &hotfix, "aaaa1111-2222-4333-8444-555566667777");
        session_in(&home, &shop, "bbbb1111-2222-4333-8444-555566667777");

        let found = agent_sessions(std::slice::from_ref(&repo), &home, now());
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|s| s.repo == "shop"), "{found:?}");

        // When that worktree is a row of its own (one repository's
        // checkouts), the session is the row's: the deepest folder wins.
        let mut row =
            RepoSnapshot::errored(RepoId("hotfix".into()), "hotfix", Some(utf8(&hotfix)), "");
        row.error = None;
        repo.worktrees.clear();
        let found = agent_sessions(&[repo, row], &home, now());
        let of = |id: &str| {
            found
                .iter()
                .find(|s| s.id.starts_with(id))
                .map(|s| s.repo.as_str())
        };
        assert_eq!((of("aaaa"), of("bbbb")), (Some("hotfix"), Some("shop")));
    }
}
