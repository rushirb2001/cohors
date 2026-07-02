//! Fleet integration tests: every upstream edge exercised on real fixtures.
//!
//! `cohors-fleet` composes three upstreams — config (what to scan and how to
//! label it), git (the snapshot facts), and github (token discovery; network
//! enrichment itself is downstream and never touched here). Each test builds a
//! real temp config + real temp repos and asserts the composition end-to-end,
//! so the front-ends can trust the Scanner/`detail_bundle` contract blindly.

use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};
use cohors_fleet::{Scanner, detail_bundle};

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
    assert!(err.to_string().contains("loading config"), "{err}");
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
