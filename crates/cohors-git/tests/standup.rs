//! Fixture-based integration tests for standup commit collection.
//!
//! Each test builds a throwaway git repo with the real `git` CLI, pinning the
//! author/committer identity and date per commit so timestamps and author
//! filtering are deterministic. Requires `git` on `PATH` — which any environment
//! running cohors's tests will have.

use std::path::Path;
use std::process::Command;

use camino::Utf8Path;
use cohors_git::collect_commits;
use tempfile::TempDir;

/// A few fixed instants (`@<unix-ts> <tz>` is git's unambiguous internal date
/// format) so the window boundaries in the tests are exact.
const T_BASE: i64 = 1_622_548_800; // 2021-06-01T12:00:00Z
const HOUR: i64 = 3_600;

/// A `git` command with no global/system config, for reproducible fixtures.
/// Identity and dates are supplied per-invocation so each commit can differ.
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

/// `git init` (default branch `main`) at `dir`, creating it if needed.
fn init_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["-c", "init.defaultBranch=main", "init", "-q"]);
}

/// Create a file and commit it as `(name, email)` at unix time `ts`. Author and
/// committer share the identity/date so committer time (what the collector
/// reads) equals `ts`.
fn commit_as(dir: &Path, file: &str, content: &str, message: &str, email: &str, ts: i64) {
    std::fs::write(dir.join(file), content).unwrap();
    let date = format!("@{ts} +0000");
    let out = git_cmd(dir)
        .env("GIT_AUTHOR_NAME", "Author")
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_COMMITTER_NAME", "Author")
        .env("GIT_COMMITTER_EMAIL", email)
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .args(["commit", "-q", "-m", message, "--", file])
        .output()
        .expect("failed to run git commit");
    // `commit -- <pathspec>` stages and commits the named file in one step, but
    // only if it's already tracked/added. New files need an explicit add first,
    // so fall back to add + commit when the direct path didn't take.
    if !out.status.success() {
        git(dir, &["add", file]);
        let out = git_cmd(dir)
            .env("GIT_AUTHOR_NAME", "Author")
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_NAME", "Author")
            .env("GIT_COMMITTER_EMAIL", email)
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .args(["commit", "-q", "-m", message])
            .output()
            .expect("failed to run git commit");
        assert!(
            out.status.success(),
            "git commit failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

fn full_head_sha(dir: &Path) -> String {
    let out = git_cmd(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("failed to run git rev-parse");
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn utf8(dir: &Path) -> &Utf8Path {
    Utf8Path::from_path(dir).expect("temp path is valid utf-8")
}

const MINE: &str = "me@example.com";
const THEIRS: &str = "other@example.com";

#[test]
fn collects_my_commits_in_window_and_excludes_other_authors() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);

    // Two of mine inside the window, one by a different author inside the window.
    commit_as(dir, "a.txt", "1", "feat: mine first", MINE, T_BASE);
    commit_as(dir, "b.txt", "2", "fix: theirs", THEIRS, T_BASE + HOUR);
    commit_as(
        dir,
        "c.txt",
        "3",
        "docs: mine second",
        MINE,
        T_BASE + 2 * HOUR,
    );

    let since = T_BASE - HOUR;
    let until = T_BASE + 3 * HOUR;
    let commits = collect_commits(utf8(dir), MINE, since, until);

    assert_eq!(commits.len(), 2, "got: {commits:?}");

    // All tagged with the repo's directory name.
    let repo_name = dir.file_name().unwrap().to_str().unwrap();
    assert!(commits.iter().all(|c| c.repo == repo_name));

    // None from the other author.
    let summaries: Vec<&str> = commits.iter().map(|c| c.summary.as_str()).collect();
    assert!(summaries.contains(&"feat: mine first"));
    assert!(summaries.contains(&"docs: mine second"));
    assert!(!summaries.iter().any(|s| s.contains("theirs")));

    // Timestamps are committer-time seconds, and short_ids are real prefixes.
    let second = commits
        .iter()
        .find(|c| c.summary == "docs: mine second")
        .unwrap();
    assert_eq!(second.timestamp, T_BASE + 2 * HOUR);
    assert!(!second.short_id.is_empty());
    let head = full_head_sha(dir);
    assert!(
        head.starts_with(&second.short_id),
        "{head} !startswith {}",
        second.short_id
    );
}

#[test]
fn author_match_is_case_insensitive() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_as(dir, "a.txt", "1", "mine", MINE, T_BASE);

    // Query with a differently-cased email; it should still match.
    let commits = collect_commits(utf8(dir), "ME@EXAMPLE.COM", T_BASE - HOUR, T_BASE + HOUR);
    assert_eq!(commits.len(), 1, "got: {commits:?}");
    assert_eq!(commits[0].summary, "mine");
}

#[test]
fn excludes_commits_outside_the_window() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);

    // Before the window, exactly at `since` (inclusive), exactly at `until`
    // (exclusive), and after the window — all by me.
    commit_as(dir, "before.txt", "1", "before", MINE, T_BASE - HOUR);
    commit_as(dir, "at_since.txt", "2", "at_since", MINE, T_BASE);
    commit_as(
        dir,
        "at_until.txt",
        "3",
        "at_until",
        MINE,
        T_BASE + 2 * HOUR,
    );
    commit_as(dir, "after.txt", "4", "after", MINE, T_BASE + 3 * HOUR);

    let since = T_BASE; // inclusive
    let until = T_BASE + 2 * HOUR; // exclusive
    let commits = collect_commits(utf8(dir), MINE, since, until);

    let summaries: Vec<&str> = commits.iter().map(|c| c.summary.as_str()).collect();
    assert_eq!(summaries, vec!["at_since"], "got: {commits:?}");
}

#[test]
fn unreadable_repo_yields_empty_vec() {
    // A path that exists but isn't a git repo → open fails → empty, no panic.
    let tmp = TempDir::new().unwrap();
    let commits = collect_commits(utf8(tmp.path()), MINE, 0, i64::MAX);
    assert!(commits.is_empty());
}

#[test]
fn nonexistent_path_yields_empty_vec() {
    let commits = collect_commits(
        Utf8Path::new("/definitely/not/a/real/repo/path"),
        MINE,
        0,
        i64::MAX,
    );
    assert!(commits.is_empty());
}

#[test]
fn empty_unborn_repo_yields_empty_vec() {
    // A freshly-initialised repo with no commits has no HEAD → empty, no panic.
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let commits = collect_commits(utf8(tmp.path()), MINE, 0, i64::MAX);
    assert!(commits.is_empty());
}
