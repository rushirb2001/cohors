//! Repos remembered from a deep search (ADR-058).
//!
//! `cohors discover --deep` and `cohors scan --deep` find repos a normal search
//! does not reach: nested inside a repo below a root, or deeper than
//! `max_depth`. Those are written to `<cache>/known-repos.json`, and every
//! later normal search (the dashboard's refresh, the web page's poll, an
//! agent's `list_repos`) includes them without walking deep again. A running
//! dashboard watches the file's modification time and rescans when it changes,
//! so a deep search in another terminal shows up without a restart.
//!
//! It is a cache, not configuration: losing it only means running `--deep`
//! again. Entries are re-checked on every read (still a repo, under a root,
//! not ignored), so a deleted or newly ignored repo drops out by itself.

use std::time::SystemTime;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

/// The file's shape. A list of absolute repo paths, sorted.
#[derive(Debug, Default, Serialize, Deserialize)]
struct KnownFile {
    #[serde(default)]
    repos: Vec<String>,
}

/// Where the remembered repos live by default: `<cache_dir>/known-repos.json`.
pub fn default_path() -> Option<Utf8PathBuf> {
    cohors_config::paths::cache_dir()
        .ok()
        .map(|dir| dir.join("known-repos.json"))
}

/// The remembered repo paths. A missing or unreadable file is an empty list.
pub(crate) fn load(path: &Utf8Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<KnownFile>(&text).ok())
        .map(|file| file.repos)
        .unwrap_or_default()
}

/// Record what a deep search over `roots` found beyond a normal search's
/// reach. The search is authoritative for the folders it covered: earlier
/// entries under any of `roots` are replaced by `found`; entries elsewhere are
/// kept. The file is written whole through a temporary file, so a dashboard
/// reading it never sees half of it. Unchanged contents are not rewritten
/// (which would wake every dashboard for nothing).
pub(crate) fn remember(
    path: &Utf8Path,
    roots: &[Utf8PathBuf],
    found: &[Utf8PathBuf],
) -> std::io::Result<()> {
    let before = load(path);
    let mut repos: Vec<String> = before
        .iter()
        .filter(|p| !roots.iter().any(|root| Utf8Path::new(p).starts_with(root)))
        .cloned()
        .chain(found.iter().map(|p| p.to_string()))
        .collect();
    repos.sort();
    repos.dedup();
    if repos == before && path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(&KnownFile { repos }).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// When the remembered list last changed (`None` when there is none). A
/// dashboard compares this between ticks to know when to rescan.
pub(crate) fn stamp(path: &Utf8Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Utf8PathBuf {
        Utf8PathBuf::from(s)
    }

    #[test]
    fn a_deep_search_replaces_what_it_covered_and_keeps_the_rest() {
        let tmp = tempfile::TempDir::new().unwrap();
        let file = Utf8PathBuf::from_path_buf(tmp.path().join("known-repos.json")).unwrap();
        assert!(load(&file).is_empty(), "no file is an empty list");

        remember(&file, &[p("/code")], &[p("/code/a/x"), p("/code/b/y")]).unwrap();
        remember(&file, &[p("/oss")], &[p("/oss/z")]).unwrap();
        assert_eq!(load(&file), ["/code/a/x", "/code/b/y", "/oss/z"]);

        // A later deep search of /code that no longer finds /code/a/x.
        remember(&file, &[p("/code")], &[p("/code/b/y")]).unwrap();
        assert_eq!(load(&file), ["/code/b/y", "/oss/z"]);
    }

    #[test]
    fn an_unchanged_list_is_not_rewritten() {
        let tmp = tempfile::TempDir::new().unwrap();
        let file = Utf8PathBuf::from_path_buf(tmp.path().join("known-repos.json")).unwrap();
        remember(&file, &[p("/code")], &[p("/code/a")]).unwrap();
        let first = stamp(&file);
        std::thread::sleep(std::time::Duration::from_millis(20));
        remember(&file, &[p("/code")], &[p("/code/a")]).unwrap();
        assert_eq!(stamp(&file), first, "same contents, same mtime");
    }

    #[test]
    fn a_garbled_file_reads_as_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let file = Utf8PathBuf::from_path_buf(tmp.path().join("known-repos.json")).unwrap();
        std::fs::write(&file, "{ not json").unwrap();
        assert!(load(&file).is_empty());
    }
}
