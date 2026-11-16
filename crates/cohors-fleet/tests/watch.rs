//! The fleet watch on real fixture repos: what the probe notices (and what it
//! deliberately leaves to the periodic rescan), and that the watch publishes a
//! state when something changed and stays quiet when nothing did.

use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use cohors_fleet::watch::{Fingerprint, Probe};
use cohors_fleet::{FleetState, FleetWatch, Reason, Scanner, Subscription, WatchOptions};

/// Long enough for a slow CI machine, short of hanging a broken test forever.
const WAIT: Duration = Duration::from_secs(15);

fn git(dir: &Utf8Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir.as_str())
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A repo with one commit at `dir`.
fn make_repo(dir: &Utf8Path) {
    std::fs::create_dir_all(dir.as_std_path()).unwrap();
    let out = Command::new("git")
        .args(["init", "-q", "-b", "main", dir.as_str()])
        .output()
        .unwrap();
    assert!(out.status.success(), "git init: {out:?}");
    git(dir, &["config", "user.email", "t@example.com"]);
    git(dir, &["config", "user.name", "Tester"]);
    write(&dir.join("f.txt"), "one");
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "init"]);
}

fn write(path: &Utf8Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent.as_std_path()).unwrap();
    }
    std::fs::write(path.as_std_path(), body).unwrap();
}

fn commit(dir: &Utf8Path, file: &str, body: &str) {
    write(&dir.join(file), body);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", file]);
}

/// Two repos under `<tmp>/code`, a config naming that root, a file for
/// remembered repos and a launchpad home, all inside one temp dir.
struct Fixture {
    _tmp: tempfile::TempDir,
    root: Utf8PathBuf,
    config: Utf8PathBuf,
    known: Utf8PathBuf,
    home: Utf8PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        // Canonical, so paths compare with what discovery reports (macOS
        // reaches /var through a symlink).
        let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
        make_repo(&root.join("code/alpha"));
        make_repo(&root.join("code/beta"));
        let config = root.join("config.toml");
        write(&config, &format!("roots = [\"{root}/code\"]\n"));
        let home = root.join("home");
        std::fs::create_dir_all(home.as_std_path()).unwrap();
        Self {
            known: root.join("cache/known-repos.json"),
            _tmp: tmp,
            root,
            config,
            home,
        }
    }

    fn repo(&self, name: &str) -> Utf8PathBuf {
        self.root.join("code").join(name)
    }

    fn scanner(&self) -> Arc<Scanner> {
        Arc::new(
            Scanner::new(Some(&self.config), &[])
                .unwrap()
                .with_known_file(Some(self.known.clone())),
        )
    }

    fn probe(&self) -> (Probe, Fingerprint) {
        let scanner = self.scanner();
        let repos = scanner.discover(false).repos;
        let probe = Probe::new(&scanner, &repos, Some(self.home.as_std_path()));
        let reading = probe.read();
        (probe, reading)
    }

    /// A watch that probes often and never rescans on its own unless told to.
    fn watch(&self, rescan_every: Duration) -> FleetWatch {
        FleetWatch::start(
            self.scanner(),
            WatchOptions {
                probe_every: Duration::from_millis(40),
                rescan_every,
                rescan_at_most: rescan_every,
                launchpad_home: Some(self.home.as_std_path().to_path_buf()),
            },
        )
    }
}

/// The next state, failing the test if none comes.
fn next(sub: &Subscription) -> Arc<FleetState> {
    sub.recv_timeout(WAIT)
        .expect("the watch should publish a change")
}

/// States until one satisfies `done`, each checked by `each`. A git command
/// is several steps (`git add` then `git commit`), and the watch may publish
/// a state between them: that is correct, so tests wait for the last one.
fn until(
    sub: &Subscription,
    each: impl Fn(&FleetState),
    done: impl Fn(&FleetState) -> bool,
) -> Arc<FleetState> {
    loop {
        let state = next(sub);
        each(&state);
        if done(&state) {
            return state;
        }
    }
}

