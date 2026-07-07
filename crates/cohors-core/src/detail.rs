//! The per-repo "drill-in" detail model: what the TUI's detail pane (and later
//! the web app) shows when you inspect a single repo. Pure data — adapters
//! (`cohors-git`) populate it; this crate just defines the shape.

use serde::{Deserialize, Serialize};

use crate::model::CommitMeta;

/// Everything the detail pane renders for one repo. Best-effort: any section may
/// be empty if it couldn't be read, but the struct itself is always produced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoDetail {
    /// The current branch name (or `None` for detached/unborn).
    pub current_branch: Option<String>,
    /// Recent commits, newest first (typically the last ~15).
    pub recent_commits: Vec<CommitMeta>,
    /// Working-tree changes (staged + unstaged + untracked).
    pub changed_files: Vec<ChangedFile>,
    /// Local branches, current first, each with its own upstream sync state.
    pub branches: Vec<BranchInfo>,
    /// Stash entries, newest first (the stash message).
    pub stashes: Vec<String>,
}

/// One changed path in the working tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    /// A short porcelain-style status, e.g. `" M"`, `"A "`, `"??"`.
    pub status: String,
    /// The path, relative to the repo root.
    pub path: String,
}

/// One side (index or worktree) of a two-character porcelain status, decoded.
/// The wire format stays the raw string (`ChangedFile::status`) for
/// compatibility; this is how consumers *interpret* it, so no front-end has to
/// pattern-match porcelain characters itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeState {
    /// Nothing on this side (the ' ' half of e.g. `" M"`).
    Clean,
    Added,
    Modified,
    Deleted,
    Renamed,
    Typechange,
    /// An unmerged path (`U` on either side).
    Conflicted,
    /// A brand-new untracked file (`??`).
    Untracked,
}

impl ChangeState {
    /// Decode one porcelain status character. Unknown characters read as
    /// `Clean` — an unrecognised code must never crash a render.
    fn from_code(c: char) -> Self {
        match c {
            'A' => Self::Added,
            'M' => Self::Modified,
            'D' => Self::Deleted,
            'R' => Self::Renamed,
            'T' => Self::Typechange,
            'U' => Self::Conflicted,
            '?' => Self::Untracked,
            _ => Self::Clean,
        }
    }
}

impl ChangedFile {
    /// The index (staged) half of the status — the first character.
    pub fn index_state(&self) -> ChangeState {
        self.status
            .chars()
            .next()
            .map_or(ChangeState::Clean, ChangeState::from_code)
    }

    /// The worktree (unstaged) half of the status — the second character.
    pub fn worktree_state(&self) -> ChangeState {
        self.status
            .chars()
            .nth(1)
            .map_or(ChangeState::Clean, ChangeState::from_code)
    }

    /// A brand-new file git isn't tracking yet (`??`).
    pub fn is_untracked(&self) -> bool {
        self.status.starts_with("??")
    }

    /// An unmerged path: any `U`, or the both-added / both-deleted conflict
    /// pairs (`AA`, `DD`) that porcelain uses without a `U`.
    pub fn is_conflicted(&self) -> bool {
        self.status.contains('U') || self.status == "AA" || self.status == "DD"
    }

    /// Whether some part of this change is staged (in the index). Untracked
    /// and conflicted paths are neither staged nor unstaged — resolve first.
    pub fn is_staged(&self) -> bool {
        !self.is_untracked() && !self.is_conflicted() && self.index_state() != ChangeState::Clean
    }
}

/// What is uncommitted in one repo: the changed-file list and, optionally, a
/// size-capped unified diff of the working tree. Adapters (`cohors-git`)
/// populate it; this crate just defines the shape. Pure data — WASM-safe.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoChanges {
    /// Each changed path with its short porcelain status (as in [`ChangedFile`]).
    pub files: Vec<ChangedFile>,
    /// A unified diff of the working tree against `HEAD`, present only when the
    /// caller asks for it. Size-capped — see `truncated`.
    pub patch: Option<String>,
    /// `true` when `patch` was cut off at the byte cap (so the agent knows the
    /// diff is partial).
    pub truncated: bool,
}

/// One local branch in the drill-in: its name and how it stands against its
/// own upstream — so "which branch has unpushed work" is visible per branch,
/// not just for HEAD.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    /// Commits ahead of / behind this branch's upstream (0/0 when in sync).
    #[serde(default)]
    pub ahead: u32,
    #[serde(default)]
    pub behind: u32,
    /// Whether the branch tracks an upstream at all (ahead/behind are only
    /// meaningful when true).
    #[serde(default)]
    pub has_upstream: bool,
    #[serde(default)]
    pub is_current: bool,
    /// The branch's own tip commit — so the dashboard can show each branch's last
    /// activity, not just HEAD's. `None` when it couldn't be read.
    #[serde(default)]
    pub last_commit: Option<CommitMeta>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(status: &str) -> ChangedFile {
        ChangedFile {
            status: status.to_string(),
            path: "src/lib.rs".to_string(),
        }
    }

    #[test]
    fn porcelain_codes_decode_to_typed_states() {
        assert_eq!(file(" M").index_state(), ChangeState::Clean);
        assert_eq!(file(" M").worktree_state(), ChangeState::Modified);
        assert_eq!(file("A ").index_state(), ChangeState::Added);
        assert_eq!(file("RM").index_state(), ChangeState::Renamed);
        assert_eq!(file("RM").worktree_state(), ChangeState::Modified);
        assert_eq!(file("??").index_state(), ChangeState::Untracked);
        // Unknown or missing characters degrade to Clean, never a panic.
        assert_eq!(file("X").worktree_state(), ChangeState::Clean);
        assert_eq!(file("").index_state(), ChangeState::Clean);
    }

    #[test]
    fn staged_untracked_conflicted_are_mutually_sensible() {
        assert!(file("??").is_untracked());
        assert!(!file("??").is_staged());

        assert!(file("A ").is_staged());
        assert!(file("MM").is_staged());
        assert!(!file(" M").is_staged());

        // All porcelain conflict pairs read as conflicted, not staged.
        for code in ["UU", "AU", "UA", "DU", "UD", "AA", "DD"] {
            assert!(file(code).is_conflicted(), "{code} should be conflicted");
            assert!(!file(code).is_staged(), "{code} should not read staged");
        }
        assert!(!file("A ").is_conflicted());
    }
}
