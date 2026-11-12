//! `cohors-core` — the pure, I/O-free brain of cohors.
//!
//! All domain logic lives here so the TUI and the future web app share the
//! *exact same* analysis:
//!
//! - [`model`] — the data types every front-end renders ([`RepoSnapshot`] & co).
//! - [`provider`] — the [`RepoProvider`] trait that adapters implement.
//! - [`sort`] / [`fuzzy`] / [`view`] — "what to show, and in what order".
//! - [`time`] — clock-free relative-time formatting.
//! - [`launchpad`] — what each repo says about itself: flows, agents, tasks.
//! - [`overview`] — the one model every front-end draws: what to do next and why.
//! - [`agents`] — the agent runners cohors can start and the prompts it hands them.
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

pub mod agents;
pub mod attention;
pub mod demo;
pub mod detail;
pub mod fuzzy;
pub mod launchpad;
pub mod model;
pub mod overview;
pub mod provider;
pub mod search;
pub mod select;
pub mod sort;
pub mod standup;
pub mod time;
pub mod update;
pub mod view;

// Re-export the most-used types at the crate root so adapters can write
// `use cohors_core::RepoSnapshot;` instead of reaching into modules.
pub use attention::{Assessment, AttentionReason, FleetSummary, Severity, assess, fleet_summary};
pub use detail::{BranchInfo, ChangeState, ChangedFile, RepoChanges, RepoDetail};
pub use model::{
    Branch, CiStatus, CommitMeta, Contributor, PullRequest, RemoteDetail, RemoteInfo, RemoteKind,
    RepoId, RepoOperation, RepoRef, RepoSnapshot, TagInfo, Upstream, WorktreeStatus,
    classify_remote, remote_host,
};
pub use overview::{
    ActionKind, KeyFigures, NextAction, Overview, RepoCommits, RepoLine, RepoWork, SuggestedAction,
    fleet_activity, overview,
};
pub use provider::RepoProvider;
pub use search::{SearchHit, SearchKind, is_sensitive_dir, is_sensitive_path, search_metadata};
pub use select::{
    AttentionLevel, ParseSelectorError, Selector, expand_tilde as expand_selector_tilde, glob_name,
    parse_shorthand, resolve,
};
pub use sort::SortMode;
pub use standup::{StandupCommit, StandupWindow, group_commits, to_markdown};
pub use view::{ViewParams, ViewRow, compute_view};
