//! Repo discovery: walk the configured roots in parallel and collect the
//! directories that contain a `.git`.
//!
//! Uses the `ignore` crate's parallel walker (we turn *off* its gitignore
//! behavior — we want the raw filesystem, including hidden `.git` dirs) plus a
//! `globset` for the user's ignore patterns. Discovery is resilient: an
//! unreadable directory is logged and skipped, never fatal.
//!
//! ## Nested repos (ADR-058)
//!
//! A repo can hold other repos: `~/code` tracked as a repo of private notes,
//! with `~/code/projects/api-dashboard` cloned inside it. Three rules decide
//! what is found:
//!
//! 1. **A root is always walked**, even when the root is itself a repo. Its
//!    nested repos are found at any depth up to `max_depth`.
//! 2. **Below a root**, `stop_at_repo` (the default) does not enter a repo it
//!    found; `deep` enters every repo and drops the depth limit.
//! 3. **Inside a repo**, the walk skips what cannot be a project of its own:
//!    build output and dependency folders ([`GENERATED_DIRS`]), and the
//!    repo's own submodules and linked worktrees, which belong to the repo
//!    that owns them rather than being counted twice.
//!
//! Separately, `known` carries repos found earlier (by a `--deep` search):
//! each one still on disk, under a root and not ignored joins the result, so a
//! dashboard keeps them without walking deep on every refresh.

use std::collections::{BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use camino::{Utf8Path, Utf8PathBuf};
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
    /// Ignored when `deep` is set.
    pub max_depth: usize,
    /// Below a root, don't enter a repo once it is found (don't look for
    /// repos nested inside it). A root that is itself a repo is always
    /// entered. Ignored when `deep` is set.
    pub stop_at_repo: bool,
    /// Follow symlinks while walking.
    pub follow_symlinks: bool,
    /// The exhaustive search: no depth limit, and every repo is entered to
    /// look for repos nested inside it.
    pub deep: bool,
    /// Repos found by an earlier search (absolute paths). Each one that is
    /// still a repo, sits under a root and matches no ignore glob is added to
    /// the result, wherever it is, without walking to it.
    pub known: Vec<String>,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            ignore: Vec::new(),
            max_depth: 4,
            stop_at_repo: true,
            follow_symlinks: false,
            deep: false,
            known: Vec::new(),
        }
    }
}

/// Folder names that, *inside a repo*, hold build output or downloaded
/// dependencies rather than projects. A git clone found in one of them is a
/// package manager's copy (a git dependency, a vendored library, CMake's
/// `_deps`), not a repo the developer works in, and the trees are large, so
/// the walk never enters them. Outside a repo they are walked like any folder
/// (someone may well keep repos in `~/build`).
pub const GENERATED_DIRS: &[&str] = &[
    "node_modules",
    "bower_components",
    "target",
    "build",
    "dist",
    ".build",
    "vendor",
    "Pods",
    "DerivedData",
    ".gradle",
    ".venv",
    "venv",
    "__pycache__",
    ".tox",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".terraform",
    ".cache",
    ".pnpm-store",
    ".yarn",
    ".cargo",
];

/// Discover all repos under the configured roots, deduped by canonical path and
/// returned in a stable (sorted) order. Includes the admissible `known` repos.
pub fn discover(opts: &DiscoveryOptions) -> Result<Vec<RepoRef>, GitError> {
    let globs = build_globset(&opts.ignore)?;
    let root_dirs = expand_roots(&opts.roots);
    if root_dirs.is_empty() {
        return Ok(Vec::new());
    }
    let mut unique = walk(opts, &root_dirs, globs.clone());
    unique.extend(admit_known(&opts.known, &root_dirs, &globs));
    Ok(to_refs(unique))
}

/// Of `known`, the repos that would join a discovery with these options: still
/// a repo on disk, under one of the roots, inside no ignored or sensitive
/// folder, and not a submodule or worktree of a repo that encloses it. Returned
/// canonical and sorted. Lets a caller tell remembered repos from walked ones.
pub fn admit_known_repos(opts: &DiscoveryOptions) -> Result<Vec<Utf8PathBuf>, GitError> {
    let globs = build_globset(&opts.ignore)?;
    let root_dirs = expand_roots(&opts.roots);
    Ok(admit_known(&opts.known, &root_dirs, &globs)
        .into_iter()
        .collect())
}

/// The roots' existing directories (globs expanded), canonical — the folders a
/// search with these options covers.
pub fn root_dirs(opts: &DiscoveryOptions) -> Vec<Utf8PathBuf> {
    expand_roots(&opts.roots)
        .into_iter()
        .map(|r| r.canonicalize_utf8().unwrap_or(r))
        .collect()
}

