//! Fixture-based integration tests for discovery and snapshotting.
//!
//! Each test builds throwaway git repos with the real `git` CLI (pinned author/
//! committer dates so commit timestamps are deterministic) and asserts the
//! resulting [`RepoSnapshot`]. Requires `git` on `PATH` — which any environment
//! running cohors's tests will have.

use std::path::Path;
use std::process::Command;

use camino::Utf8PathBuf;
use cohors_core::{Branch, RepoId, RepoRef};
use cohors_git::{DiscoveryOptions, discover, snapshot_repo};
use tempfile::TempDir;

/// All fixture commits use this instant (`@<unix-ts> <tz>` is git's unambiguous
/// internal date format), so snapshot timestamps are exact.
const COMMIT_TS: i64 = 1_622_548_800; // 2021-06-01T12:00:00Z
const GIT_DATE: &str = "@1622548800 +0000";

/// A `git` command pre-wired with a fixed identity/date and no global/system
/// config, for reproducible fixtures.
fn git_cmd(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Dev")
        .env("GIT_AUTHOR_EMAIL", "dev@example.com")
        .env("GIT_COMMITTER_NAME", "Dev")
        .env("GIT_COMMITTER_EMAIL", "dev@example.com")
        .env("GIT_AUTHOR_DATE", GIT_DATE)
        .env("GIT_COMMITTER_DATE", GIT_DATE)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null");
    cmd
}

fn git(dir: &Path, args: &[&str]) {
    let out = git_cmd(dir).args(args).output().expect("failed to run git");
    assert!(
        out.status.success(),
        "git {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_output(dir: &Path, args: &[&str]) -> String {
    let out = git_cmd(dir).args(args).output().expect("failed to run git");
    assert!(
        out.status.success(),
        "git {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// `git init` (default branch `main`) at `dir`, creating it if needed.
fn init_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["-c", "init.defaultBranch=main", "init", "-q"]);
}

/// Init a repo with a single committed file.
fn init_repo_with_commit(dir: &Path, file: &str, content: &str, message: &str) {
    init_repo(dir);
    std::fs::write(dir.join(file), content).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", message]);
}

fn write(dir: &Path, file: &str, content: &str) {
    std::fs::write(dir.join(file), content).unwrap();
}

fn repo_ref(path: &Path) -> RepoRef {
    let p = Utf8PathBuf::from_path_buf(path.to_path_buf()).unwrap();
    RepoRef {
        id: RepoId(p.as_str().to_string()),
        path: Some(p),
    }
}

fn opts_for(root: &Path) -> DiscoveryOptions {
    DiscoveryOptions {
        roots: vec![root.to_str().unwrap().to_string()],
        ignore: Vec::new(),
        max_depth: 6,
        stop_at_repo: true,
        follow_symlinks: false,
    }
}

// ----- snapshot tests -------------------------------------------------------

#[test]
fn clean_repo_snapshot() {
    let tmp = TempDir::new().unwrap();
    init_repo_with_commit(tmp.path(), "README.md", "hi", "init: first commit");

    let snap = snapshot_repo(&repo_ref(tmp.path()), COMMIT_TS);

    assert!(snap.error.is_none(), "error: {:?}", snap.error);
    assert_eq!(snap.branch, Branch::Named("main".to_string()));
    assert!(!snap.is_dirty());
    assert!(snap.upstream.is_none());
    let commit = snap.last_commit.expect("has a commit");
    assert_eq!(commit.author, "Dev");
    assert_eq!(commit.summary, "init: first commit");
    assert_eq!(commit.timestamp, COMMIT_TS);
    assert!(!commit.short_id.is_empty());
}

#[test]
fn activity_sparkline_buckets_commits_by_week() {
    let tmp = TempDir::new().unwrap();
    init_repo_with_commit(tmp.path(), "a.txt", "x", "init");
    // `now == COMMIT_TS`, so the single fixture commit lands in the current week
    // (the last bucket); the 11 older weeks are empty.
    let snap = snapshot_repo(&repo_ref(tmp.path()), COMMIT_TS);
    assert_eq!(snap.activity.len(), 12, "12 weekly buckets");
    assert_eq!(*snap.activity.last().unwrap(), 1, "1 commit this week");
    assert_eq!(
        snap.activity[..11].iter().map(|&c| c as u32).sum::<u32>(),
        0,
        "no commits in the older weeks"
    );
}

#[test]
fn dirty_repo_counts_staged_modified_untracked() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo_with_commit(dir, "tracked.txt", "v1", "init");

    write(dir, "tracked.txt", "v2"); // modified (unstaged)
    write(dir, "staged.txt", "new");
    git(dir, &["add", "staged.txt"]); // staged
    write(dir, "untracked.txt", "u"); // untracked

    let snap = snapshot_repo(&repo_ref(dir), COMMIT_TS);

    assert!(snap.is_dirty());
    assert!(
        snap.worktree.staged >= 1,
        "expected staged: {:?}",
        snap.worktree
    );
    assert!(
        snap.worktree.modified >= 1,
        "expected modified: {:?}",
        snap.worktree
    );
    assert!(
        snap.worktree.untracked >= 1,
        "expected untracked: {:?}",
        snap.worktree
    );
    assert!(snap.needs_attention());
}

