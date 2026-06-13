//! The local filesystem implementation of [`cohors_core::RepoProvider`].

use cohors_core::{RepoProvider, RepoRef, RepoSnapshot};
use rayon::prelude::*;

use crate::discover::{DiscoveryOptions, discover};
use crate::error::GitError;
use crate::snapshot::snapshot_repo;

/// Reads repos from the local filesystem under the configured roots.
pub struct LocalGitProvider {
    options: DiscoveryOptions,
}

impl LocalGitProvider {
    pub fn new(options: DiscoveryOptions) -> Self {
        Self { options }
    }

    pub fn options(&self) -> &DiscoveryOptions {
        &self.options
    }

    /// Discover repos and snapshot them all in parallel.
    ///
    /// This is where the synchronous [`RepoProvider`] gets its speed: per
    /// ADR-010 the trait stays sync/pure and the adapter parallelizes with
    /// `rayon`. Each task opens its own repo handles, so nothing git-related is
    /// shared across threads.
    pub fn scan(&self) -> Result<Vec<RepoSnapshot>, GitError> {
        let refs = self.list()?;
        Ok(refs.par_iter().map(snapshot_repo).collect())
    }
}

impl RepoProvider for LocalGitProvider {
    type Error = GitError;

    fn list(&self) -> Result<Vec<RepoRef>, GitError> {
        discover(&self.options)
    }

    fn snapshot(&self, repo: &RepoRef) -> Result<RepoSnapshot, GitError> {
        // Always returns a snapshot (read failures land in `error`), so this is
        // infallible in practice — the `Result` satisfies the trait.
        Ok(snapshot_repo(repo))
    }
}
