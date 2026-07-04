//! Comment-preserving, surgical edits to `config.toml`.
//!
//! `toml_edit` round-trips the document (comments, ordering, whitespace intact),
//! so cohors can *modify* the user's config instead of only writing the starter:
//! the TUI can append detected roots, a future `cohors config set` can flip one
//! key — without flattening a hand-annotated file. A missing file is seeded from
//! the starter template first, so edits always land in a fully-documented file.

use camino::Utf8Path;
use toml_edit::{Array, DocumentMut, value};

use crate::error::ConfigError;

/// Read the config document at `path`, or seed it from the starter template
/// (with no concrete roots) when the file doesn't exist yet.
fn load_document(path: &Utf8Path) -> Result<DocumentMut, ConfigError> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            crate::config::STARTER_CONFIG.to_string()
        }
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    content.parse().map_err(|e: toml_edit::TomlError| {
        // Surface the parse failure through the existing typed error by
        // round-tripping it as a toml serde error message.
        ConfigError::Edit {
            path: path.to_owned(),
            message: e.to_string(),
        }
    })
}

/// Write the document back, creating parent directories as needed.
fn store_document(path: &Utf8Path, doc: &DocumentMut) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: parent.to_owned(),
            source,
        })?;
    }
    std::fs::write(path, doc.to_string()).map_err(|source| ConfigError::Write {
        path: path.to_owned(),
        source,
    })
}

/// Replace `roots` with exactly `roots`, preserving every comment and all other
/// keys. Creates the file from the starter template when absent.
pub fn set_roots(path: &Utf8Path, roots: &[String]) -> Result<(), ConfigError> {
    let mut doc = load_document(path)?;
    let mut array = Array::new();
    for root in roots {
        array.push(root.as_str());
    }
    doc["roots"] = value(array);
    store_document(path, &doc)
}

/// Append `new_roots` to the existing `roots` (deduplicated, order kept),
/// preserving comments. This is what "adopt the detected roots" should call —
/// it must never clobber roots the user wrote by hand.
pub fn add_roots(path: &Utf8Path, new_roots: &[String]) -> Result<(), ConfigError> {
    let mut doc = load_document(path)?;
    let existing: Vec<String> = doc["roots"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut merged = existing.clone();
    for root in new_roots {
        if !merged.contains(root) {
            merged.push(root.clone());
        }
    }
    if merged == existing {
        return Ok(()); // nothing to do; don't rewrite the file
    }
    let mut array = Array::new();
    for root in &merged {
        array.push(root.as_str());
    }
    doc["roots"] = value(array);
    store_document(path, &doc)
}

/// Set one top-level string key (e.g. `editor = "hx"`, `icons = "ascii"`),
/// preserving comments. The caller is responsible for the key being meaningful —
/// [`crate::Config::parse`] will warn about unknown keys on the next load.
pub fn set_string(path: &Utf8Path, key: &str, val: &str) -> Result<(), ConfigError> {
    let mut doc = load_document(path)?;
    doc[key] = value(val);
    store_document(path, &doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;

    fn tmp_config(body: &str) -> (tempfile::TempDir, Utf8PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = Utf8PathBuf::from_path_buf(dir.path().join("config.toml")).unwrap();
        std::fs::write(&path, body).unwrap();
        (dir, path)
    }

    #[test]
    fn add_roots_appends_dedupes_and_keeps_comments() {
        let (_d, path) = tmp_config(
            "# my precious comment\nroots = [\"~/work\"]\n\n# trailing note\nmax_depth = 3\n",
        );
        add_roots(&path, &["~/oss".into(), "~/work".into()]).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("# my precious comment"), "{out}");
        assert!(out.contains("# trailing note"), "{out}");
        assert!(out.contains("max_depth = 3"), "{out}");

        // Round-trips through the real parser with the merged roots.
        let c = crate::Config::parse(&out, &path).unwrap();
        assert_eq!(c.roots, ["~/work", "~/oss"]);
        assert_eq!(c.max_depth, 3);
    }

    #[test]
    fn add_roots_is_a_noop_when_nothing_new() {
        let (_d, path) = tmp_config("roots = [\"~/work\"]\n");
        let before = std::fs::read_to_string(&path).unwrap();
        add_roots(&path, &["~/work".into()]).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn set_roots_replaces_and_missing_file_seeds_starter() {
        let dir = tempfile::tempdir().unwrap();
        let path = Utf8PathBuf::from_path_buf(dir.path().join("fresh.toml")).unwrap();
        set_roots(&path, &["~/a".into(), "~/b".into()]).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        // Seeded from the starter: its documentation comments survive.
        assert!(out.contains("# cohors configuration."), "{out}");
        let c = crate::Config::parse(&out, &path).unwrap();
        assert_eq!(c.roots, ["~/a", "~/b"]);
    }

    #[test]
    fn set_string_updates_one_key_in_place() {
        let (_d, path) = tmp_config("roots = [\"~/x\"]\n# about the editor\n# editor = \"code\"\n");
        set_string(&path, "editor", "hx").unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("# about the editor"), "{out}");
        let c = crate::Config::parse(&out, &path).unwrap();
        assert_eq!(c.editor.as_deref(), Some("hx"));
        assert_eq!(c.roots, ["~/x"]);
    }
}