/// The parallel walk itself: every repo reachable under `root_dirs`.
fn walk(
    opts: &DiscoveryOptions,
    root_dirs: &[Utf8PathBuf],
    globs: GlobSet,
) -> BTreeSet<Utf8PathBuf> {
    let globs = Arc::new(globs);

    // Collected from many walker threads, so guard it behind a mutex.
    let found = Arc::new(Mutex::new(Vec::<Utf8PathBuf>::new()));
    // The walk paths (as the walker spells them) of every repo seen so far, so
    // an entry can tell whether it sits inside a repo. The walker reads a
    // directory's children only after the visitor returned for the directory
    // itself, so a parent is always recorded before its children look.
    let repo_dirs = Arc::new(RwLock::new(HashSet::<PathBuf>::new()));

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
        .max_depth(if opts.deep {
            None
        } else {
            Some(opts.max_depth)
        });

    // Below a root, a found repo is left unentered unless the search is deep.
    let stop_at_repo = opts.stop_at_repo && !opts.deep;
    builder.build_parallel().run(|| {
        let found = Arc::clone(&found);
        let globs = Arc::clone(&globs);
        let repo_dirs = Arc::clone(&repo_dirs);
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
            let depth = entry.depth();
            let name = entry.path().file_name().and_then(|n| n.to_str());

            // Never descend into a `.git` directory itself.
            if name == Some(".git") {
                return WalkState::Skip;
            }

            // Never descend into secret stores (`~` as a root must not walk
            // `~/.ssh`, `~/.aws`, …) — the core privacy rule the content search
            // uses. Depth 0 is exempt: a root the user configured explicitly is
            // their call, even if it's a directory we'd otherwise prune.
            if depth > 0 && name.is_some_and(cohors_core::is_sensitive_dir) {
                return WalkState::Skip;
            }

            // Inside a repo, build output and dependency folders are never
            // projects of their own (see GENERATED_DIRS).
            if depth > 0
                && name.is_some_and(|n| GENERATED_DIRS.contains(&n))
                && inside_repo(entry.path(), depth, &repo_dirs)
            {
                return WalkState::Skip;
            }

            let Ok(dir) = Utf8PathBuf::from_path_buf(entry.path().to_path_buf()) else {
                return WalkState::Continue; // non-UTF-8 path: skip quietly
            };

            // Honor the user's ignore globs.
            if globs.is_match(dir.as_std_path()) {
                return WalkState::Skip;
            }

            // A directory containing `.git` (dir or file) is a repo — and a
            // directory that IS a git dir (a bare repo / mirror: `HEAD` +
            // `objects/` + `refs/`, no worktree) counts too, so server-style
            // clones like `foo.git` show up in the fleet.
            let dot_git = dir.join(".git");
            let has_dot_git = dot_git.exists();
            if !has_dot_git && !is_bare_repo(&dir) {
                return WalkState::Continue;
            }

            // A bare repo inside another repo's worktree is never listed: git
            // lets a bare repo be committed (a `.git` dir it refuses), so in a
            // cloned project it is attacker-controlled, and acting on it would
            // run git under its config (hooks, fsmonitor, sshCommand). Real
            // mirrors live outside worktrees.
            if !has_dot_git && depth > 0 && inside_repo(entry.path(), depth, &repo_dirs) {
                return WalkState::Skip;
            }

            // A `.git` *file* inside another repo is that repo's submodule or
            // linked worktree: part of the repo that owns it, not a second
            // repo. (A worktree kept beside its repo is still listed, as it
            // always was: it is a checkout of its own, outside the owner.)
            if depth > 0 && dot_git.is_file() && owned_by_enclosing_repo(&dir) {
                return WalkState::Skip;
            }

            let canonical = dir.canonicalize_utf8().unwrap_or(dir);
            if let Ok(mut guard) = found.lock() {
                guard.push(canonical);
            }
            if let Ok(mut guard) = repo_dirs.write() {
                guard.insert(entry.path().to_path_buf());
            }
            // A bare repo is a git dir: nothing inside it is a project.
            if !has_dot_git {
                return WalkState::Skip;
            }
            // A root is always walked (rule 1); below it, stop_at_repo decides.
            if depth > 0 && stop_at_repo {
                return WalkState::Skip;
            }
            WalkState::Continue
        })
    });

    // Dedupe (a repo can be reached via multiple roots) and sort for stable output.
    Arc::try_unwrap(found)
        .expect("all walker threads finished")
        .into_inner()
        .expect("mutex not poisoned")
        .into_iter()
        .collect()
}

fn to_refs(paths: BTreeSet<Utf8PathBuf>) -> Vec<RepoRef> {
    paths
        .into_iter()
        .map(|path| RepoRef {
            id: RepoId(path.as_str().to_string()),
            path: Some(path),
        })
        .collect()
}

