//! Weekly-standup aggregation (pure): group the user's commits across repos and
//! render them as a shareable markdown digest. Clock-injected like the rest of
//! core. (v0.2)
//!
//! `cohors-git` collects the commits ([`StandupCommit`]); this module turns them
//! into markdown ([`to_markdown`]).

use serde::{Deserialize, Serialize};

/// One commit authored by the user, tagged with its repo. Produced by
/// `cohors-git`'s standup collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandupCommit {
    pub repo: String,
    pub short_id: String,
    pub summary: String,
    /// Commit time, Unix seconds (UTC).
    pub timestamp: i64,
}

/// The time window for a standup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandupWindow {
    Today,
    Week,
    Custom { since: i64, until: i64 },
}

impl StandupWindow {
    /// Resolve to a `[since, until)` range (Unix seconds) against `now`. Day
    /// boundaries are UTC.
    pub fn range(self, now: i64) -> (i64, i64) {
        const DAY: i64 = 24 * 60 * 60;
        match self {
            StandupWindow::Today => (now - now.rem_euclid(DAY), now),
            StandupWindow::Week => (now - 7 * DAY, now),
            StandupWindow::Custom { since, until } => (since, until),
        }
    }

    /// Short label for the header, e.g. `today`, `this week`.
    pub fn label(self) -> &'static str {
        match self {
            StandupWindow::Today => "today",
            StandupWindow::Week => "this week",
            StandupWindow::Custom { .. } => "custom range",
        }
    }
}

/// Render the commits as markdown grouped by repo, for `window` (evaluated
/// against `now`). Implemented in the standup-aggregation step.
pub fn to_markdown(commits: &[StandupCommit], window: StandupWindow, now: i64) -> String {
    // Stub — filled by the standup-aggregation agent.
    let _ = (commits, window, now);
    String::new()
}