#[test]
fn unborn_repo_has_no_commits() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());

    let snap = snapshot_repo(&repo_ref(tmp.path()), COMMIT_TS);

    assert!(snap.error.is_none(), "error: {:?}", snap.error);
    assert_eq!(snap.branch, Branch::Unborn);
    assert!(snap.last_commit.is_none());
    assert!(!snap.is_dirty());
}

#[test]
fn detached_head_reports_short_sha() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo_with_commit(dir, "a.txt", "1", "c1");
    let full_sha = git_output(dir, &["rev-parse", "HEAD"]);
    git(dir, &["checkout", "-q", "--detach", "HEAD"]);

    let snap = snapshot_repo(&repo_ref(dir), COMMIT_TS);

    match snap.branch {
        Branch::Detached(short) => {
            assert!(short.len() >= 4, "short sha too short: {short}");
            assert!(
                full_sha.starts_with(&short),
                "{full_sha} !startswith {short}"
            );
        }
        other => panic!("expected detached HEAD, got {other:?}"),
    }
}

#[test]
fn stash_is_counted_and_worktree_clean_after() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo_with_commit(dir, "f.txt", "v1", "init");
    write(dir, "f.txt", "v2");
    git(dir, &["stash"]);

    let snap = snapshot_repo(&repo_ref(dir), COMMIT_TS);

    assert_eq!(snap.stash_count, 1);
    assert!(!snap.is_dirty(), "worktree should be clean after stash");
    assert!(snap.needs_attention(), "a stash should flag attention");
}

#[test]
fn ahead_of_upstream_counts_commits() {
    let tmp = TempDir::new().unwrap();
    let remote = tmp.path().join("origin.git");
    std::fs::create_dir_all(&remote).unwrap();
    git(
        &remote,
        &["-c", "init.defaultBranch=main", "init", "-q", "--bare"],
    );

    let work = tmp.path().join("work");
    init_repo_with_commit(&work, "a.txt", "1", "c1");
    git(
        &work,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&work, &["push", "-q", "-u", "origin", "main"]);

    // One local commit not yet pushed → ahead 1.
    write(&work, "b.txt", "2");
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "c2"]);

    let snap = snapshot_repo(&repo_ref(&work), COMMIT_TS);

    let upstream = snap.upstream.expect("has an upstream");
    assert!(
        upstream.name.contains("origin/main"),
        "upstream name: {}",
        upstream.name
    );
    assert_eq!(upstream.ahead, 1);
    assert_eq!(upstream.behind, 0);
}

#[test]
fn unreadable_repo_becomes_error_snapshot_not_panic() {
    // A path that exists but isn't a git repo → gix open fails → error snapshot.
    let tmp = TempDir::new().unwrap();
    let snap = snapshot_repo(&repo_ref(tmp.path()), COMMIT_TS);
    assert!(snap.has_error());
}

// ----- discovery tests ------------------------------------------------------

#[test]
fn discovers_multiple_repos() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_repo_with_commit(&root.join("repo-a"), "f", "1", "c");
    init_repo_with_commit(&root.join("repo-b"), "f", "1", "c");
    std::fs::create_dir_all(root.join("not-a-repo")).unwrap();

    let refs = discover(&opts_for(root)).unwrap();

    assert_eq!(refs.len(), 2, "found: {refs:?}");
    assert!(refs.iter().all(|r| r.path.is_some()));
}

#[test]
fn respects_ignore_globs() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_repo_with_commit(&root.join("real"), "f", "1", "c");
    init_repo_with_commit(&root.join("node_modules").join("pkg"), "f", "1", "c");

    let mut opts = opts_for(root);
    opts.ignore = vec!["**/node_modules/**".to_string()];
    let refs = discover(&opts).unwrap();

    assert_eq!(refs.len(), 1, "found: {refs:?}");
    assert!(refs[0].path.as_ref().unwrap().as_str().ends_with("real"));
}

#[test]
fn stop_at_repo_controls_nested_discovery() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_repo_with_commit(&root.join("outer"), "f", "1", "c");
    init_repo_with_commit(&root.join("outer").join("inner"), "f", "1", "c");

    let mut opts = opts_for(root);
    opts.stop_at_repo = true;
    assert_eq!(
        discover(&opts).unwrap().len(),
        1,
        "stop_at_repo should prune nested"
    );

    opts.stop_at_repo = false;
    assert_eq!(
        discover(&opts).unwrap().len(),
        2,
        "without stop_at_repo, nested repo is found"
    );
}

#[test]
fn max_depth_limits_descent() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    // root/a/b/c/deep is at depth 4 from the root.
    let deep = root.join("a").join("b").join("c").join("deep");
    init_repo_with_commit(&deep, "f", "1", "c");

    let mut opts = opts_for(root);
    opts.max_depth = 2;
    assert_eq!(
        discover(&opts).unwrap().len(),
        0,
        "too shallow to reach depth 4"
    );

    opts.max_depth = 6;
    assert_eq!(discover(&opts).unwrap().len(), 1, "deep enough to find it");
}
