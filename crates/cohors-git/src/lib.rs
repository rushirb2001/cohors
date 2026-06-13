//! `cohors-git` — the local data source.
//!
//! Discovers git repositories under the configured roots and produces a
//! `cohors_core` snapshot per repo via `gix` (with a feature-gated `git2`
//! fallback for paths gix doesn't cover yet). This is the native
//! implementation of `cohors_core`'s `RepoProvider` trait.
//!
//! A failure in one repo is captured in that snapshot's `error` field and
//! never propagated as a panic — one bad repo must not crash the dashboard.
//!
//! Scaffold for now; discovery and snapshotting land in the git milestone step.
#![forbid(unsafe_code)]
