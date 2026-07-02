//! `cohors-core` — the pure, I/O-free brain of cohors.
//!
//! All domain logic lives here so the TUI and the future web app share the
//! *exact same* analysis:
//!
//! - [`model`] — the data types every front-end renders ([`RepoSnapshot`] & co).
//! - [`provider`] — the [`RepoProvider`] trait that adapters implement.
//! - [`sort`] / [`fuzzy`] / [`view`] — "what to show, and in what order".
//! - [`time`] — clock-free relative-time formatting.
//!
//! ## WASM safety (non-negotiable)
//!
//! To keep the WebAssembly target viable, this crate stays free of `std::fs`,
//! `std::process`, `std::net`, thread spawning, and `std::time::Instant`. All
//! I/O happens in adapter crates (`cohors-git`, later `cohors-github`) behind
//! [`RepoProvider`], and the current time is *injected* into [`time::relative`]
//! rather than read from the system clock. CI builds this crate for
//! `wasm32-unknown-unknown` to catch any regression.
#![forbid(unsafe_code)]

pub mod attention;
pub mod demo;
pub mod detail;
pub mod fuzzy;
pub mod model;
pub mod provider;
pub mod search;
pub mod select;
pub mod sort;
pub mod standup;
pub mod time;
pub mod view;

// Re-export the most-used types at the crate root so adapters can write
// `use cohors_core::RepoSnapshot;` instead of reaching into modules.
pub use attention::{Assessment, AttentionReason, FleetSummary, Severity, assess, fleet_summary};
pub use detail::{ChangedFile, RepoChanges, RepoDetail};
pub use model::{
    Branch, CiStatus, CommitMeta, Contributor, PullRequest, RemoteDetail, RemoteInfo, RepoId,
    RepoOperation, RepoRef, RepoSnapshot, Upstream, WorktreeStatus,
};
pub use provider::RepoProvider;
pub use search::{SearchHit, SearchKind, search_metadata};
pub use select::{AttentionLevel, Selector, glob_name, resolve};
pub use sort::SortMode;
pub use standup::{StandupCommit, StandupWindow, group_commits, to_markdown};
pub use view::{ViewParams, ViewRow, compute_view};
