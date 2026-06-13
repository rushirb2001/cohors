//! Repo discovery: walk the configured roots in parallel and collect the
//! directories that contain a `.git`.
//!
//! Uses the `ignore` crate's parallel walker (we turn *off* its gitignore
//! behavior — we want the raw filesystem, including hidden `.git` dirs) plus a
//! `globset` for the user's ignore patterns. Discovery is resilient: an
//! unreadable directory is logged and skipped, never fatal.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use camino::Utf8PathBuf;
use cohors_core::{RepoId, RepoRef};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::{WalkBuilder, WalkState};

use crate::error::GitError;

/// Inputs to discovery. The binary fills this from `cohors-config` (roots are
/// already `~`-expanded; globs are still allowed and expanded here).
#[derive(Debug, Clone)]
pub struct DiscoveryOptions {
    /// Directories or globs to search (e.g. `/home/me/projects`, `/home/me/work/*`).
    pub roots: Vec<String>,
    /// Glob patterns whose matching directories are skipped.
    pub ignore: Vec<String>,
    /// How deep to descend from each root (depth 0 = the root itself).
    pub max_depth: usize,
    /// Stop descending once a repo is found (don't look for nested repos).
    pub stop_at_repo: bool,
    /// Follow symlinks while walking.
    pub follow_symlinks: bool,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            ignore: Vec::new(),
            max_depth: 4,
            stop_at_repo: true,
            follow_symlinks: false,
        }
    }
}

/// Discover all repos under the configured roots, deduped by canonical path and
/// returned in a stable (sorted) order.
pub fn discover(opts: &DiscoveryOptions) -> Result<Vec<RepoRef>, GitError> {
    let globs = Arc::new(build_globset(&opts.ignore)?);

    let root_dirs = expand_roots(&opts.roots);
    if root_dirs.is_empty() {
        return Ok(Vec::new());
    }

    // Collected from many walker threads, so guard it behind a mutex.
    let found = Arc::new(Mutex::new(Vec::<Utf8PathBuf>::new()));

    let mut builder = WalkBuilder::new(&root_dirs[0]);
    for dir in &root_dirs[1..] {
        builder.add(dir);
    }
    builder
        .hidden(false) // we *need* to see `.git`
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .parents(false)
        .require_git(false)
        .follow_links(opts.follow_symlinks)
        .max_depth(Some(opts.max_depth));

    let stop_at_repo = opts.stop_at_repo;
    builder.build_parallel().run(|| {
        let found = Arc::clone(&found);
        let globs = Arc::clone(&globs);
        Box::new(move |result| {
            let entry = match result {
                Ok(e) => e,
                Err(err) => {
                    tracing::debug!(error = %err, "skipping unreadable entry during discovery");
                    return WalkState::Continue;
                }
            };

            // We only reason about directories.
            if !entry.file_type().is_some_and(|t| t.is_dir()) {
                return WalkState::Continue;
            }

            // Never descend into a `.git` directory itself.
            if entry.path().file_name().is_some_and(|n| n == ".git") {
                return WalkState::Skip;
            }

            let Ok(dir) = Utf8PathBuf::from_path_buf(entry.path().to_path_buf()) else {
                return WalkState::Continue; // non-UTF-8 path: skip quietly
            };

            // Honor the user's ignore globs.
            if globs.is_match(dir.as_std_path()) {
                return WalkState::Skip;
            }

            // A directory containing `.git` (dir or file) is a repo.
            if dir.join(".git").exists() {
                let canonical = dir.canonicalize_utf8().unwrap_or(dir);
                if let Ok(mut guard) = found.lock() {
                    guard.push(canonical);
                }
                if stop_at_repo {
                    return WalkState::Skip;
                }
            }
            WalkState::Continue
        })
    });

    // Dedupe (a repo can be reached via multiple roots) and sort for stable output.
    let unique: BTreeSet<Utf8PathBuf> = Arc::try_unwrap(found)
        .expect("all walker threads finished")
        .into_inner()
        .expect("mutex not poisoned")
        .into_iter()
        .collect();

    Ok(unique
        .into_iter()
        .map(|path| RepoRef {
            id: RepoId(path.as_str().to_string()),
            path: Some(path),
        })
        .collect())
}

/// Expand root patterns (globs allowed) into existing directories.
fn expand_roots(roots: &[String]) -> Vec<Utf8PathBuf> {
    let mut dirs = Vec::new();
    for pattern in roots {
        match glob::glob(pattern) {
            Ok(paths) => dirs.extend(
                paths
                    .flatten()
                    .filter(|p| p.is_dir())
                    .filter_map(|p| Utf8PathBuf::from_path_buf(p).ok()),
            ),
            Err(err) => tracing::warn!(pattern, error = %err, "ignoring invalid root glob"),
        }
    }
    dirs
}

/// Compile ignore globs into a [`GlobSet`].
///
/// For a directory-tree pattern like `**/node_modules/**` we also add the
/// matching directory itself (`**/node_modules`) so the directory is pruned
/// immediately rather than after descending one level.
fn build_globset(patterns: &[String]) -> Result<GlobSet, GitError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        add_glob(&mut builder, pattern)?;
        if let Some(dir_pattern) = pattern.strip_suffix("/**") {
            add_glob(&mut builder, dir_pattern)?;
        }
    }
    builder.build().map_err(|source| GitError::IgnoreGlob {
        pattern: "<glob set>".to_string(),
        source,
    })
}

fn add_glob(builder: &mut GlobSetBuilder, pattern: &str) -> Result<(), GitError> {
    let glob = Glob::new(pattern).map_err(|source| GitError::IgnoreGlob {
        pattern: pattern.to_string(),
        source,
    })?;
    builder.add(glob);
    Ok(())
}
