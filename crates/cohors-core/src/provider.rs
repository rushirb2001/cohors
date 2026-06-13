//! The [`RepoProvider`] abstraction.
//!
//! Front-ends depend on this trait, never on a concrete data source, so the
//! local-git and (v0.2) GitHub adapters are interchangeable.
//!
//! Per **ADR-010** the trait is **synchronous and CPU-pure**: adapters do their
//! own I/O and parallelism and hand `cohors-core` finished data. Network-bound
//! work (the GitHub adapter) gets its own `async` API in that adapter which
//! *returns* these same models — the shared trait never becomes `async`, which
//! keeps this crate runtime-agnostic and WASM-clean.

use crate::model::{RepoRef, RepoSnapshot};

/// A source of repositories and their snapshots.
pub trait RepoProvider {
    /// Error type for this provider's I/O.
    type Error;

    /// Enumerate the repos this provider knows about. Cheap by contract: no
    /// heavy status work yet, just discovery.
    fn list(&self) -> Result<Vec<RepoRef>, Self::Error>;

    /// Produce a full snapshot for one repo.
    ///
    /// Implementations should capture per-repo failures in
    /// [`RepoSnapshot::error`] and still return `Ok`, so one bad repo never
    /// aborts a whole scan. Reserve `Err` for failures that prevent producing
    /// any snapshot at all.
    fn snapshot(&self, repo: &RepoRef) -> Result<RepoSnapshot, Self::Error>;
}
