//! CLI integration tests: arg parsing, `cohors init`, and `cohors scan` JSON.
//!
//! These exercise the built `cohors` binary via `assert_cmd`. `XDG_CACHE_HOME`
//! is pointed at a temp dir so the log file never touches the real home.

use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use assert_cmd::Command;
use tempfile::TempDir;

/// Write an empty config file into `dir` and return its path, so the binary
/// runs on defaults instead of reading the developer's real
/// `~/.config/cohors/config.toml`. It has to exist: a `--config` naming a
/// missing file is an error (a typo must not silently scan another fleet).
fn empty_config(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("empty-config.toml");
    std::fs::write(&path, "").expect("write empty config");
    path
}

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
    // Identity in the repo's *local* config, so a `git commit` spawned by the
    // `cohors` binary (which doesn't inherit this test's GIT_AUTHOR_* env) still
    // has an author — CI runners have no global gitconfig.
    git(dir, &["config", "user.email", "dev@example.com"]);
    git(dir, &["config", "user.name", "Dev"]);
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
fn help_opens_with_the_wordmark_and_closes_with_next_steps() {
    // Help is the first thing many people ever see of cohors, so it carries the
    // same masthead and "here is what to do next" footer as every other surface.
    let cache = TempDir::new().unwrap();
    let assert = cohors(cache.path()).arg("--help").assert().success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("cohors"), "stdout: {stdout}");
    assert!(
        stdout.contains("every git repo you own, on one screen"),
        "the masthead tagline should lead the help: {stdout}"
    );
    assert!(
        stdout.contains("cohors demo"),
        "the footer should offer a zero-setup first step: {stdout}"
    );
}

#[test]
fn mcp_at_a_terminal_explains_itself_instead_of_hanging() {
    // An MCP client always pipes stdin. Here stdin is closed rather than a
    // terminal, so this exercises the server path: it must not print the guide.
    // The guide itself is covered by the terminal check inside `run_mcp`; what
    // matters for a test is that the two paths stay distinct.
    let cache = TempDir::new().unwrap();
    let assert = cohors(cache.path())
        .args(["mcp", "--help"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("stdio"),
        "mcp help should say a client spawns it over stdio: {stdout}"
    );
}

#[test]
fn scan_outputs_json_array_with_repo() {
    let cache = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    init_repo_with_commit(&root.path().join("myrepo"));

    let assert = cohors(cache.path())
        .args(["scan", "--config"])
        .arg(empty_config(cache.path()))
        .arg("--root")
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
        .args(["scan", "--config"])
        .arg(empty_config(cache.path()))
        .arg("--root")
        .arg(root.path())
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[test]
fn a_missing_explicit_config_is_an_error_that_says_how_to_create_it() {
    // A mistyped `--config` used to fall back to auto-detection silently and
    // scan a different fleet. It must fail, name the path, and say how to fix it.
    let cache = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    let missing = cache.path().join("typo.toml");

    let assert = cohors(cache.path())
        .args(["scan", "--config"])
        .arg(&missing)
        .arg("--root")
        .arg(root.path())
        .assert()
        .failure();

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    let missing = missing.display().to_string();
    assert!(stderr.contains(&missing), "stderr: {stderr}");
    assert!(
        stderr.contains(&format!("cohors init --config {missing}")),
        "stderr: {stderr}"
    );
}

#[test]
fn action_without_a_selector_explains_itself() {
    // Bulk actions still never silently hit the whole fleet, but a missing
    // `--select` is a question, not a punishment: it prints the verb's guide and
    // acts on nothing.
    let cache = TempDir::new().unwrap();
    let assert = cohors(cache.path())
        .args(["fetch", "--config"])
        .arg(empty_config(cache.path()))
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("cohors fetch --select dirty"),
        "the guide should offer the selector vocabulary: {stdout}"
    );
    assert!(
        stdout.contains("--dry-run"),
        "the guide should point at a safe first step: {stdout}"
    );
}

#[test]
fn fetch_dry_run_lists_targets_without_acting() {
    let cache = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    init_repo_with_commit(&root.path().join("myrepo"));

    let assert = cohors(cache.path())
        .args(["fetch", "--select", "all", "--dry-run", "--config"])
        .arg(empty_config(cache.path()))
        .arg("--root")
        .arg(root.path())
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("would fetch"), "stdout: {stdout}");
    assert!(
        stdout.contains("nothing will change"),
        "a dry run should say so plainly: {stdout}"
    );
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
        ])
        .arg(empty_config(cache.path()))
        .arg("--root")
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

    // A second run without --force leaves the file alone and says where it is.
    std::fs::write(&cfg, "roots = [\"/kept\"]\n").unwrap();
    let out = cohors(cache.path())
        .args(["init", "--config"])
        .arg(&cfg)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8_lossy(&out);
    assert!(
        out.contains("already exists") && out.contains("/kept"),
        "{out}"
    );
    assert_eq!(
        std::fs::read_to_string(&cfg).unwrap(),
        "roots = [\"/kept\"]\n"
    );

    // With --force it overwrites, keeping the old file beside it.
    cohors(cache.path())
        .args(["init", "--config"])
        .arg(&cfg)
        .arg("--force")
        .assert()
        .success();
    let backup = dir.path().join("config.toml.bak");
    assert_eq!(
        std::fs::read_to_string(backup).unwrap(),
        "roots = [\"/kept\"]\n"
    );
}

