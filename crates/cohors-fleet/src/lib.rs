//! cohors-fleet — the composition layer that turns *configuration* into a
//! *scanned fleet*.
//!
//! `cohors-git` knows how to read repos but not where to look; `cohors-config`
//! knows the user's settings but does no I/O; the front-ends should do neither.
//! This crate wires them together once — config → [`DiscoveryOptions`] →
//! parallel snapshot → groups/aliases stamping — so the TUI, MCP, and web all
//! scan the same way instead of each re-composing (or worse, skipping) the
//! config half. Native by nature (disk + subprocess); never a `cohors-core` dep.

#![forbid(unsafe_code)]

mod detail;
mod detect;
mod scanner;

pub use detail::{DetailBundle, detail_bundle};
pub use detect::detect_roots;
pub use scanner::{FleetError, Scanner};

// ── The read facade ──────────────────────────────────────────────────────────
// Front-ends read the fleet through THIS crate only — never `cohors-git` or
// `cohors-github` directly (a discipline test in tests/ enforces it). That makes
// fleet the single choke point for read behavior: an adapter change lands here
// once instead of flooding into every surface, and a new read capability added
// here is immediately available to all of them. Writes go through
// `cohors-actions`, the write-side peer of this facade.
pub use cohors_git::{
    ContentHit, collect_commits, repo_changes, repo_detail, search_content, snapshot_repo,
};
pub use cohors_github::{discover_token, enrich, fetch_repo_detail};
