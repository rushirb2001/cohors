//! `cohors-git` — the local data source.
//!
//! Discovers git repositories under the configured roots ([`discover`]) and
//! produces a [`cohors_core::RepoSnapshot`] per repo ([`snapshot_repo`]).
//! [`LocalGitProvider`] ties the two together as the local implementation of
//! [`cohors_core::RepoProvider`] and offers a parallel [`LocalGitProvider::scan`].
//!
//! Per ADR-004, `gix` (pure Rust) is primary for reads; `git2` (the default-on
//! `git2-fallback` feature) covers worktree status, ahead/behind, and stash
//! count, which gix 0.84 doesn't yet expose ergonomically. A failure in one
//! repo is captured in that snapshot's `error` field — never a panic, so one
//! bad repo can't crash the dashboard.
#![forbid(unsafe_code)]

mod discover;
mod error;
mod provider;
mod snapshot;

pub use discover::{DiscoveryOptions, discover};
pub use error::GitError;
pub use provider::LocalGitProvider;
pub use snapshot::snapshot_repo;
