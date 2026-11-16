//! The one repository a coding agent's session is in.
//!
//! A session starts in a folder, not in "the fleet": what it needs to know is
//! what that folder's repository holds, and it needs to know quickly, since a
//! session waits on its start-up hooks. So this reads exactly one repository,
//! with no config, no discovery walk and no network, wherever it is on disk.

use camino::{Utf8Path, Utf8PathBuf};
use cohors_core::{RepoId, RepoRef, RepoSnapshot};

/// The repository a folder belongs to, as a session in that folder sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    /// The whole repository: its main checkout and every linked worktree.
    pub repo: RepoSnapshot,
    /// The linked worktree the folder is in, when it is in one.
    pub worktree: Option<String>,
}

/// The repository that owns whatever checkout `folder` is in, or `None` when
/// `folder` is not inside a git repository (or cannot be read as one).
pub fn checkout_at(folder: &Utf8Path, now: i64) -> Option<Checkout> {
    let (main, worktree): (Utf8PathBuf, Option<String>) = cohors_git::owning_repo(folder)?;
    let repo = cohors_git::snapshot_repo(
        &RepoRef {
            id: RepoId(main.to_string()),
            path: Some(main),
        },
        now,
    );
    (!repo.has_error()).then_some(Checkout { repo, worktree })
}