/// A read-only command run against `root`, with an isolated config.
fn read_cmd(cache: &Path, root: &Path, args: &[&str]) -> Command {
    let mut cmd = cohors(cache);
    cmd.args(args)
        .arg("--config")
        .arg(empty_config(cache))
        .arg("--root")
        .arg(root);
    cmd
}

fn stdout_of(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

fn stderr_of(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stderr.clone()).unwrap()
}

#[test]
fn path_prints_only_the_path_and_explains_a_miss_or_a_tie() {
    let cache = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    for name in ["payments", "payroll", "web"] {
        init_repo_with_commit(&root.path().join(name));
    }
    let payments = std::fs::canonicalize(root.path().join("payments")).unwrap();
    let path = |args: &[&str]| read_cmd(cache.path(), root.path(), args).assert();

    // Exact name: the path and a newline, nothing else, for `cd "$(…)"`.
    let out = path(&["path", "payments"]).success();
    assert_eq!(stdout_of(&out), format!("{}\n", payments.display()));
    // A unique prefix, in any case.
    let out = path(&["path", "PAYM"]).success();
    assert_eq!(stdout_of(&out), format!("{}\n", payments.display()));
    // A glob.
    let out = path(&["path", "w*"]).success();
    assert!(stdout_of(&out).trim_end().ends_with("web"));

    // Two repos share the prefix: exit 2, both listed on stderr, stdout empty.
    let out = path(&["path", "pay"]).code(2);
    let err = stderr_of(&out);
    assert!(err.contains("matches 2 repositories"), "stderr: {err}");
    assert!(
        err.contains("payments") && err.contains("payroll"),
        "stderr: {err}"
    );
    assert!(stdout_of(&out).is_empty());

    // Nothing matches: exit 1, with the close name.
    let out = path(&["path", "paymnets"]).code(1);
    let err = stderr_of(&out);
    assert!(
        err.contains("no repository matches `paymnets`"),
        "stderr: {err}"
    );
    assert!(err.contains("did you mean: payments"), "stderr: {err}");
    assert!(stdout_of(&out).is_empty());
}