/// Assert nothing is published for a while (several probes' worth).
fn quiet(sub: &Subscription) {
    if let Ok(state) = sub.recv_timeout(Duration::from_millis(500)) {
        panic!(
            "nothing changed, yet state {} was published ({:?}, changed {:?})",
            state.seq, state.reason, state.changed
        );
    }
}

fn snapshot<'a>(state: &'a FleetState, repo: &Utf8Path) -> &'a cohors_core::RepoSnapshot {
    state
        .snapshots
        .iter()
        .find(|s| s.path.as_deref() == Some(repo))
        .unwrap_or_else(|| panic!("{repo} is not in the fleet"))
}

// ── The probe ───────────────────────────────────────────────────────────────

#[test]
fn the_probe_is_stable_and_scanning_does_not_move_it() {
    let fx = Fixture::new();
    let (probe, before) = fx.probe();
    assert_eq!(probe.len(), 2);
    // Reading the repos (status, branches, history) writes nothing git
    // tracks, or the watch would rescan itself forever.
    let _ = fx.scanner().scan();
    let after = probe.read();
    assert!(before.diff(&after).is_empty(), "{:?}", before.diff(&after));
}

#[test]
fn a_commit_moves_only_that_repos_git_state() {
    let fx = Fixture::new();
    let (probe, before) = fx.probe();
    commit(&fx.repo("alpha"), "g.txt", "two");
    let diff = before.diff(&probe.read());
    assert_eq!(diff.repos, [fx.repo("alpha")]);
    assert!(!diff.layout && !diff.launchpad && !diff.config, "{diff:?}");
}

#[test]
fn staging_stashing_and_branching_move_git_state() {
    let fx = Fixture::new();
    let alpha = fx.repo("alpha");
    let (mut probe, mut seen) = fx.probe();
    // As the watch does after each change: list the ref folders again, so a
    // new branch folder is stamped from then on.
    let mut moved = |what: &str| {
        let now = probe.read();
        assert_eq!(
            seen.diff(&now).repos,
            std::slice::from_ref(&alpha),
            "{what}"
        );
        probe.relist(std::slice::from_ref(&alpha));
        seen = probe.read();
    };

    write(&alpha.join("f.txt"), "changed");
    git(&alpha, &["add", "f.txt"]);
    moved("staging a file");
    git(&alpha, &["stash", "-q"]);
    moved("stashing");
    git(&alpha, &["stash", "drop", "-q"]);
    moved("dropping the stash");
    // A branch name with a slash lives in a folder under refs/heads.
    git(&alpha, &["checkout", "-q", "-b", "team/feature"]);
    moved("switching to a new branch");
    commit(&alpha, "h.txt", "three");
    moved("committing on a nested branch");
    // Moving a branch that is not checked out touches neither HEAD nor its
    // log: only the folder holding the ref.
    git(&alpha, &["branch", "-q", "-f", "team/other", "main"]);
    moved("creating another nested branch");
    git(&alpha, &["update-ref", "refs/heads/team/other", "HEAD"]);
    moved("moving a branch that is not checked out");
}

#[test]
fn editing_a_tracked_file_is_left_to_the_rescan() {
    // Git records nothing when a tracked file is edited, so the probe does
    // not see it. This is why the watch also rescans on an interval.
    let fx = Fixture::new();
    let (probe, before) = fx.probe();
    write(&fx.repo("alpha").join("f.txt"), "edited in place");
    assert!(before.diff(&probe.read()).is_empty());
}

#[test]
fn a_new_or_removed_repo_moves_the_layout() {
    let fx = Fixture::new();
    let (probe, before) = fx.probe();
    make_repo(&fx.repo("gamma"));
    let after = probe.read();
    assert!(before.diff(&after).layout);

    std::fs::remove_dir_all(fx.repo("beta").as_std_path()).unwrap();
    let diff = after.diff(&probe.read());
    assert!(diff.layout, "{diff:?}");
}

