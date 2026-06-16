//! CLI integration tests: arg parsing, `cohors init`, and `cohors scan` JSON.
//!
//! These exercise the built `cohors` binary via `assert_cmd`. `XDG_CACHE_HOME`
//! is pointed at a temp dir so the log file never touches the real home.

use std::path::Path;
use std::process::Command as StdCommand;

use assert_cmd::Command;
use tempfile::TempDir;

/// A config path that doesn't exist, so the binary falls back to defaults
/// instead of reading the developer's real `~/.config/cohors/config.toml`.
const MISSING_CONFIG: &str = "/nonexistent/cohors-test-config.toml";

/// The `cohors` binary, with logging pointed at an isolated cache dir.
fn cohors(cache: &Path) -> Command {
    let mut cmd = Command::cargo_bin("cohors").expect("binary built");
    cmd.env("XDG_CACHE_HOME", cache);
    cmd
}

fn git(dir: &Path, args: &[&str]) {
    let out = StdCommand::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Dev")
        .env("GIT_AUTHOR_EMAIL", "dev@example.com")
        .env("GIT_COMMITTER_NAME", "Dev")
        .env("GIT_COMMITTER_EMAIL", "dev@example.com")
        .env("GIT_AUTHOR_DATE", "@1622548800 +0000")
        .env("GIT_COMMITTER_DATE", "@1622548800 +0000")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("failed to run git");
    assert!(
        out.status.success(),
        "git {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo_with_commit(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["-c", "init.defaultBranch=main", "init", "-q"]);
    std::fs::write(dir.join("README.md"), "hi").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "init"]);
}

#[test]
fn version_flag_succeeds() {
    let cache = TempDir::new().unwrap();
    cohors(cache.path()).arg("--version").assert().success();
}

#[test]
fn help_flag_succeeds() {
    let cache = TempDir::new().unwrap();
    cohors(cache.path()).arg("--help").assert().success();
}

#[test]
fn scan_outputs_json_array_with_repo() {
    let cache = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    init_repo_with_commit(&root.path().join("myrepo"));

    let assert = cohors(cache.path())
        .args(["scan", "--config", MISSING_CONFIG, "--root"])
        .arg(root.path())
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("scan emits valid JSON");
    let array = json.as_array().expect("top-level array");
    assert_eq!(array.len(), 1, "json: {stdout}");
    assert_eq!(array[0]["name"], "myrepo");
    assert_eq!(array[0]["branch"]["kind"], "named");
    assert_eq!(array[0]["branch"]["value"], "main");
    assert!(array[0]["error"].is_null());
}

#[test]
fn scan_empty_root_is_empty_json_array() {
    let cache = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();

    let assert = cohors(cache.path())
        .args(["scan", "--config", MISSING_CONFIG, "--root"])
        .arg(root.path())
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[test]
fn action_requires_a_selector() {
    // Bulk actions never silently hit the whole fleet — `--select` is required.
    let cache = TempDir::new().unwrap();
    cohors(cache.path())
        .args(["fetch", "--config", MISSING_CONFIG])
        .assert()
        .failure();
}

#[test]
fn fetch_dry_run_lists_targets_without_acting() {
    let cache = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    init_repo_with_commit(&root.path().join("myrepo"));

    let assert = cohors(cache.path())
        .args([
            "fetch",
            "--select",
            "all",
            "--dry-run",
            "--config",
            MISSING_CONFIG,
            "--root",
        ])
        .arg(root.path())
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Would fetch"), "stdout: {stdout}");
    assert!(stdout.contains("myrepo"), "stdout: {stdout}");
}

#[test]
fn commit_creates_a_commit_across_selected_repos() {
    let cache = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    let repo = root.path().join("myrepo");
    init_repo_with_commit(&repo);
    // An uncommitted change for the bulk commit to capture.
    std::fs::write(repo.join("new.txt"), "wip").unwrap();

    let assert = cohors(cache.path())
        .args([
            "commit",
            "--select",
            "all",
            "--message",
            "snapshot wip",
            "--config",
            MISSING_CONFIG,
            "--root",
        ])
        .arg(root.path())
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("committed myrepo"), "stdout: {stdout}");

    // The change is now committed: the worktree is clean.
    let status = StdCommand::new("git")
        .current_dir(&repo)
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&status.stdout).trim().is_empty(),
        "worktree should be clean after commit"
    );
}

#[test]
fn init_writes_config_then_respects_force() {
    let cache = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("config.toml");

    // First write succeeds and creates the file.
    cohors(cache.path())
        .args(["init", "--config"])
        .arg(&cfg)
        .assert()
        .success();
    assert!(cfg.exists());

    // Second write without --force is refused.
    cohors(cache.path())
        .args(["init", "--config"])
        .arg(&cfg)
        .assert()
        .failure();

    // With --force it overwrites.
    cohors(cache.path())
        .args(["init", "--config"])
        .arg(&cfg)
        .arg("--force")
        .assert()
        .success();
}
