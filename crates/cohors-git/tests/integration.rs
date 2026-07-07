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
    assert!(snap.is_dirty(), "worktree changes flag the repo");
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
    assert!(snap.stash_count > 0, "the stash is counted");
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

// ----- changes / diff -------------------------------------------------------

#[test]
fn repo_changes_lists_files_and_caps_patch() {
    let tmp = TempDir::new().unwrap();
    init_repo_with_commit(tmp.path(), "a.txt", "one\ntwo\nthree\n", "init");
    // Modify a tracked file and add an untracked one — both should surface.
    write(tmp.path(), "a.txt", "one\nTWO\nthree\n");
    write(tmp.path(), "new.txt", "brand new file\n");

    let path = camino::Utf8Path::from_path(tmp.path()).unwrap();

    // No patch requested → just the changed-file list.
    let ch = cohors_git::repo_changes(path, false, 20_000);
    assert_eq!(ch.files.len(), 2, "modified a.txt + untracked new.txt");
    assert!(ch.patch.is_none());
    assert!(
        ch.files
            .iter()
            .any(|f| f.path == "new.txt" && f.status == "??"),
        "untracked file is `??`"
    );
    assert!(ch.files.iter().any(|f| f.path == "a.txt"));

    // Patch requested with a generous cap → real diff, not truncated.
    let ch = cohors_git::repo_changes(path, true, 20_000);
    let patch = ch.patch.expect("patch was requested");
    assert!(patch.contains("TWO"), "diff shows the modified line");
    assert!(
        patch.contains("brand new file"),
        "diff shows the untracked file"
    );
    assert!(!ch.truncated);

    // Tiny cap → the patch is truncated and flagged.
    let ch = cohors_git::repo_changes(path, true, 10);
    assert!(ch.truncated, "a 10-byte cap must truncate");
    assert!(ch.patch.unwrap().len() <= 10);
}

/// The history memo must never serve stale data: a new commit (HEAD moved) has
/// to invalidate it, so ahead-of-upstream and the activity sparkline update on
/// the very next scan. And while nothing moves, repeated snapshots agree.
#[test]
fn history_memo_invalidates_on_new_commit() {
    let tmp = TempDir::new().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let bare = root.join("origin.git");
    let work = root.join("work");
    git(tmp.path(), &["init", "-q", "--bare", bare.as_str()]);
    git(tmp.path(), &["init", "-q", "-b", "main", work.as_str()]);
    let w = work.as_std_path();
    git(w, &["config", "user.email", "t@example.com"]);
    git(w, &["config", "user.name", "Tester"]);
    std::fs::write(work.join("f.txt"), "1").unwrap();
    git(w, &["add", "-A"]);
    git(w, &["commit", "-q", "-m", "one"]);
    git(w, &["remote", "add", "origin", bare.as_str()]);
    git(w, &["push", "-q", "-u", "origin", "main"]);

    let repo_ref = RepoRef {
        id: RepoId(work.as_str().to_string()),
        path: Some(work.clone()),
    };
    let now = 1_700_000_000;

    // Two scans with nothing moved: the memoized fields agree exactly.
    let first = snapshot_repo(&repo_ref, now);
    let second = snapshot_repo(&repo_ref, now);
    assert_eq!(first.upstream, second.upstream);
    assert_eq!(first.activity, second.activity);
    assert_eq!(first.ahead(), 0);

    // A new commit moves HEAD → the memo must roll over, not serve ahead=0.
    std::fs::write(work.join("f.txt"), "2").unwrap();
    git(w, &["add", "-A"]);
    git(w, &["commit", "-q", "-m", "two"]);
    let third = snapshot_repo(&repo_ref, now);
    assert_eq!(third.ahead(), 1, "stale memo served after HEAD moved");
}

#[test]
fn discovery_prunes_secret_directories_but_honors_explicit_roots() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    init_repo_with_commit(&root.join("normal"), "f", "1", "c");
    // A repo hiding inside a secret store must not be discovered from above…
    init_repo_with_commit(&root.join(".ssh").join("oops"), "f", "1", "c");

    let refs = discover(&opts_for(root)).unwrap();
    assert_eq!(refs.len(), 1, "found: {refs:?}");
    assert!(refs[0].path.as_ref().unwrap().as_str().ends_with("normal"));

    // …but a sensitive directory the user configured *as a root* is their call.
    let refs = discover(&opts_for(&root.join(".ssh"))).unwrap();
    assert_eq!(refs.len(), 1, "explicit root: {refs:?}");
    assert!(refs[0].path.as_ref().unwrap().as_str().ends_with("oops"));
}

#[test]
fn remote_url_falls_back_beyond_origin() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo_with_commit(dir, "f", "1", "c");
    git(
        dir,
        &[
            "remote",
            "add",
            "upstream",
            "https://github.com/o/upstream-only.git",
        ],
    );

    let snap = snapshot_repo(&repo_ref(dir), 0);
    assert_eq!(
        snap.remote_url.as_deref(),
        Some("https://github.com/o/upstream-only.git"),
        "a repo without `origin` still resolves its remote"
    );

    // With `origin` present it wins over other names.
    git(
        dir,
        &["remote", "add", "origin", "https://github.com/o/origin.git"],
    );
    let snap = snapshot_repo(&repo_ref(dir), 0);
    assert_eq!(
        snap.remote_url.as_deref(),
        Some("https://github.com/o/origin.git")
    );
}

/// A repo with several local branches: the snapshot lists all of them,
/// current-first and flagged, each carrying its own tip commit.
#[test]
fn snapshot_lists_all_local_branches() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo_with_commit(dir, "a.txt", "1", "init on main");

    // A feature branch with its own commit, then back to main.
    git(dir, &["checkout", "-q", "-b", "feat/x"]);
    write(dir, "b.txt", "2");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "feat commit"]);
    git(dir, &["checkout", "-q", "main"]);

    let snap = snapshot_repo(&repo_ref(dir), COMMIT_TS);

    let names: Vec<&str> = snap.branches.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"main"), "branches: {names:?}");
    assert!(names.contains(&"feat/x"), "branches: {names:?}");

    // The checked-out branch is hoisted first and flagged current.
    let first = &snap.branches[0];
    assert_eq!(first.name, "main");
    assert!(first.is_current);

    // Each branch carries its OWN tip commit, not HEAD's.
    let feat = snap.branches.iter().find(|b| b.name == "feat/x").unwrap();
    assert!(!feat.is_current);
    assert_eq!(
        feat.last_commit.as_ref().map(|c| c.summary.as_str()),
        Some("feat commit")
    );
    let main = snap.branches.iter().find(|b| b.name == "main").unwrap();
    assert_eq!(
        main.last_commit.as_ref().map(|c| c.summary.as_str()),
        Some("init on main")
    );
    // No remote → no per-branch upstream.
    assert!(!main.has_upstream);
}