#[test]
fn launchpad_files_move_only_the_launchpad() {
    let fx = Fixture::new();
    let (probe, mut seen) = fx.probe();
    for (what, path) in [
        (
            "a repo's contribution",
            fx.repo("alpha").join(".cohors/repo.json"),
        ),
        ("the focus board", fx.home.join("board.json")),
        ("the activity log", fx.home.join("history/2026-11-16.jsonl")),
    ] {
        write(&path, "{}");
        let now = probe.read();
        let diff = seen.diff(&now);
        assert!(diff.launchpad, "{what}: {diff:?}");
        assert!(diff.repos.is_empty() && !diff.layout, "{what}: {diff:?}");
        seen = now;
    }
}

#[test]
fn the_config_and_the_remembered_repos_are_watched() {
    let fx = Fixture::new();
    let (probe, before) = fx.probe();
    write(
        &fx.config,
        &format!("roots = [\"{}/code\"]\nmax_depth = 3\n", fx.root),
    );
    let after = probe.read();
    assert!(before.diff(&after).config);

    write(&fx.known, "{\"repos\": []}");
    let diff = after.diff(&probe.read());
    assert!(diff.remembered && !diff.layout, "{diff:?}");
}

// ── The watch ───────────────────────────────────────────────────────────────

#[test]
fn the_watch_publishes_each_change_once_and_stays_quiet_otherwise() {
    let fx = Fixture::new();
    let watch = fx.watch(Duration::from_secs(600));
    let first = watch.wait_latest(WAIT).expect("a first scan");
    assert_eq!((first.seq, first.reason), (1, Reason::Start));
    assert_eq!(first.snapshots.len(), 2);
    assert_eq!(first.changed.len(), 2, "the first state names every repo");

    let sub = watch.subscribe();
    assert_eq!(
        next(&sub).seq,
        1,
        "a new subscriber starts from the current state"
    );
    quiet(&sub);
    assert!(watch.stats().probes > 3, "the probe kept running meanwhile");

    // A commit: just that repo is re-read.
    let alpha = fx.repo("alpha");
    commit(&alpha, "g.txt", "two");
    let state = until(
        &sub,
        |state| {
            assert_eq!(state.reason, Reason::Git);
            assert_eq!(state.changed, [alpha.to_string()]);
        },
        |state| {
            let snap = snapshot(state, &alpha);
            snap.last_commit.as_ref().unwrap().summary == "g.txt" && snap.worktree.staged == 0
        },
    );
    assert!(state.seq >= 2);
    let full_scans = watch.stats().full_scans;
    assert_eq!(
        full_scans, 1,
        "a git change re-reads one repo, not the fleet"
    );
    quiet(&sub);

    // A new repo under the root. `git init` and its first commit are
    // separate steps, so it may arrive in more than one state.
    let gamma = fx.repo("gamma");
    make_repo(&gamma);
    let state = until(
        &sub,
        |state| {
            assert!(
                matches!(state.reason, Reason::Layout | Reason::Git),
                "{:?}",
                state.reason
            );
            assert_eq!(state.changed, [gamma.to_string()]);
        },
        |state| {
            state
                .snapshots
                .iter()
                .any(|s| s.path.as_deref() == Some(&gamma) && s.last_commit.is_some())
        },
    );
    assert_eq!(state.snapshots.len(), 3);
    // The watch searches once more a second after a layout change (a new
    // repo arrives in steps); with nothing else new, that publishes nothing.
    std::thread::sleep(Duration::from_millis(1200));
    quiet(&sub);

    // A launchpad task: no git read at all.
    write(
        &fx.repo("beta").join(".cohors/repo.json"),
        r#"{"specVersion":1,"name":"beta","tasks":[{"id":"ship","title":"Ship it"}]}"#,
    );
    let state = next(&sub);
    assert_eq!(state.reason, Reason::Launchpad);
    assert_eq!(state.changed, [fx.repo("beta").to_string()]);
    let entry = state
        .launchpad
        .entries
        .iter()
        .find(|e| e.name == "beta")
        .unwrap();
    assert!(entry.configured);
    quiet(&sub);

    // An edited file waits for a rescan; asking for one shows it at once.
    write(&alpha.join("f.txt"), "edited");
    quiet(&sub);
    let state = watch.rescan(WAIT).expect("a rescan");
    assert_eq!(state.reason, Reason::Requested);
    assert_eq!(snapshot(&state, &alpha).worktree.modified, 1);
    assert_eq!(next(&sub).seq, state.seq);

    // Asking again with nothing changed scans, but publishes nothing.
    let again = watch.rescan(WAIT).expect("a rescan");
    assert_eq!(again.seq, state.seq);
    quiet(&sub);
}

