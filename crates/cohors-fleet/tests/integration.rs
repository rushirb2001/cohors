//! Fleet integration tests: every upstream edge exercised on real fixtures.
//!
//! `cohors-fleet` composes three upstreams — config (what to scan and how to
//! label it), git (the snapshot facts), and github (token discovery; network
//! enrichment itself is downstream and never touched here). Each test builds a
//! real temp config + real temp repos and asserts the composition end-to-end,
//! so the front-ends can trust the Scanner/`detail_bundle` contract blindly.

use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};
use cohors_fleet::{Scanner, checkout_at, detail_bundle};

/// Create a git repo with one commit at `dir` (created if needed).
fn make_repo(dir: &Utf8Path) {
    std::fs::create_dir_all(dir.as_std_path()).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir.as_str())
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    };
    Command::new("git")
        .args(["init", "-q", "-b", "main", dir.as_str()])
        .output()
        .unwrap();
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "Tester"]);
    std::fs::write(dir.join("f.txt").as_std_path(), "one").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "init"]);
}

/// Write a config file and return its path.
fn write_config(dir: &Utf8Path, body: &str) -> Utf8PathBuf {
    let path = dir.join("config.toml");
    std::fs::write(path.as_std_path(), body).unwrap();
    path
}

fn tmp() -> (tempfile::TempDir, Utf8PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let p = Utf8PathBuf::from_path_buf(t.path().to_path_buf()).unwrap();
    (t, p)
}

/// Config edge: roots come from the config file; ignore globs prune discovery.
#[test]
fn scanner_honors_config_roots_and_ignore() {
    let (_t, root) = tmp();
    make_repo(&root.join("code/alpha"));
    make_repo(&root.join("code/skipme-x"));
    let cfg = write_config(
        &root,
        &format!(
            "roots = [\"{}/code\"]\nignore = [\"skipme*\", \"**/skipme*\"]\n",
            root
        ),
    );

    let scanner = Scanner::new(Some(&cfg), &[]).unwrap();
    let names: Vec<String> = scanner.scan().into_iter().map(|s| s.name).collect();
    assert!(names.contains(&"alpha".to_string()), "got {names:?}");
    assert!(
        !names.iter().any(|n| n.starts_with("skipme")),
        "ignore glob leaked: {names:?}"
    );
    assert_eq!(scanner.roots(), vec![format!("{root}/code")]);
}

/// Override edge: explicit roots (a CLI's `--root`) beat the config's roots.
#[test]
fn root_overrides_beat_config_roots() {
    let (_t, root) = tmp();
    make_repo(&root.join("config-side/one"));
    make_repo(&root.join("override-side/two"));
    let cfg = write_config(&root, &format!("roots = [\"{root}/config-side\"]\n"));

    let scanner = Scanner::new(Some(&cfg), &[format!("{root}/override-side")]).unwrap();
    let names: Vec<String> = scanner.scan().into_iter().map(|s| s.name).collect();
    assert_eq!(names, vec!["two".to_string()], "override should win");
}

/// Config edge: groups stamp by directory-name glob, and aliasing (a cosmetic
/// rename) happens AFTER grouping so it can't change membership.
#[test]
fn groups_stamp_before_aliases_rename() {
    let (_t, root) = tmp();
    make_repo(&root.join("code/payments"));
    make_repo(&root.join("code/website"));
    let cfg = write_config(
        &root,
        &format!(
            "roots = [\"{root}/code\"]\n\n[aliases]\npayments = \"PayCore\"\n\n[groups]\npay = [\"pay*\"]\n"
        ),
    );

    let scanner = Scanner::new(Some(&cfg), &[]).unwrap();
    let snaps = scanner.scan();
    let pay = snaps
        .iter()
        .find(|s| s.name == "PayCore")
        .expect("alias applied");
    assert_eq!(
        pay.groups,
        vec!["pay".to_string()],
        "grouped by dir name, not alias"
    );
    let web = snaps.iter().find(|s| s.name == "website").unwrap();
    assert!(web.groups.is_empty());
}

