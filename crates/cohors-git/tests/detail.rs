//! Fixture-based integration test for the per-repo detail reader.
//!
//! Builds a throwaway git repo with the real `git` CLI — a commit, a working-tree
//! change, and an extra branch — then asserts `repo_detail` reports the basics.
//! Requires `git` on `PATH`, which any environment running cohors's tests has.

use std::path::Path;
use std::process::Command;

use camino::Utf8Path;
use cohors_git::repo_detail;
use tempfile::TempDir;

/// A `git` command with no global/system config, for reproducible fixtures.
fn git_cmd(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir)
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

fn commit_file(dir: &Path, file: &str, content: &str, message: &str) {
    std::fs::write(dir.join(file), content).unwrap();
    let out = git_cmd(dir)
        .env("GIT_AUTHOR_NAME", "Author")
        .env("GIT_AUTHOR_EMAIL", "me@example.com")
        .env("GIT_COMMITTER_NAME", "Author")
        .env("GIT_COMMITTER_EMAIL", "me@example.com")
        .args(["add", file])
        .output()
        .expect("failed to run git add");
    assert!(out.status.success());
    let out = git_cmd(dir)
        .env("GIT_AUTHOR_NAME", "Author")
        .env("GIT_AUTHOR_EMAIL", "me@example.com")
        .env("GIT_COMMITTER_NAME", "Author")
        .env("GIT_COMMITTER_EMAIL", "me@example.com")
        .args(["commit", "-q", "-m", message])
        .output()
        .expect("failed to run git commit");
    assert!(
        out.status.success(),
        "git commit failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn reports_branch_commits_and_changes() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    git(dir, &["-c", "init.defaultBranch=main", "init", "-q"]);

    // A commit, an extra branch, and an unstaged working-tree change.
    commit_file(dir, "a.txt", "1", "feat: first");
    git(dir, &["branch", "feature"]);
    std::fs::write(dir.join("a.txt"), "modified").unwrap();

    let path = Utf8Path::from_path(dir).expect("temp path is valid utf-8");
    let detail = repo_detail(path);

    assert_eq!(detail.current_branch.as_deref(), Some("main"));
    assert!(!detail.recent_commits.is_empty(), "got: {detail:?}");
    // The current branch is hoisted to the front of the branch list.
    assert_eq!(detail.branches.first().map(String::as_str), Some("main"));
    assert!(detail.branches.iter().any(|b| b == "feature"));
    // The modified file shows up as a worktree change.
    assert!(
        detail.changed_files.iter().any(|f| f.path == "a.txt"),
        "got: {:?}",
        detail.changed_files
    );
}