#[test]
fn the_periodic_rescan_catches_an_edited_file() {
    let fx = Fixture::new();
    let watch = fx.watch(Duration::from_millis(200));
    let sub = watch.subscribe();
    assert_eq!(next(&sub).seq, 1);
    let beta = fx.repo("beta");
    write(&beta.join("f.txt"), "edited");
    let state = next(&sub);
    assert_eq!(state.reason, Reason::Rescan);
    assert_eq!(state.changed, [beta.to_string()]);
    assert_eq!(snapshot(&state, &beta).worktree.modified, 1);
    // Checks keep running, but an unchanged fleet publishes nothing; and
    // they re-read only the repo whose status moved.
    quiet(&sub);
    // The check interval stretches when checks get expensive (a loaded
    // machine, a full parallel test run), so wait for the third one rather
    // than assume when it lands.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut stats = watch.stats();
    while stats.worktree_checks <= 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        stats = watch.stats();
    }
    quiet(&sub);
    assert!(stats.worktree_checks > 2, "{stats:?}");
    assert_eq!(stats.repo_scans, 1, "{stats:?}");
    assert_eq!(stats.full_scans, 1, "{stats:?}");
}

#[test]
fn a_deep_search_elsewhere_and_a_config_edit_show_up() {
    let fx = Fixture::new();
    // A repo inside a repo: a normal search does not enter alpha.
    let nested = fx.repo("alpha/tools/linter");
    make_repo(&nested);
    let watch = fx.watch(Duration::from_secs(600));
    let sub = watch.subscribe();
    assert_eq!(next(&sub).snapshots.len(), 2);

    // `cohors scan --deep` in another process remembers it.
    let deep = fx.scanner().discover(true);
    assert_eq!(deep.deep_only, std::slice::from_ref(&nested));
    let state = next(&sub);
    assert_eq!(state.reason, Reason::Remembered);
    assert_eq!(state.changed, [nested.to_string()]);

    // A second root in the config: read again, and its repos appear.
    make_repo(&fx.root.join("more/delta"));
    write(
        &fx.config,
        &format!("roots = [\"{0}/code\", \"{0}/more\"]\n", fx.root),
    );
    let state = next(&sub);
    assert_eq!(state.reason, Reason::Config);
    assert!(
        state
            .changed
            .contains(&fx.root.join("more/delta").to_string())
    );
    assert_eq!(
        state.scanner.roots().len(),
        2,
        "the reloaded scanner comes along"
    );

    // A config that does not parse keeps the fleet as it was.
    write(&fx.config, "roots = [");
    quiet(&sub);
    assert!(watch.is_running());
}

#[test]
fn dropping_the_watch_ends_every_subscription() {
    let fx = Fixture::new();
    let watch = fx.watch(Duration::from_secs(600));
    let sub = watch.subscribe();
    assert!(sub.recv_timeout(WAIT).is_ok());
    drop(watch);
    assert!(
        sub.recv().is_none(),
        "a stopped watch closes its subscriptions"
    );
}