/// Git edge: the snapshot carries real git facts through the composition —
/// branch, last commit, dirty counts (a change made after the commit).
#[test]
fn scan_surfaces_git_facts() {
    let (_t, root) = tmp();
    let repo = root.join("code/facts");
    make_repo(&repo);
    std::fs::write(repo.join("f.txt").as_std_path(), "dirty now").unwrap();
    let cfg = write_config(&root, &format!("roots = [\"{root}/code\"]\n"));

    let snaps = Scanner::new(Some(&cfg), &[]).unwrap().scan();
    let s = &snaps[0];
    assert_eq!(s.branch, cohors_core::Branch::Named("main".into()));
    assert!(s.last_commit.is_some(), "last commit read via gix");
    assert!(s.worktree.modified >= 1, "dirty file counted via git2");
    assert!(s.error.is_none());
}

/// Detail edge: the one-repo bundle composes the two local reads (and skips the
/// remote one without a token) — commits, changed files, and a capped patch.
#[test]
fn detail_bundle_composes_local_reads() {
    let (_t, root) = tmp();
    let repo = root.join("bundled");
    make_repo(&repo);
    std::fs::write(repo.join("f.txt").as_std_path(), "changed").unwrap();

    let b = detail_bundle(Some(&repo), None, None, 20_000);
    assert_eq!(b.local.recent_commits.len(), 1);
    assert_eq!(b.changes.files.len(), 1);
    assert!(b.changes.patch.as_deref().unwrap_or("").contains("changed"));
    assert!(b.remote.is_none(), "no token → no remote read");

    // Path-less form degrades to defaults rather than erroring.
    let empty = detail_bundle(None, None, None, 20_000);
    assert!(empty.local.recent_commits.is_empty() && empty.changes.files.is_empty());
}

/// Error edge: an unreadable config is a typed error, not a panic or a silent
/// default — the front-end decides how to present it.
#[test]
fn invalid_config_is_a_fleet_error() {
    let (_t, root) = tmp();
    let cfg = write_config(&root, "roots = [not valid toml");
    let Err(err) = Scanner::new(Some(&cfg), &[]) else {
        panic!("invalid TOML must not build a scanner");
    };
    let msg = err.to_string();
    assert!(msg.contains("loading config"), "{msg}");
    assert!(msg.contains(&format!("invalid TOML in {cfg}")), "{msg}");
    // The cause survives the flattening: where and why, not just "invalid".
    assert!(msg.contains("line 1, column"), "{msg}");
}

/// Error edge: a config file named explicitly (`--config`) that does not exist
/// is an error naming the path and how to create it, never a silent fall back
/// to auto-detected roots (which scanned a different fleet).
#[test]
fn explicit_missing_config_is_a_fleet_error() {
    let (_t, root) = tmp();
    let missing = root.join("typo.toml");
    let Err(err) = Scanner::new(Some(&missing), &[]) else {
        panic!("a missing explicit config must not build a scanner");
    };
    let msg = err.to_string();
    assert!(msg.contains(missing.as_str()), "{msg}");
    assert!(
        msg.contains(&format!("cohors init --config {missing}")),
        "{msg}"
    );
}

/// The dashboard's constructor tolerates the missing file: its first-run
/// greeting and empty-state rescue are what create it at that path.
#[test]
fn dashboard_scanner_tolerates_a_missing_explicit_config() {
    let (_t, root) = tmp();
    make_repo(&root.join("code/solo"));
    let missing = root.join("not-yet.toml");
    let scanner =
        Scanner::new_allowing_missing_config(Some(&missing), &[format!("{root}/code")]).unwrap();
    assert_eq!(scanner.config_path(), missing.to_string());
    assert!(scanner.config_warnings().is_empty());
    let names: Vec<String> = scanner.scan().into_iter().map(|s| s.name).collect();
    assert_eq!(names, vec!["solo".to_string()]);
}