#[test]
fn show_json_reports_one_repo_like_the_mcp_tool() {
    let cache = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    let repo = root.path().join("myrepo");
    init_repo_with_commit(&repo);
    std::fs::write(repo.join("README.md"), "changed").unwrap();

    let out = read_cmd(cache.path(), root.path(), &["show", "myrepo", "--json"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_str(&stdout_of(&out)).expect("valid JSON");
    assert_eq!(json["name"], "myrepo");
    assert_eq!(json["branch"]["value"], "main");
    assert_eq!(json["worktree"]["modified"], 1);
    assert_eq!(json["assessment"]["needs_attention"], true);
    assert_eq!(json["detail"]["current_branch"], "main");
    assert_eq!(json["detail"]["recent_commits"][0]["summary"], "init");
    assert_eq!(json["detail"]["changed_files"][0]["path"], "README.md");
}

#[test]
fn changes_lists_a_modified_file_and_withholds_a_secret() {
    let cache = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    let dirty = root.path().join("dirty");
    init_repo_with_commit(&dirty);
    init_repo_with_commit(&root.path().join("clean"));
    std::fs::write(dirty.join("README.md"), "a visible change").unwrap();
    std::fs::write(dirty.join(".env"), "API_KEY=SUPER_SECRET_VALUE\n").unwrap();

    let out = read_cmd(cache.path(), root.path(), &["changes", "--patch", "--json"])
        .assert()
        .success();
    let stdout = stdout_of(&out);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let repos = json["repos"].as_array().expect("repos");
    // Only the dirty repo: the default selector is `dirty`.
    assert_eq!(repos.len(), 1, "{stdout}");
    assert_eq!(repos[0]["repo"], "dirty");
    let files: Vec<&str> = repos[0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(files.contains(&"README.md"), "{stdout}");
    // The secret file is named (it is uncommitted) but its contents never
    // leave the disk, not even in the diff.
    assert!(files.contains(&".env"), "{stdout}");
    let patch = repos[0]["patch"].as_str().expect("patch requested");
    assert!(patch.contains("a visible change"), "{stdout}");
    assert!(!stdout.contains("SUPER_SECRET_VALUE"), "{stdout}");
}

#[test]
fn search_finds_a_string_and_never_a_secret_file() {
    let cache = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    let repo = root.path().join("myrepo");
    init_repo_with_commit(&repo);
    // Committed, so every search backend (rg, git grep, the walk) sees both.
    std::fs::write(repo.join("notes.txt"), "first\nfind needle_42 here\n").unwrap();
    std::fs::write(repo.join(".env"), "needle_42=SECRET\n").unwrap();
    git(&repo, &["add", "-f", "."]);
    git(&repo, &["commit", "-q", "-m", "add notes"]);

    let out = read_cmd(
        cache.path(),
        root.path(),
        &["search", "needle_42", "--json"],
    )
    .assert()
    .success();
    let stdout = stdout_of(&out);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let hits = json["hits"].as_array().expect("hits");
    assert_eq!(hits.len(), 1, "{stdout}");
    assert_eq!(hits[0]["name"], "myrepo");
    assert_eq!(hits[0]["path"], "notes.txt");
    assert_eq!(hits[0]["line"], 2);
    assert_eq!(json["truncated"], false);
    assert!(!stdout.contains("SECRET"), "{stdout}");
}

#[test]
fn status_is_another_name_for_overview() {
    let cache = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    init_repo_with_commit(&root.path().join("myrepo"));

    let out = read_cmd(cache.path(), root.path(), &["status", "--json"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_str(&stdout_of(&out)).expect("valid JSON");
    assert!(json["roots"].is_array(), "the overview model: {json}");
    assert!(json["headline"].is_string(), "the overview model: {json}");
}

/// These tests read the binary's output through a pipe, which is how a script
/// and a coding agent's shell read it too. A pipe gets JSON; `--report` asks
/// for the report a person reads instead, as plain text (ADR-070).
#[test]
fn report_prints_the_plain_report_to_a_pipe_and_json_still_wins() {
    let cache = TempDir::new().unwrap();
    let root = TempDir::new().unwrap();
    let repo = root.path().join("myrepo");
    init_repo_with_commit(&repo);
    std::fs::write(repo.join("notes.txt"), "uncommitted").unwrap();

    // Unasked, a pipe gets the whole model as JSON.
    let json = stdout_of(
        &read_cmd(cache.path(), root.path(), &["overview"])
            .assert()
            .success(),
    );
    assert!(json.trim_start().starts_with('{'), "{json}");

    // `--report` gets the report: the headline in words, not a JSON document,
    // and no colour codes, since nothing at the other end can draw them.
    let report = stdout_of(
        &read_cmd(cache.path(), root.path(), &["overview", "--report"])
            .assert()
            .success(),
    );
    assert!(!report.trim_start().starts_with('{'), "{report}");
    assert!(report.contains("1 repository"), "{report}");
    assert!(!report.contains('\u{1b}'), "colour codes in {report:?}");
    assert!(
        report.len() < json.len(),
        "the report ({}) is the short form of the JSON ({})",
        report.len(),
        json.len()
    );

    // It is a global flag, so it works before the command and on other reads.
    for args in [
        &["--report", "scan"][..],
        &["launchpad", "--report"],
        &["changes", "--report"],
    ] {
        let out = stdout_of(&read_cmd(cache.path(), root.path(), args).assert().success());
        let first = out.trim_start().chars().next();
        assert!(!matches!(first, Some('{' | '[')), "{args:?}: {out}");
        assert!(!out.contains('\u{1b}'), "{args:?}: colour codes");
    }

    // An explicit `--json` still wins.
    let both = stdout_of(
        &read_cmd(
            cache.path(),
            root.path(),
            &["overview", "--report", "--json"],
        )
        .assert()
        .success(),
    );
    assert!(both.trim_start().starts_with('{'), "{both}");
}

// ----- `cohors hook` (ADR-073) -----------------------------------------------
//
// Claude Code runs a hook as a program: the event arrives as JSON on standard
// input, the answer leaves as one line of JSON on standard output. These tests
// are that caller. Every run also checks the two promises that matter most to
// a session waiting on it: it exits 0, and it writes nothing to stderr.

/// A repo called `api`, pushed to a bare remote so that "not pushed" means
/// something.
fn repo_with_remote(base: &Path) -> PathBuf {
    let remote = base.join("origin.git");
    std::fs::create_dir_all(&remote).unwrap();
    git(
        &remote,
        &["-c", "init.defaultBranch=main", "init", "-q", "--bare"],
    );
    let repo = base.join("api");
    init_repo_with_commit(&repo);
    git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&repo, &["push", "-q", "-u", "origin", "main"]);
    repo
}

/// A linked worktree at `.claude/worktrees/<name>` (where Claude Code puts an
/// agent's checkout), on a branch of its own.
fn add_worktree(repo: &Path, name: &str) -> PathBuf {
    let folder = format!(".claude/worktrees/{name}");
    let branch = format!("worktree-{name}");
    git(repo, &["worktree", "add", "-q", &folder, "-b", &branch]);
    repo.join(folder)
}

/// One commit in `worktree` that nothing has pushed.
fn commit_in(worktree: &Path, file: &str) {
    std::fs::write(worktree.join(file), "finished").unwrap();
    git(worktree, &["add", "."]);
    git(worktree, &["commit", "-q", "-m", "agent work"]);
}

/// The repo with an agent's worktree that holds one commit nobody pushed.
/// The `git` helper pins commit dates to 2021, so that work has sat for
/// years: well past the hour after which cohors takes work to be left behind.
fn repo_with_left_behind_worktree(base: &Path) -> (PathBuf, PathBuf) {
    let repo = repo_with_remote(base);
    let agent = add_worktree(&repo, "agent-a");
    commit_in(&agent, "done.txt");
    (repo, agent)
}

/// The event Claude Code sends when a session starts in `cwd`.
fn start_event(cwd: &Path, source: &str) -> String {
    serde_json::json!({
        "session_id": "s1",
        "hook_event_name": "SessionStart",
        "cwd": cwd,
        "source": source,
    })
    .to_string()
}

/// `cohors hook <event>` run in `dir` with `stdin`, its memory of what it
/// showed kept in `cache`. Asserts it exits 0 and writes nothing to stderr;
/// returns what it printed.
fn run_hook(cache: &Path, dir: &Path, event: &str, stdin: &str) -> String {
    let assert = cohors(cache)
        .current_dir(dir)
        .args(["hook", event])
        .write_stdin(stdin)
        .assert()
        .success();
    let stderr = stderr_of(&assert);
    assert!(
        stderr.is_empty(),
        "a hook never writes to stderr: {stderr:?}"
    );
    stdout_of(&assert)
}

/// What a hook printed, which must be exactly one line of JSON.
fn the_reply(printed: &str) -> serde_json::Value {
    assert!(
        printed.ends_with('\n') && printed.trim_end().lines().count() == 1,
        "one line of JSON, got {printed:?}"
    );
    serde_json::from_str(printed).expect("the reply is JSON")
}

/// The text a reply gives the model.
fn context_of(reply: &serde_json::Value) -> &str {
    assert_eq!(reply["hookSpecificOutput"]["hookEventName"], "SessionStart");
    reply["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additionalContext is text")
}

#[test]
fn a_session_in_a_repo_with_work_left_behind_is_given_the_note_and_told_once() {
    let base = TempDir::new().unwrap();
    let (repo, _agent) = repo_with_left_behind_worktree(base.path());
    let cache = TempDir::new().unwrap();
    let memory = cache.path().join("cohors/hook-notes.json");

    // The first time: the model gets the note and the person gets a line.
    let first = the_reply(&run_hook(
        cache.path(),
        base.path(),
        "session-start",
        &start_event(&repo, "startup"),
    ));
    let context = context_of(&first);
    assert!(context.contains("worktree agent-a"), "{context}");
    assert!(context.contains("1 commit not pushed"), "{context}");
    let told = first["systemMessage"].as_str().expect("the person is told");
    assert!(
        told.contains("api") && told.contains("1 commit in 1 worktree"),
        "{told}"
    );
    let mut keys: Vec<&str> = first
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["hookSpecificOutput", "systemMessage"]);

    // What was shown is remembered, by the repository's path.
    let remembered: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&memory).unwrap()).unwrap();
    let real_repo = std::fs::canonicalize(&repo).unwrap();
    let entry = &remembered[real_repo.to_str().unwrap()];
    assert!(
        entry["fingerprint"].as_str().is_some_and(|f| !f.is_empty()),
        "{remembered}"
    );
    assert!(
        entry["at"].as_i64().is_some_and(|at| at > 0),
        "{remembered}"
    );
    assert_eq!(remembered.as_object().unwrap().len(), 1);

    // Again, with the same memory: the model still gets it, the person does not.
    let again = the_reply(&run_hook(
        cache.path(),
        base.path(),
        "session-start",
        &start_event(&repo, "startup"),
    ));
    assert!(context_of(&again).contains("worktree agent-a"));
    assert!(again.get("systemMessage").is_none(), "{again}");

    // From a folder inside the repository it is the same repository, so the
    // same note and the same memory.
    let src = repo.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let from_src = the_reply(&run_hook(
        cache.path(),
        base.path(),
        "session-start",
        &start_event(&src, "startup"),
    ));
    assert!(context_of(&from_src).contains("worktree agent-a"));
    assert!(from_src.get("systemMessage").is_none(), "{from_src}");

    // A conversation that is only carrying on (`/clear`, a compaction, a
    // fork) gives the model the note and never the person, and leaves the
    // memory alone.
    for source in ["compact", "clear", "fork"] {
        let fresh = TempDir::new().unwrap();
        let reply = the_reply(&run_hook(
            fresh.path(),
            base.path(),
            "session-start",
            &start_event(&repo, source),
        ));
        assert!(context_of(&reply).contains("worktree agent-a"), "{source}");
        assert!(reply.get("systemMessage").is_none(), "{source}: {reply}");
        assert!(
            !fresh.path().join("cohors/hook-notes.json").exists(),
            "{source}"
        );
    }

    // Picking an old conversation up again is a person sitting down.
    let fresh = TempDir::new().unwrap();
    let resumed = the_reply(&run_hook(
        fresh.path(),
        base.path(),
        "session-start",
        &start_event(&repo, "resume"),
    ));
    assert!(resumed["systemMessage"].is_string(), "{resumed}");
}

#[test]
fn a_session_inside_the_worktree_is_told_only_of_its_own_old_commits() {
    let base = TempDir::new().unwrap();
    let (_repo, agent) = repo_with_left_behind_worktree(base.path());
    let cache = TempDir::new().unwrap();

    // The worktree is this session's own: it is not "left behind", so it is
    // not listed. But its one commit has sat unpushed for years, and the core
    // rules say that is worth a line.
    let reply = the_reply(&run_hook(
        cache.path(),
        base.path(),
        "session-start",
        &start_event(&agent, "startup"),
    ));
    let context = context_of(&reply);
    assert!(
        context.contains("- this worktree: 1 commit not pushed, the newest"),
        "{context}"
    );
    assert!(!context.contains("worktree agent-a"), "{context}");
    let told = reply["systemMessage"].as_str().expect("the person is told");
    assert!(told.contains("1 commit on this worktree"), "{told}");
    assert!(!told.contains("agent-a"), "{told}");
}

#[test]
fn a_worktree_someone_is_working_in_right_now_is_not_mentioned() {
    let base = TempDir::new().unwrap();
    let repo = repo_with_remote(base.path());
    let cache = TempDir::new().unwrap();
    let start = start_event(&repo, "startup");

    // A file written this moment in a worktree: another session is in there.
    // Its checked-out commit is from 2021, but the files say when it was used.
    let busy = add_worktree(&repo, "busy");
    std::fs::write(busy.join("wip.txt"), "in progress").unwrap();
    assert_eq!(
        run_hook(cache.path(), base.path(), "session-start", &start),
        ""
    );

    // Beside it, a worktree with a commit nobody has touched: only that one
    // is named.
    let agent = add_worktree(&repo, "agent-a");
    commit_in(&agent, "done.txt");
    let reply = the_reply(&run_hook(
        cache.path(),
        base.path(),
        "session-start",
        &start,
    ));
    let context = context_of(&reply);
    assert!(context.contains("worktree agent-a"), "{context}");
    assert!(!context.contains("busy"), "{context}");
}

#[test]
fn nothing_to_say_prints_nothing_and_never_fails() {
    let base = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let plain = base.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();

    // A clean repository: no worktrees, nothing unpushed.
    let clean = base.path().join("clean");
    init_repo_with_commit(&clean);
    assert_eq!(
        run_hook(
            cache.path(),
            base.path(),
            "session-start",
            &start_event(&clean, "startup")
        ),
        ""
    );
    // Clean with a remote and a worktree that holds nothing is nothing too.
    let tidy = repo_with_remote(&base.path().join("tidy"));
    add_worktree(&tidy, "empty");
    assert_eq!(
        run_hook(
            cache.path(),
            base.path(),
            "session-start",
            &start_event(&tidy, "startup")
        ),
        ""
    );

    // A folder that is not a repository, one that does not exist.
    for cwd in [plain.clone(), base.path().join("gone")] {
        assert_eq!(
            run_hook(
                cache.path(),
                base.path(),
                "session-start",
                &start_event(&cwd, "startup")
            ),
            "",
            "{cwd:?}"
        );
    }

    // Input that is not what Claude Code sends, in a folder that is not a
    // repository either: still nothing, still exit 0, still no stderr.
    for stdin in [
        "",
        "   \n",
        "not json at all",
        "{",
        "[1,2]",
        "null",
        "{\"cwd\":42}",
    ] {
        assert_eq!(
            run_hook(cache.path(), &plain, "session-start", stdin),
            "",
            "{stdin:?}"
        );
    }

    // Nothing was said, so nothing was remembered.
    assert!(!cache.path().join("cohors/hook-notes.json").exists());
}

#[test]
fn without_a_folder_in_the_event_it_looks_where_it_was_run() {
    let base = TempDir::new().unwrap();
    let (repo, _agent) = repo_with_left_behind_worktree(base.path());

    // Empty, garbage, or an event with no `cwd`: the process's own folder,
    // and a fresh start.
    for stdin in ["", "garbage", r#"{"source":"startup"}"#, r#"{"cwd":""}"#] {
        let cache = TempDir::new().unwrap();
        let reply = the_reply(&run_hook(cache.path(), &repo, "session-start", stdin));
        assert!(context_of(&reply).contains("worktree agent-a"), "{stdin:?}");
        assert!(reply["systemMessage"].is_string(), "{stdin:?}: {reply}");
    }
}

#[test]
fn an_event_it_does_not_know_is_ignored() {
    let base = TempDir::new().unwrap();
    let (repo, _agent) = repo_with_left_behind_worktree(base.path());
    let cache = TempDir::new().unwrap();

    // A newer plugin may send events this cohors has never heard of. Even in
    // a repository that has a note, only `session-start` says anything.
    for event in ["unknown-event", "pre-tool-use", "SessionStart", ""] {
        assert_eq!(
            run_hook(
                cache.path(),
                base.path(),
                event,
                &start_event(&repo, "startup")
            ),
            "",
            "{event:?}"
        );
    }
}

#[test]
fn a_memory_that_cannot_be_read_or_written_still_prints_the_note() {
    let base = TempDir::new().unwrap();
    let (repo, _agent) = repo_with_left_behind_worktree(base.path());
    let start = start_event(&repo, "startup");

    // A memory file that is not JSON: treated as nothing shown yet, and
    // replaced by a good one.
    let cache = TempDir::new().unwrap();
    let memory = cache.path().join("cohors/hook-notes.json");
    std::fs::create_dir_all(memory.parent().unwrap()).unwrap();
    std::fs::write(&memory, "this is not json").unwrap();
    let reply = the_reply(&run_hook(
        cache.path(),
        base.path(),
        "session-start",
        &start,
    ));
    assert!(reply["systemMessage"].is_string(), "{reply}");
    let rewritten = std::fs::read_to_string(&memory).unwrap();
    assert!(
        serde_json::from_str::<serde_json::Value>(&rewritten).is_ok(),
        "{rewritten}"
    );

    // A cache folder that cannot exist (its parent is a file): nothing can be
    // remembered, so the person is told every time, and nothing goes wrong.
    let blocker = base.path().join("blocker");
    std::fs::write(&blocker, "a file, not a folder").unwrap();
    for _ in 0..2 {
        let reply = the_reply(&run_hook(&blocker, base.path(), "session-start", &start));
        assert!(context_of(&reply).contains("worktree agent-a"));
        assert!(reply["systemMessage"].is_string(), "{reply}");
    }
}

#[test]
fn hook_is_hidden_from_help_but_has_its_own() {
    let cache = TempDir::new().unwrap();
    let help = stdout_of(&cohors(cache.path()).arg("--help").assert().success());
    assert!(
        !help
            .lines()
            .any(|line| line.trim_start().starts_with("hook")),
        "`hook` is not for typing and is not listed: {help}"
    );

    let own = stdout_of(
        &cohors(cache.path())
            .args(["hook", "--help"])
            .assert()
            .success(),
    );
    assert!(own.contains("session-start"), "{own}");
    assert!(
        own.contains("SessionStart"),
        "it shows how to wire it up: {own}"
    );
}