/// Does `path` (at walk depth `depth`) sit inside a repo the walk has seen? Its
/// ancestors up to and including the root are checked against `repo_dirs`.
fn inside_repo(path: &Path, depth: usize, repo_dirs: &RwLock<HashSet<PathBuf>>) -> bool {
    let Ok(seen) = repo_dirs.read() else {
        return false;
    };
    path.ancestors()
        .skip(1)
        .take(depth)
        .any(|ancestor| seen.contains(ancestor))
}

/// Is `dir` (whose `.git` is a file) a submodule or linked worktree of a repo
/// that encloses it? Its `gitdir:` points into `<owner>/.git/modules/…` or
/// `<owner>/.git/worktrees/…` (or `<owner>.git/worktrees/…` for a bare owner);
/// it is owned when `<owner>` is an ancestor of `dir`.
fn owned_by_enclosing_repo(dir: &Utf8Path) -> bool {
    let Some(owner) = owner_of_gitdir_file(dir) else {
        return false;
    };
    let here = dir
        .canonicalize_utf8()
        .unwrap_or_else(|_| dir.to_path_buf());
    here != owner && here.starts_with(&owner)
}

/// The repo a `.git` file's `gitdir:` points into, canonical, when it points
/// into another repo's `modules/` or `worktrees/` (otherwise `None`).
fn owner_of_gitdir_file(dir: &Utf8Path) -> Option<Utf8PathBuf> {
    let text = std::fs::read_to_string(dir.join(".git")).ok()?;
    let target = text
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))?
        .trim();
    let gitdir = if Path::new(target).is_absolute() {
        PathBuf::from(target)
    } else {
        dir.as_std_path().join(target)
    };
    let gitdir = gitdir.canonicalize().ok()?;
    let parts: Vec<Component> = gitdir.components().collect();
    // The first `<x>.git/modules` or `<x>.git/worktrees` pair names the owner's
    // git dir; a nested submodule (`.git/modules/a/modules/b`) still resolves to
    // the outermost repo, which is the one that encloses it on disk.
    let at = parts.windows(2).position(|pair| {
        let git_dir = pair[0].as_os_str().to_str().unwrap_or("");
        let kind = pair[1].as_os_str();
        git_dir.ends_with(".git") && (kind == "modules" || kind == "worktrees")
    })?;
    let owner_git: PathBuf = parts[..=at].iter().collect();
    // `<owner>/.git` → `<owner>`; a bare owner (`foo.git`) is the repo itself.
    let owner = if owner_git.file_name().is_some_and(|n| n == ".git") {
        owner_git.parent()?.to_path_buf()
    } else {
        owner_git
    };
    Utf8PathBuf::from_path_buf(owner).ok()
}

/// See [`admit_known_repos`].
fn admit_known(
    known: &[String],
    root_dirs: &[Utf8PathBuf],
    globs: &GlobSet,
) -> BTreeSet<Utf8PathBuf> {
    if known.is_empty() {
        return BTreeSet::new();
    }
    let roots: Vec<Utf8PathBuf> = root_dirs
        .iter()
        .map(|r| r.canonicalize_utf8().unwrap_or_else(|_| r.clone()))
        .collect();
    known
        .iter()
        .filter_map(|raw| {
            let path = Utf8PathBuf::from(raw).canonicalize_utf8().ok()?;
            let root = roots.iter().find(|root| path.starts_with(root))?;
            if !(path.join(".git").exists() || is_bare_repo(&path)) {
                return None; // gone, or no longer a repo
            }
            // Every folder from just below the root down to the repo must pass
            // the same checks the walk applies on its way there.
            let below = path.strip_prefix(root).ok()?;
            let mut at = root.clone();
            for part in below.components() {
                let name = part.as_str();
                at.push(name);
                if name == ".git"
                    || cohors_core::is_sensitive_dir(name)
                    || globs.is_match(at.as_std_path())
                {
                    return None;
                }
            }
            if path.join(".git").is_file() && owned_by_enclosing_repo(&path) {
                return None;
            }
            // A bare repo inside a worktree is never admitted (see the walk).
            if !path.join(".git").exists()
                && path
                    .ancestors()
                    .skip(1)
                    .take_while(|a| a.starts_with(root))
                    .any(|a| a.join(".git").exists())
            {
                return None;
            }
            Some(path)
        })
        .collect()
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

/// Does `dir` look like a bare repository (it *is* the git dir)? All three
/// hallmarks are required, so an ordinary source tree can't false-positive.
fn is_bare_repo(dir: &Utf8Path) -> bool {
    dir.join("HEAD").is_file() && dir.join("objects").is_dir() && dir.join("refs").is_dir()
}