/// Config edge: unknown keys (typos) are surfaced for a front-end to print,
/// not only logged; the typo itself is ignored.
#[test]
fn config_warnings_name_unknown_keys() {
    let (_t, root) = tmp();
    let cfg = write_config(
        &root,
        &format!("roots = [\"{root}/code\"]\nmax_dept = 9\nthme = \"dark\"\n"),
    );
    let scanner = Scanner::new(Some(&cfg), &[]).unwrap();
    let warnings = scanner.config_warnings();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("`max_dept`") && w.contains("`max_depth`")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("`thme`") && w.contains("`theme`")),
        "{warnings:?}"
    );
}

/// GitHub edge (offline half): token discovery must never fail construction —
/// with or without `gh`/`$GITHUB_TOKEN`, the Scanner builds and exposes an
/// Option. (Network enrichment is downstream of fleet and not tested here.)
#[test]
fn token_discovery_is_best_effort() {
    let (_t, root) = tmp();
    make_repo(&root.join("code/tok"));
    let cfg = write_config(&root, &format!("roots = [\"{root}/code\"]\n"));
    let scanner = Scanner::new(Some(&cfg), &[]).unwrap();
    // Whatever the environment holds, this is Some or None — never a crash.
    let _ = scanner.github_token();
    let _ = scanner.author_email();
}

/// Nested-repo edge (ADR-058): a deep search remembers what a normal search
/// cannot reach, and every later normal scan includes it — the path that lets a
/// running dashboard pick up `cohors scan --deep` without a restart.
#[test]
fn a_deep_search_is_remembered_for_normal_scans() {
    let (_t, root) = tmp();
    // `code` is a repo; `code/archive` is an intermediate repo holding another.
    make_repo(&root.join("code"));
    make_repo(&root.join("code/projects/api-dashboard"));
    make_repo(&root.join("code/archive"));
    std::fs::create_dir_all(root.join("code/archive/tools").as_std_path()).unwrap();
    make_repo(&root.join("code/archive/tools/linter"));
    let cfg = write_config(
        &root,
        &format!("roots = [\"{root}/code\"]\nmax_depth = 4\n"),
    );
    let known = root.join("cache/known-repos.json");
    let scanner = Scanner::new(Some(&cfg), &[])
        .unwrap()
        .with_known_file(Some(known.clone()));
    let names = |snaps: Vec<cohors_core::RepoSnapshot>| {
        let mut n: Vec<String> = snaps.into_iter().map(|s| s.name).collect();
        n.sort();
        n
    };

    // Normal search: the root repo is walked (its nested repos are found), but
    // the repo inside `archive` is not, and nothing is remembered yet.
    assert_eq!(names(scanner.scan()), ["api-dashboard", "archive", "code"]);
    assert!(scanner.known_stamp().is_none());

    // A read-only deep search finds it too, but remembers nothing.
    let looked = scanner.discover_read_only(true);
    assert_eq!(looked.repos.len(), 4);
    assert!(looked.deep_only[0].as_str().ends_with("tools/linter"));
    assert_eq!(
        looked
            .nested_in(&looked.deep_only[0])
            .map(|p| p.as_str().ends_with("code/archive")),
        Some(true),
        "the closest repo around it"
    );
    assert!(scanner.known_stamp().is_none(), "nothing was written");
    assert_eq!(names(scanner.scan()), ["api-dashboard", "archive", "code"]);

    // The deep search finds it, says it was deep-only, and remembers it.
    let deep = scanner.discover(true);
    assert_eq!(deep.repos.len(), 4);
    assert_eq!(deep.deep_only.len(), 1);
    assert!(deep.deep_only[0].as_str().ends_with("tools/linter"));
    assert!(
        scanner.known_stamp().is_some(),
        "the memory file now exists"
    );

    // From now on a normal scan includes it, and says where it came from.
    assert_eq!(
        names(scanner.scan()),
        ["api-dashboard", "archive", "code", "linter"]
    );
    let normal = scanner.discover(false);
    assert_eq!(normal.deep_only, deep.deep_only);

    // Deleting the repo drops it again, with no deep search needed.
    std::fs::remove_dir_all(root.join("code/archive/tools/linter").as_std_path()).unwrap();
    assert_eq!(names(scanner.scan()), ["api-dashboard", "archive", "code"]);
}

