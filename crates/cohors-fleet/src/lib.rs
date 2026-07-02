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

mod detect;
mod scanner;

pub use detect::detect_roots;
pub use scanner::{FleetError, Scanner};
