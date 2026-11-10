//! The reads behind the overview, composed once.
//!
//! `cohors_core::overview` is pure: it turns snapshots plus launchpad entries
//! into the model every front-end draws. Something still has to *read* those
//! launchpad entries off disk, and before this module the web server, the
//! terminal UI, the CLI and the MCP server each did it their own way. These
//! three functions are now the only place it happens:
//!
//! - [`launchpad_bundle`] — every repo's launchpad entry, the focus board and
//!   recent agent activity.
//! - [`overview`] — the bundle fed through `cohors_core::overview`.
//! - [`fleet_commits`] — one author's commits across the fleet, for a standup.

use std::path::Path;

use cohors_core::launchpad::{ActivityEvent, Board, LaunchpadEntry};
use cohors_core::{Overview, RepoSnapshot, StandupCommit};
use serde::Serialize;

/// How many recent activity events a bundle carries.
const RECENT_ACTIVITY: usize = 20;

/// What the launchpad knows about the fleet. Serializable as-is, so an HTTP
/// surface or `--json` output can return it directly.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LaunchpadBundle {
    /// One entry per repository that exists on disk, sorted by name.
    pub entries: Vec<LaunchpadEntry>,
    /// The cross-repository focus board (empty when there is none).
    pub board: Board,
    /// The newest agent activity first.
    pub recent_activity: Vec<ActivityEvent>,
}

/// Read the launchpad for `snapshots`.
///
/// `home` is the launchpad home directory (`cohors_fleet::launchpad::
/// launchpad_home()` in production); it is a parameter so tests and the MCP
/// server can point it somewhere else. `None` yields an empty board.
pub fn launchpad_bundle(snapshots: &[RepoSnapshot], home: Option<&Path>) -> LaunchpadBundle {
    let mut entries: Vec<LaunchpadEntry> = snapshots
        .iter()
        .filter_map(|s| {
            let path = s.path.as_ref()?;
            Some(cohors_launchpad::entry(
                &s.id.0,
                &s.name,
                path.as_std_path(),
            ))
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    LaunchpadBundle {
        entries,
        board: home.map(cohors_launchpad::read_board).unwrap_or_default(),
        recent_activity: home
            .map(|h| cohors_launchpad::recent_activity(h, RECENT_ACTIVITY))
            .unwrap_or_default(),
    }
}

/// The overview of `snapshots` as of `now`: reads the launchpad, then hands
/// everything to the pure `cohors_core::overview`. This is the one call every
/// front-end makes, which is what keeps their numbers identical.
pub fn overview(snapshots: &[RepoSnapshot], home: Option<&Path>, now: i64) -> Overview {
    let bundle = launchpad_bundle(snapshots, home);
    cohors_core::overview(snapshots, &bundle.entries, &bundle.board, now)
}

/// Commits by `author_email` in `[since, until)` across the fleet.
///
/// Each commit is tagged with the repository's *display name* (its alias when
/// one is configured), so a standup names repositories the way the rest of the
/// product does rather than by directory.
pub fn fleet_commits(
    snapshots: &[RepoSnapshot],
    author_email: &str,
    since: i64,
    until: i64,
) -> Vec<StandupCommit> {
    let mut commits = Vec::new();
    for snap in snapshots {
        let Some(path) = &snap.path else { continue };
        let mut found = cohors_git::collect_commits(path, author_email, since, until);
        for commit in &mut found {
            commit.repo = snap.name.clone();
        }
        commits.extend(found);
    }
    commits
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use cohors_core::RepoId;

    const NOW: i64 = 1_700_000_000;

    /// A snapshot pointing at `dir`, readable enough for the launchpad.
    fn snap(name: &str, dir: &Path) -> RepoSnapshot {
        let mut s = RepoSnapshot::errored(
            RepoId(name.to_string()),
            name,
            Some(Utf8PathBuf::from_path_buf(dir.to_path_buf()).unwrap()),
            "placeholder",
        );
        s.error = None;
        s
    }

    #[test]
    fn bundle_reads_each_repo_and_sorts_by_name() {
        let zed = tempfile::tempdir().unwrap();
        let app = tempfile::tempdir().unwrap();
        std::fs::create_dir(app.path().join(".principia")).unwrap();
        std::fs::write(
            app.path().join(".principia/repo.json"),
            r#"{"specVersion":1,"name":"app","flows":[],"agents":[],
                "tasks":[{"id":"ship","title":"Ship it","priority":"high"}]}"#,
        )
        .unwrap();

        let fleet = [snap("zed", zed.path()), snap("app", app.path())];
        let bundle = launchpad_bundle(&fleet, None);
        let names: Vec<&str> = bundle.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["app", "zed"]);
        assert!(bundle.entries[0].configured && !bundle.entries[1].configured);
        assert!(bundle.board.focus.is_empty() && bundle.recent_activity.is_empty());

        // The same read feeds the overview: the task shows up in the figures.
        let o = overview(&fleet, None, NOW);
        assert_eq!(o.figures.open_tasks, 1);
        assert_eq!(o.figures.high_tasks, 1);
        assert_eq!(o.figures.configured_repos, 1);
    }

    #[test]
    fn repos_without_a_path_are_skipped_not_fatal() {
        let mut remote_only = snap("remote", Path::new("/nowhere"));
        remote_only.path = None;
        assert!(launchpad_bundle(&[remote_only], None).entries.is_empty());
    }
}