/// Checkout edge: a session that opened in a linked worktree is handed the
/// whole repository (the main checkout, listing that worktree), and is told
/// which worktree it is in. No config, no roots: only the folder.
#[test]
fn checkout_at_reads_the_whole_repo_from_inside_a_linked_worktree() {
    let (_t, root) = tmp();
    let main = root.join("api");
    make_repo(&main);
    let git = |dir: &Utf8Path, args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir.as_str())
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    };
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            ".claude/worktrees/agent-a",
            "-b",
            "worktree-agent-a",
        ],
    );
    let agent = main.join(".claude/worktrees/agent-a");
    std::fs::write(agent.join("left.txt").as_std_path(), "never committed").unwrap();
    // On macOS the temp folder sits behind a symlink; the repo's path is the
    // real one.
    let real_main = Utf8PathBuf::from_path_buf(main.as_std_path().canonicalize().unwrap()).unwrap();

    // From inside the worktree: the main checkout's snapshot, which lists
    // the worktree (with its own uncommitted file), and the worktree's name.
    let inside = checkout_at(&agent, 1_700_000_000).expect("a worktree is a checkout");
    assert_eq!(inside.worktree.as_deref(), Some("agent-a"));
    assert_eq!(inside.repo.path.as_deref(), Some(real_main.as_path()));
    assert_eq!(inside.repo.name, "api");
    assert_eq!(
        inside.repo.worktrees.len(),
        1,
        "{:?}",
        inside.repo.worktrees
    );
    assert_eq!(inside.repo.worktrees[0].name, "agent-a");
    assert_eq!(inside.repo.worktrees[0].status.untracked, 1);

    // From the main checkout, or a folder inside it: the same repository, in
    // no worktree.
    std::fs::create_dir_all(main.join("src").as_std_path()).unwrap();
    for folder in [main.clone(), main.join("src")] {
        let at = checkout_at(&folder, 1_700_000_000).expect("the main checkout");
        assert_eq!(at.worktree, None, "{folder}");
        assert_eq!(at.repo.path.as_deref(), Some(real_main.as_path()));
        assert_eq!(at.repo.worktrees.len(), 1);
    }

    // A folder that is not in a repository, or is not there: nothing.
    let plain = root.join("plain");
    std::fs::create_dir_all(plain.as_std_path()).unwrap();
    assert!(checkout_at(&plain, 1_700_000_000).is_none());
    assert!(checkout_at(&root.join("gone"), 1_700_000_000).is_none());
}

