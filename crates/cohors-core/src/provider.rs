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

    /// Discover and snapshot the whole fleet: [`list`](Self::list), then one
    /// [`snapshot`](Self::snapshot) per repo. A per-repo `Err` degrades to an
    /// errored snapshot rather than aborting the scan.
    ///
    /// This provided default is **sequential** (core stays WASM-safe — no
    /// threads); native providers override it with a parallel fan-out (the
    /// local git provider uses `rayon`).
    fn scan_fleet(&self) -> Result<Vec<RepoSnapshot>, Self::Error> {
        let refs = self.list()?;
        Ok(refs
            .iter()
            .map(|r| {
                self.snapshot(r).unwrap_or_else(|_| {
                    RepoSnapshot::errored(
                        r.id.clone(),
                        r.path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .unwrap_or("(unknown)"),
                        r.path.clone(),
                        "provider failed to snapshot this repo",
                    )
                })
            })
            .collect())
    }

    /// A human label for this data source (for logs/diagnostics).
    fn source_name(&self) -> &'static str {
        "unknown"
    }
}
