//! Best-effort discovery of *where* the user keeps git repos.
//!
//! This powers the zero-config first run: when no roots are configured, the
//! Scanner falls back to these detected roots, and `cohors init` seeds the
//! config with them instead of a blind placeholder. Because every surface
//! (CLI, TUI, MCP) resolves through the Scanner, "just find my repos" behaves
//! identically everywhere.

use camino::{Utf8Path, Utf8PathBuf};

/// Directories developers commonly keep code in, relative to `$HOME`.
const COMMON: &[&str] = &[
    "code",
    "projects",
    "dev",
    "src",
    "work",
    "Developer",
    "repos",
    "git",
    "Documents/code",
    "Documents/GitHub",
    "go/src",
];

/// How deep to peek when deciding whether a candidate directory "contains repos"
/// — enough for both `~/code/repo` and `~/code/org/repo` layouts.
const PROBE_DEPTH: usize = 2;

/// Detect candidate roots that actually contain git repos: the common code
/// directories that exist and hold at least one repo (returned home-relative as
/// `~/x`), plus the current directory when it isn't already covered. Empty when
/// nothing is found — callers then fall back to the current directory.
pub fn detect_roots(home: Option<&Utf8Path>) -> Vec<String> {
    let mut display: Vec<String> = Vec::new();
    let mut absolute: Vec<Utf8PathBuf> = Vec::new();

    if let Some(home) = home {
        for name in COMMON {
            let dir = home.join(name);
            if dir.is_dir() && contains_repo(dir.as_std_path(), PROBE_DEPTH) {
                display.push(format!("~/{name}"));
                absolute.push(dir);
            }
        }
    }

    // Factor in the current directory. If it's *itself* a repo, the useful root
    // is its parent (where sibling repos live); otherwise, if it holds repos,
    // use it directly. Skip when already inside a detected root.
    if let Ok(cwd) = std::env::current_dir()
        && let Ok(cwd) = Utf8PathBuf::from_path_buf(cwd)
    {
        let candidate = if cwd.join(".git").is_dir() {
            cwd.parent().map(Utf8Path::to_owned)
        } else if contains_repo(cwd.as_std_path(), PROBE_DEPTH) {
            Some(cwd)
        } else {
            None
        };
        if let Some(candidate) = candidate
            && !absolute.iter().any(|root| candidate.starts_with(root))
        {
            let shown = home
                .and_then(|h| candidate.strip_prefix(h).ok())
                .map(|rel| format!("~/{rel}"))
                .unwrap_or_else(|| candidate.to_string());
            if !display.contains(&shown) {
                display.push(shown);
            }
        }
    }

    display
}

/// Shallow check: does `dir` (or a descendant within `depth`) contain a `.git`?
/// Stops at the first hit and skips dependency/build noise.
fn contains_repo(dir: &std::path::Path, depth: usize) -> bool {
    if dir.join(".git").exists() {
        return true;
    }
    if depth == 0 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str())
            && (name.starts_with('.')
                || matches!(name, "node_modules" | "target" | "vendor" | "Library"))
        {
            continue;
        }
        if contains_repo(&path, depth - 1) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch_repo(at: &std::path::Path) {
        fs::create_dir_all(at.join(".git")).unwrap();
    }

    #[test]
    fn contains_repo_finds_direct_and_nested() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // ~/code/myrepo/.git  → found from `code` at depth 2.
        touch_repo(&root.join("code/myrepo"));
        assert!(contains_repo(&root.join("code"), 2));
        // A bare, repo-less directory is not a root.
        fs::create_dir_all(root.join("empty/sub")).unwrap();
        assert!(!contains_repo(&root.join("empty"), 2));
    }

    #[test]
    fn detect_roots_picks_common_dirs_with_repos() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Utf8Path::from_path(tmp.path()).unwrap();
        touch_repo(tmp.path().join("code/alpha").as_path());
        touch_repo(tmp.path().join("projects/beta").as_path());
        // `dev` exists but holds no repo → excluded.
        fs::create_dir_all(tmp.path().join("dev/notes")).unwrap();

        let roots = detect_roots(Some(home));
        assert!(roots.contains(&"~/code".to_string()), "got {roots:?}");
        assert!(roots.contains(&"~/projects".to_string()), "got {roots:?}");
        assert!(!roots.contains(&"~/dev".to_string()), "got {roots:?}");
    }
}
