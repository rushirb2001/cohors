//! Typed errors for the local git provider.

/// Errors that can abort *discovery* (enumerating repos). Per-repo read
/// failures are not errors here — they're captured in
/// [`cohors_core::RepoSnapshot::error`] so one bad repo never aborts a scan.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("invalid ignore glob {pattern:?}")]
    IgnoreGlob {
        pattern: String,
        #[source]
        source: globset::Error,
    },

    #[error("repository reference has no local path")]
    NoPath,
}
