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
    /// Local branch names, current first.
    pub branches: Vec<String>,
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
