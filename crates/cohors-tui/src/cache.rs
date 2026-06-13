//! The snapshot cache for instant warm starts.
//!
//! After each scan the dashboard writes the result set to
//! `<cache>/cache.json`; on the next launch it paints that instantly (no git
//! I/O) and kicks off a background refresh. Everything here is best-effort: a
//! missing or unreadable cache simply means a cold start, never an error.
//!
//! v0.1 caches the whole result set and always refreshes in the background
//! (see DECISIONS.md). Per-repo, mtime-keyed incremental refresh is a v0.2
//! optimization.

use cohors_core::RepoSnapshot;

/// Load the cached snapshots, or `None` if there's no usable cache.
pub fn load() -> Option<Vec<RepoSnapshot>> {
    let path = cohors_config::paths::cache_file().ok()?;
    let bytes = std::fs::read(&path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(snapshots) => Some(snapshots),
        Err(err) => {
            tracing::debug!(%path, error = %err, "ignoring unreadable cache");
            None
        }
    }
}

/// Write snapshots to the cache (best-effort; failures are logged, not fatal).
pub fn save(snapshots: &[RepoSnapshot]) {
    let Ok(path) = cohors_config::paths::cache_file() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_vec(snapshots) {
        Ok(json) => {
            if let Err(err) = std::fs::write(&path, json) {
                tracing::debug!(%path, error = %err, "could not write cache");
            }
        }
        Err(err) => tracing::debug!(error = %err, "could not serialize cache"),
    }
}
