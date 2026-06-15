//! Content search inside a single repository.
//!
//! Backends in order of preference (ADR-023 open question): `rg` (fast, honors
//! `.gitignore`, searches all non-ignored files), then `git grep` (tracked
//! files), then a dependency-only fallback that walks with the `ignore` crate
//! and matches lines itself — so there is **no hard external dependency**.
//!
//! The query is matched as a **fixed string** (case-sensitive) across every
//! backend, so "find every repo calling `X`" behaves identically regardless of
//! which tool is present.

use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

/// One content match: the file (relative to the repo root), the 1-based line
/// number, and the line text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContentHit {
    pub path: Utf8PathBuf,
    pub line: u32,
    pub text: String,
}

/// Search `root` for `query`, returning at most `max_results` hits. Tries `rg`,
/// then `git grep`, then an in-process walk.
pub fn search_content(root: &Utf8Path, query: &str, max_results: usize) -> Vec<ContentHit> {
    if max_results == 0 || query.is_empty() {
        return Vec::new();
    }
    if let Some(hits) = ripgrep(root, query, max_results) {
        return hits;
    }
    if let Some(hits) = git_grep(root, query, max_results) {
        return hits;
    }
    walk_fallback(root, query, max_results)
}

/// `rg --line-number --no-heading --color never -F -- <query> .` run inside
/// `root`. `None` if `rg` isn't installed (so we fall through).
fn ripgrep(root: &Utf8Path, query: &str, max_results: usize) -> Option<Vec<ContentHit>> {
    let output = Command::new("rg")
        .current_dir(root)
        .args([
            "--line-number",
            "--no-heading",
            "--color",
            "never",
            "-F",
            "--",
            query,
            ".",
        ])
        .output()
        .ok()?; // rg not on PATH ⇒ fall through.
    // rg exits 1 when there are no matches; that's a valid (empty) result.
    Some(parse_grep_lines(
        &String::from_utf8_lossy(&output.stdout),
        max_results,
    ))
}

/// `git -C <root> grep -nI -F -e <query>` over tracked files. `None` if `git`
/// isn't installed; an empty result (exit 1) is authoritative for tracked files.
fn git_grep(root: &Utf8Path, query: &str, max_results: usize) -> Option<Vec<ContentHit>> {
    let output = Command::new("git")
        .args(["-C", root.as_str(), "grep", "-nI", "-F", "-e", query])
        .output()
        .ok()?;
    Some(parse_grep_lines(
        &String::from_utf8_lossy(&output.stdout),
        max_results,
    ))
}

/// Parse `path:line:text` lines (the shared `rg`/`git grep` format) into hits.
fn parse_grep_lines(stdout: &str, max_results: usize) -> Vec<ContentHit> {
    let mut hits = Vec::new();
    for raw in stdout.lines() {
        if hits.len() >= max_results {
            break;
        }
        // Split into path, line, and the (possibly colon-containing) text.
        let mut parts = raw.splitn(3, ':');
        let (Some(path), Some(line), Some(text)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let Ok(line) = line.parse::<u32>() else {
            continue;
        };
        hits.push(ContentHit {
            path: Utf8PathBuf::from(path.trim_start_matches("./")),
            line,
            text: text.to_string(),
        });
    }
    hits
}

/// In-process fallback: walk `root` with the `ignore` crate (honors
/// `.gitignore`) and substring-match each UTF-8 line.
fn walk_fallback(root: &Utf8Path, query: &str, max_results: usize) -> Vec<ContentHit> {
    let mut hits = Vec::new();
    for entry in ignore::WalkBuilder::new(root)
        .hidden(false)
        .build()
        .flatten()
    {
        if hits.len() >= max_results {
            break;
        }
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(entry.path()) else {
            continue; // binary / non-UTF-8 / unreadable — skip.
        };
        let rel = entry
            .path()
            .strip_prefix(root.as_std_path())
            .unwrap_or(entry.path());
        let Ok(rel) = Utf8PathBuf::from_path_buf(rel.to_path_buf()) else {
            continue;
        };
        for (i, line) in contents.lines().enumerate() {
            if hits.len() >= max_results {
                break;
            }
            if line.contains(query) {
                hits.push(ContentHit {
                    path: rel.clone(),
                    line: (i + 1) as u32,
                    text: line.to_string(),
                });
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn walk_fallback_finds_matches_with_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        fs::write(root.join("a.txt"), "alpha\nNEEDLE here\nomega\n").unwrap();
        fs::write(root.join("b.txt"), "nothing\n").unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/c.txt"), "x\ny NEEDLE\n").unwrap();

        let mut hits = walk_fallback(&root, "NEEDLE", 100);
        hits.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].path, Utf8PathBuf::from("a.txt"));
        assert_eq!(hits[0].line, 2);
        assert!(hits[0].text.contains("NEEDLE"));
        assert_eq!(hits[1].path, Utf8PathBuf::from("sub/c.txt"));
        assert_eq!(hits[1].line, 2);
    }

    #[test]
    fn walk_fallback_respects_max_results() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        fs::write(root.join("f.txt"), "hit\nhit\nhit\nhit\n").unwrap();
        assert_eq!(walk_fallback(&root, "hit", 2).len(), 2);
    }

    #[test]
    fn parse_handles_text_with_colons() {
        let hits = parse_grep_lines("./src/main.rs:42:let url = \"http://x\";\n", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, Utf8PathBuf::from("src/main.rs"));
        assert_eq!(hits[0].line, 42);
        assert!(hits[0].text.contains("http://x"));
    }
}
