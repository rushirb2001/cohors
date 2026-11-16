//! What a live surface sends for one state of the fleet: the overview, and
//! the suggestions with, for each "resume", the conversation it reopens.
//!
//! The web server's `/api/next` and `cohors watch --json` both build their
//! payload here from a [`crate::FleetState`] (or, for the demo, from sample
//! data), so the editor extension and the browser show the same list. The
//! ranking itself is `cohors_core::suggest`; this only assembles its inputs
//! from what the watch already read (no second read of the launchpad) and
//! pairs each resume with its session, which the list alone does not name.

use std::path::Path;

use cohors_core::launchpad::{Board, LaunchpadEntry};
use cohors_core::suggest::{AgentSession, Suggestion, SuggestionKind};
use cohors_core::{Overview, RepoSnapshot};
use serde::Serialize;

/// How many suggestions a live surface shows.
pub const LIVE_SUGGESTIONS: usize = 6;

/// One suggestion as a live surface sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuggestionItem {
    #[serde(flatten)]
    pub suggestion: Suggestion,
    /// For a resume, the conversation it reopens: a front-end starts it
    /// with `cohors agent command resume --session <id> --repo <repo>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<AgentSession>,
}

/// The overview and the suggestions for one state of the fleet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveView {
    pub overview: Overview,
    pub suggestions: Vec<SuggestionItem>,
    /// Every agent conversation found in the fleet's repositories.
    pub sessions: Vec<AgentSession>,
}

/// Build the live view of `snapshots` with their launchpad `entries` and
/// `board`, as of `now`. `user_home` is where the coding agents keep their
/// conversations (`None` reads none, as for sample data); `here` is the
/// repository the user is in, if any.
pub fn live_view(
    snapshots: &[RepoSnapshot],
    entries: &[LaunchpadEntry],
    board: &Board,
    here: Option<&str>,
    user_home: Option<&Path>,
    now: i64,
) -> LiveView {
    let overview = cohors_core::overview(snapshots, entries, board, now);
    let sessions = user_home
        .map(|home| crate::agent_sessions(snapshots, home, now))
        .unwrap_or_default();
    let list = cohors_core::suggest::suggest(&overview, &sessions, here, now, LIVE_SUGGESTIONS);
    let suggestions = pair_sessions(list, &sessions, &overview);
    LiveView {
        overview,
        suggestions,
        sessions,
    }
}

/// Attach to each resume suggestion the session it was made from: the one
/// whose resume command, written the way the ranking writes it, is the
/// suggestion's command.
fn pair_sessions(
    list: Vec<Suggestion>,
    sessions: &[AgentSession],
    overview: &Overview,
) -> Vec<SuggestionItem> {
    let name_of = |id: &str| {
        overview
            .repos
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| id.rsplit('/').next().unwrap_or(id).to_string())
    };
    list.into_iter()
        .map(|suggestion| {
            let session = (suggestion.kind == SuggestionKind::Resume)
                .then(|| {
                    sessions.iter().find(|s| {
                        suggestion.repo.as_deref() == Some(s.repo.as_str())
                            && cohors_core::suggest::resume_command(s, &name_of(&s.repo))
                                == suggestion.command
                    })
                })
                .flatten()
                .cloned();
            SuggestionItem {
                suggestion,
                session,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    fn session(id: &str, repo: &str, age: i64) -> AgentSession {
        AgentSession {
            runner: "claude-code".to_string(),
            id: id.to_string(),
            repo: repo.to_string(),
            cwd: repo.to_string(),
            branch: Some("main".to_string()),
            title: format!("work in {id}"),
            last_active: NOW - age,
        }
    }

    #[test]
    fn each_resume_carries_its_session_and_nothing_else_does() {
        let fleet = cohors_core::demo::fleet(NOW);
        let (entries, board) = cohors_core::launchpad::demo();
        let overview = cohors_core::overview(&fleet, &entries, &board, NOW);
        let repo = fleet[0].id.0.clone();
        let sessions = vec![session("aaa", &repo, 600), session("bbb", &repo, 7_200)];
        let list = cohors_core::suggest::suggest(&overview, &sessions, None, NOW, 10);
        let items = pair_sessions(list, &sessions, &overview);

        let resumes: Vec<&SuggestionItem> = items
            .iter()
            .filter(|i| i.suggestion.kind == SuggestionKind::Resume)
            .collect();
        assert_eq!(resumes.len(), 2);
        for item in &resumes {
            let session = item.session.as_ref().expect("a resume names its session");
            assert!(item.suggestion.command.contains(&session.id));
        }
        assert!(
            items
                .iter()
                .filter(|i| i.suggestion.kind != SuggestionKind::Resume)
                .all(|i| i.session.is_none())
        );
        // The session sits beside the suggestion's own fields in JSON.
        let json = serde_json::to_value(resumes[0]).unwrap();
        assert_eq!(json["kind"], "resume");
        assert!(json["title"].is_string() && json["session"]["id"].is_string());
    }

    #[test]
    fn sample_data_reads_no_sessions() {
        let fleet = cohors_core::demo::fleet(NOW);
        let (entries, board) = cohors_core::launchpad::demo();
        let view = live_view(&fleet, &entries, &board, None, None, NOW);
        assert!(view.sessions.is_empty());
        assert!(
            !view.suggestions.is_empty(),
            "the demo fleet has steps to take"
        );
        assert_eq!(view.overview.figures.repos, fleet.len());
    }
}
