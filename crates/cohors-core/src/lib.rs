//! `cohors-core` — the pure, I/O-free brain of cohors.
//!
//! All domain logic lives here so the TUI and the future web app share the
//! *exact same* analysis: the data models, the [`RepoProvider`] trait, and the
//! sort / filter / fuzzy-rank routines.
//!
//! To keep the WebAssembly target viable, this crate must stay free of
//! `std::fs`, `std::process`, `std::net`, thread spawning, and
//! `std::time::Instant`. All I/O happens in adapter crates (`cohors-git`,
//! later `cohors-github`) behind the provider trait.
//!
//! This is the scaffold; the real models and logic land in the next milestone
//! step.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    /// Smoke test confirming the workspace's test harness runs. Replaced by
    /// real model and logic tests in the next step.
    #[test]
    fn scaffold_builds() {
        assert_eq!(2 + 2, 4);
    }
}