/// One repository is the whole fleet: its linked worktrees are rows of their
/// own (ADR-077), each saying what it holds that is on no remote, and a
/// commit two of them share counted once in the total. With a second
/// repository beside it, they are that repository's own list again (ADR-071).
#[test]
fn a_lone_repositorys_worktrees_are_rows_and_a_fleets_are_not() {
    let (_t, root) = tmp();
    let git = |dir: &Utf8Path, args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir.as_str())
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    };
    let main = root.join("code/shop");
    make_repo(&main);
    // A remote, so "not pushed" means something.
    let remote = root.join("remote.git");
    std::fs::create_dir_all(remote.as_std_path()).unwrap();
    git(&remote, &["init", "-q", "--bare", "-b", "main"]);
    git(&main, &["remote", "add", "origin", remote.as_str()]);
    git(&main, &["push", "-q", "-u", "origin", "main"]);
    // A stash: it belongs to the repository, whichever checkout lists it.
    std::fs::write(main.join("f.txt").as_std_path(), "changed").unwrap();
    git(&main, &["stash", "-q"]);

    // One agent's worktree with a commit and a file it never committed, and
    // a second agent's, cut from the first, with one commit more.
    let commit = |dir: &Utf8Path, file: &str| {
        std::fs::write(dir.join(file).as_std_path(), "work").unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", file]);
    };
    let first = main.join(".claude/worktrees/agent-a");
    let second = main.join(".claude/worktrees/agent-b");
    git(
        &main,
        &["worktree", "add", "-q", first.as_str(), "-b", "agent-a"],
    );
    commit(&first, "a.txt");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            second.as_str(),
            "-b",
            "agent-b",
            "agent-a",
        ],
    );
    commit(&second, "b.txt");
    std::fs::write(first.join("left.txt").as_std_path(), "never committed").unwrap();

    let cfg = write_config(&root, &format!("roots = [\"{root}/code\"]\n"));
    let real = |path: &Utf8Path| {
        Utf8PathBuf::from_path_buf(path.as_std_path().canonicalize().unwrap()).unwrap()
    };
    let check = |rows: &[cohors_core::RepoSnapshot]| {
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["shop", "agent-a", "agent-b"]);
        assert_eq!(
            cohors_core::lone_repository(rows),
            Some(real(&main).as_path())
        );
        let (shop, a, b) = (&rows[0], &rows[1], &rows[2]);
        // The main checkout: its worktrees are rows now, not its own list.
        assert!(shop.linked.is_none() && shop.worktrees.is_empty());
        assert_eq!(shop.stash_count, 1);
        // Each worktree's row: what it holds, and what only it holds.
        let held = |row: &cohors_core::RepoSnapshot| {
            let linked = row.linked.as_ref().expect("a linked worktree's row");
            assert_eq!(linked.main, real(&main));
            (linked.unpushed, linked.unpushed_unique)
        };
        assert_eq!(held(a), (1, 1));
        assert_eq!(held(b), (2, 1), "one of its two commits is agent-a's");
        assert_eq!((a.unpushed(), b.unpushed()), (1, 2));
        assert_eq!(a.worktree.untracked, 1);
        // The repository's one stash is reported once, by the main checkout.
        assert_eq!((a.stash_count, b.stash_count), (0, 0));
        // This week's commits: a worktree counts what the main checkout does
        // not already have (two worktrees can still share one between them).
        let this_week = |row: &cohors_core::RepoSnapshot| row.activity.last().copied();
        assert_eq!(
            (this_week(shop), this_week(a), this_week(b)),
            (Some(1), Some(1), Some(2))
        );
    };

    // Pointed at the folder that holds the repository, at the repository
    // itself, or at one of its worktrees: the same three checkouts.
    let rows = Scanner::new(Some(&cfg), &[]).unwrap().scan();
    check(&rows);
    for at in [&main, &second] {
        check(&Scanner::new(Some(&cfg), &[at.to_string()]).unwrap().scan());
    }

    // The words say "checkout", and the total counts the shared commit once.
    let overview = cohors_core::overview(&rows, &[], &Default::default(), 0);
    assert!(
        overview.headline.starts_with("3 checkouts."),
        "{}",
        overview.headline
    );
    let push = overview
        .actions
        .iter()
        .find(|a| a.kind == cohors_core::ActionKind::Push)
        .expect("a push step");
    assert_eq!(push.title, "Push 2 commits from 2 checkouts");
    assert!(
        !overview
            .actions
            .iter()
            .any(|a| a.kind == cohors_core::ActionKind::Worktrees),
        "the worktrees are rows, so there is no separate step about them"
    );

    // A second repository beside it makes a fleet: one row each, and the
    // worktrees are the first repository's own list again.
    make_repo(&root.join("code/other"));
    let rows = Scanner::new(Some(&cfg), &[]).unwrap().scan();
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["other", "shop"]);
    assert_eq!(cohors_core::lone_repository(&rows), None);
    let shop = &rows[1];
    assert_eq!(shop.worktrees.len(), 2);
    assert!(rows.iter().all(|r| r.linked.is_none()));
}
