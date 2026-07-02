//! Small, best-effort user preferences that aren't worth editing `config.toml`
//! for — currently just the default editor chosen from the "Open with…" picker.
//!
//! Stored as `<cache>/prefs.json` (next to the snapshot cache). Like the cache,
//! every operation here is best-effort: a missing or unreadable file just means
//! "no saved default", never an error.

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
struct Prefs {
    /// The editor command the user picked as their default (e.g. `"code"`).
    #[serde(default)]
    editor: Option<String>,
}

fn load() -> Prefs {
    let Ok(path) = crate::paths::cache_dir() else {
        return Prefs::default();
    };
    let path = path.join("prefs.json");
    std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// The saved default editor command, if any.
pub fn default_editor() -> Option<String> {
    load().editor.filter(|s| !s.trim().is_empty())
}

/// Persist `command` as the default editor (best-effort; logged, not fatal).
pub fn set_default_editor(command: &str) {
    let Ok(dir) = crate::paths::cache_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("prefs.json");
    let prefs = Prefs {
        editor: Some(command.to_string()),
    };
    match serde_json::to_vec_pretty(&prefs) {
        Ok(json) => {
            if let Err(err) = std::fs::write(&path, json) {
                tracing::debug!(%path, error = %err, "could not write prefs");
            }
        }
        Err(err) => tracing::debug!(error = %err, "could not serialize prefs"),
    }
}
