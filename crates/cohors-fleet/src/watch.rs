//! Live fleet updates: the one change source every front-end subscribes to.
//!
//! [`FleetWatch`] keeps a scan of the fleet current on a background thread
//! and tells its subscribers when, and only when, something they would show
//! has changed. The terminal dashboard, the web server's event stream and
//! `cohors watch` all read from it, so they agree on what changed and when.
//!
//! It works in three layers, cheapest first:
//!
//! 1. **The probe** ([`Probe`]), every half second or so. It reads no file:
//!    it `stat`s the files git rewrites whenever a repository's state moves
//!    (`HEAD`, `index`, `FETCH_HEAD`, `packed-refs`, `logs/HEAD`, the stash
//!    log, the merge and rebase markers, and every folder under `refs/`),
//!    each repository's `.cohors/repo.json`, the focus board and the activity
//!    log, the folders repositories live in, the file a deep search remembers
//!    repositories in, and the config file. What moved decides the work:
//!    - one repository's git state: re-read just that repository;
//!    - a folder or the remembered repositories: search the roots again;
//!    - the config file: read it again, then search;
//!    - a launchpad file: re-read the launchpad, no git at all.
//! 2. **A worktree check** on a slower interval, because editing a tracked
//!    file changes nothing under `.git`. It searches the roots again and reads
//!    each repository's worktree status only (no history, branches or
//!    stashes: those live under `.git`, where the probe looks), then re-reads
//!    in full just the repositories whose status moved. Its interval stretches
//!    with what a check costs ([`RESCAN_COST_FACTOR`]), so a large fleet is
//!    not checked back to back. Once a minute a full rescan reads everything,
//!    for what only the clock moves (the week a commit counts in).
//! 3. **Change detection.** A scan is published only when the snapshots or
//!    the launchpad differ from the ones last published. Nothing in a
//!    snapshot records when it was taken, so an unchanged fleet compares
//!    equal and subscribers hear nothing.
//!
//! Why not file-system events (the `notify` crate)? They would catch an
//! edited file at once, but on Linux inotify needs one watch per directory,
//! and a fleet's worktrees (with their `node_modules` and `target` folders)
//! easily pass the per-user limit, which other tools (editors, bundlers)
//! share. Git already funnels every state change through a few files, so a
//! `stat` of those covers commits, checkouts, staging, fetches, pushes,
//! pulls, stashes and merges at any fleet size; only worktree edits wait for
//! the rescan.
//!
//! Nothing here blocks a caller: the scanning happens on the watch's own
//! thread, and a subscriber reads finished states from a channel.

use std::collections::{BTreeSet, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use camino::Utf8PathBuf;
use cohors_core::RepoSnapshot;
use serde::Serialize;

use crate::{LaunchpadBundle, Scanner, launchpad_bundle};

/// A full rescan waits at least this many times as long as the last one
/// took, so rescanning never uses more than about a twentieth of one core.
/// (A background rescan reads one repository at a time, so the time it takes
/// is close to the CPU it uses.)
pub const RESCAN_COST_FACTOR: u32 = 20;

/// The probe waits at least this many times as long as the last probe took.
pub const PROBE_COST_FACTOR: u32 = 20;

/// However quiet the fleet, everything is read again this often.
const FULL_RESCAN_EVERY: Duration = Duration::from_secs(60);

/// After repositories appear or disappear, search once more this much later.
/// A new repository arrives in steps (`mkdir`, then `git init` or a clone
/// filling in `.git`), and the first search can land between them; once the
/// folder exists, what happens inside it touches nothing the probe stamps.
const LAYOUT_FOLLOW_UP: Duration = Duration::from_secs(1);

/// How often the watch looks, and how hard.
#[derive(Debug, Clone)]
pub struct WatchOptions {
    /// The shortest wait between two probes.
    pub probe_every: Duration,
    /// The shortest wait between two worktree checks (the ones that catch an
    /// edited file). A slow check stretches it; see [`RESCAN_COST_FACTOR`].
    pub rescan_every: Duration,
    /// The longest wait between two worktree checks, however slow one is.
    pub rescan_at_most: Duration,
    /// Where the focus board and the activity log live (`~/.cohors`).
    pub launchpad_home: Option<PathBuf>,
}

impl Default for WatchOptions {
    /// Measured on the 24-repository demo fleet: a probe reads in 0.4 ms of
    /// CPU and a worktree check in about 25 ms (a full rescan, one repository
    /// at a time, about 65 ms). Probing every half second and checking every
    /// three seconds costs about 1% of one core while idle; a git change then
    /// shows within a second, and an edited file within three. A larger fleet
    /// stretches both intervals ([`RESCAN_COST_FACTOR`], [`PROBE_COST_FACTOR`]).
    fn default() -> Self {
        Self {
            probe_every: Duration::from_millis(500),
            rescan_every: Duration::from_secs(3),
            rescan_at_most: Duration::from_secs(60),
            launchpad_home: crate::launchpad::launchpad_home(),
        }
    }
}

/// Why a state was published.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// The first scan.
    Start,
    /// The config file changed and was read again.
    Config,
    /// Repositories appeared or disappeared under the roots.
    Layout,
    /// A deep search (`cohors scan --deep`, possibly in another terminal)
    /// remembered repositories a normal search does not reach.
    Remembered,
    /// A repository's git state moved: a commit, checkout, stage, fetch,
    /// push, pull, stash, merge or rebase.
    Git,
    /// A launchpad file changed: a repository's `.cohors/repo.json`, the
    /// focus board, or the activity log.
    Launchpad,
    /// The periodic full rescan found a change git does not record (an
    /// edited file in a worktree).
    Rescan,
    /// A front-end asked for a rescan (a refresh key, an action that ran).
    Requested,
}

/// One published state of the fleet.
pub struct FleetState {
    /// 1 for the first scan, then one more for every published change.
    pub seq: u64,
    /// Every repository, local facts only (GitHub enrichment is the caller's).
    pub snapshots: Vec<RepoSnapshot>,
    /// The launchpad read alongside: each repository's entry, the focus
    /// board and recent agent activity.
    pub launchpad: LaunchpadBundle,
    /// The scanner that produced it. A new one after the config was reloaded,
    /// so a front-end can pick up the new roots.
    pub scanner: Arc<Scanner>,
    /// What set this scan off.
    pub reason: Reason,
    /// Ids of the repositories that appeared, disappeared or changed since
    /// the previous state (all of them for the first).
    pub changed: Vec<String>,
}

/// What the last probe and scan cost, and the intervals they led to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WatchStats {
    /// Repositories being probed.
    pub repos: usize,
    /// The last probe.
    pub probe: Duration,
    /// The last full rescan (search plus every snapshot).
    pub full_scan: Duration,
    /// The last worktree check (search plus every worktree's status).
    pub worktree_check: Duration,
    /// The wait between probes now in force.
    pub probe_every: Duration,
    /// The wait between worktree checks now in force.
    pub rescan_every: Duration,
    /// Probes, full rescans, worktree checks and single-repository re-reads
    /// run so far.
    pub probes: u64,
    pub full_scans: u64,
    pub worktree_checks: u64,
    pub repo_scans: u64,
}

/// A running watch. Dropping it stops the thread (after any scan in flight).
/// Share it between threads with an `Arc`.
pub struct FleetWatch {
    shared: Arc<Shared>,
}

/// One subscriber's queue of published states.
pub struct Subscription {
    rx: Receiver<Arc<FleetState>>,
}

#[derive(Default)]
struct Shared {
    inner: Mutex<Inner>,
    /// Signalled whenever a state is published or a full rescan finishes.
    published: Condvar,
    /// Wakes the worker early: a requested rescan, or stop.
    wake: Condvar,
}

#[derive(Default)]
struct Inner {
    latest: Option<Arc<FleetState>>,
    subscribers: Vec<Sender<Arc<FleetState>>>,
    rescan_requested: bool,
    /// Full rescans begun so far (`stats.full_scans` counts those finished).
    scans_started: u64,
    stop: bool,
    /// The worker thread is gone (it could not start, or it panicked).
    dead: bool,
    stats: WatchStats,
}

/// Lock without ever panicking: a poisoned lock still holds valid data here
/// (every update is a plain assignment).
fn lock(mutex: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl FleetWatch {
    /// Start watching the fleet `scanner` describes. Returns at once; the
    /// first scan runs on the watch's thread and arrives as state 1.
    pub fn start(scanner: Arc<Scanner>, options: WatchOptions) -> Self {
        let shared = Arc::new(Shared::default());
        let worker_shared = shared.clone();
        let spawned = thread::Builder::new()
            .name("cohors-watch".to_string())
            .spawn(move || {
                let guard = worker_shared.clone();
                // A bug must not take the front-end down with it, nor leave
                // subscribers waiting forever: mark the watch dead and close
                // every subscription, so each surface falls back to scanning.
                let run = std::panic::AssertUnwindSafe(|| {
                    Worker::new(worker_shared, scanner, options).run()
                });
                if std::panic::catch_unwind(run).is_err() {
                    tracing::error!("the fleet watch stopped after a panic");
                    guard.die();
                }
            });
        if let Err(err) = spawned {
            tracing::error!(error = %err, "could not start the fleet watch");
            shared.die();
        }
        Self { shared }
    }

    /// Hear about every published state from now on, starting with the
    /// current one when there is one.
    pub fn subscribe(&self) -> Subscription {
        let (tx, rx) = mpsc::channel();
        let mut inner = lock(&self.shared.inner);
        if let Some(latest) = &inner.latest {
            let _ = tx.send(latest.clone());
        }
        if !inner.dead {
            inner.subscribers.push(tx);
        }
        Subscription { rx }
    }

    /// The newest published state, if the first scan has finished.
    pub fn latest(&self) -> Option<Arc<FleetState>> {
        lock(&self.shared.inner).latest.clone()
    }

    /// The newest state, waiting up to `timeout` for the first scan.
    pub fn wait_latest(&self, timeout: Duration) -> Option<Arc<FleetState>> {
        let deadline = Instant::now() + timeout;
        let mut inner = lock(&self.shared.inner);
        loop {
            if inner.latest.is_some() || inner.dead {
                return inner.latest.clone();
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            inner = match self.shared.published.wait_timeout(inner, left) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    /// Ask for a full rescan soon, without waiting for it.
    pub fn request_rescan(&self) {
        let mut inner = lock(&self.shared.inner);
        inner.rescan_requested = true;
        self.shared.wake.notify_all();
    }

    /// Run a full rescan now and return the state after it, waiting up to
    /// `timeout`. For a surface that must show what is on disk this moment
    /// (a refresh the user asked for, or right after an action ran). `None`
    /// when the watch is not running or the scan took longer than `timeout`.
    pub fn rescan(&self, timeout: Duration) -> Option<Arc<FleetState>> {
        let deadline = Instant::now() + timeout;
        let mut inner = lock(&self.shared.inner);
        if inner.dead {
            return None;
        }
        // Only a scan that begins after this request will do: one already
        // running may have read the disk before the change the caller wants.
        let wanted = inner.scans_started + 1;
        inner.rescan_requested = true;
        self.shared.wake.notify_all();
        loop {
            if inner.dead {
                return None;
            }
            if inner.stats.full_scans >= wanted && inner.latest.is_some() {
                return inner.latest.clone();
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            inner = match self.shared.published.wait_timeout(inner, left) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    /// False once the watch's thread is gone. A surface then reads the fleet
    /// itself rather than serve a state that will never move again.
    pub fn is_running(&self) -> bool {
        !lock(&self.shared.inner).dead
    }

    /// What the probe and the scans cost so far.
    pub fn stats(&self) -> WatchStats {
        lock(&self.shared.inner).stats
    }
}

impl Drop for FleetWatch {
    fn drop(&mut self) {
        let mut inner = lock(&self.shared.inner);
        inner.stop = true;
        inner.subscribers.clear();
        self.shared.wake.notify_all();
    }
}

impl Shared {
    fn die(&self) {
        let mut inner = lock(&self.inner);
        inner.dead = true;
        inner.subscribers.clear();
        self.published.notify_all();
    }
}

impl Subscription {
    /// Wait for the next state. `None` once the watch has stopped.
    pub fn recv(&self) -> Option<Arc<FleetState>> {
        self.rx.recv().ok().map(|first| self.newest(first))
    }

    /// Wait up to `timeout` for the next state. `Err(Timeout)` when nothing
    /// changed in that time, `Err(Disconnected)` once the watch has stopped.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Arc<FleetState>, RecvTimeoutError> {
        self.rx
            .recv_timeout(timeout)
            .map(|first| self.newest(first))
    }

    /// The newest state waiting, if any, without blocking.
    pub fn try_recv(&self) -> Option<Arc<FleetState>> {
        self.rx.try_recv().ok().map(|first| self.newest(first))
    }

    /// States queued behind `first` supersede it: a subscriber that fell
    /// behind only needs the newest.
    fn newest(&self, mut state: Arc<FleetState>) -> Arc<FleetState> {
        while let Ok(newer) = self.rx.try_recv() {
            state = newer;
        }
        state
    }
}

// ── The worker ──────────────────────────────────────────────────────────────

enum Wake {
    Tick,
    Rescan,
    Stop,
}

struct Worker {
    shared: Arc<Shared>,
    scanner: Arc<Scanner>,
    options: WatchOptions,
    probe: Probe,
    /// The fingerprint the current snapshots and launchpad reflect.
    seen: Fingerprint,
    snapshots: Vec<RepoSnapshot>,
    launchpad: LaunchpadBundle,
    seq: u64,
    last_full: Instant,
    /// The last time every worktree's status was read (a full rescan or a
    /// worktree check).
    last_check: Instant,
    stats: WatchStats,
    /// The config stamp the scanner was built from, and one that changed
    /// and is waiting to hold still before it is read.
    config_seen: Option<u64>,
    config_settling: Option<u64>,
    /// When to search again after a layout change (see [`LAYOUT_FOLLOW_UP`]).
    follow_up: Option<Instant>,
}

impl Worker {
    fn new(shared: Arc<Shared>, scanner: Arc<Scanner>, options: WatchOptions) -> Self {
        let probe = Probe::new(&scanner, &[], options.launchpad_home.as_deref());
        Self {
            shared,
            scanner,
            options,
            probe,
            seen: Fingerprint::default(),
            snapshots: Vec::new(),
            launchpad: LaunchpadBundle::default(),
            seq: 0,
            last_full: Instant::now(),
            last_check: Instant::now(),
            stats: WatchStats::default(),
            config_seen: None,
            config_settling: None,
            follow_up: None,
        }
    }

    fn run(mut self) {
        self.full(Reason::Start);
        loop {
            let requested = match self.sleep(self.probe_interval()) {
                Wake::Stop => return,
                Wake::Rescan => true,
                Wake::Tick => false,
            };
            let started = Instant::now();
            let now = self.probe.read();
            self.stats.probe = started.elapsed();
            self.stats.probes += 1;
            let diff = self.seen.diff(&now);

            if self.config_seen != Some(now.config) {
                if self.config_settling == Some(now.config) {
                    self.config_settling = None;
                    self.config_seen = Some(now.config);
                    match self.scanner.reload() {
                        Ok(reloaded) => {
                            self.scanner = Arc::new(reloaded);
                            self.full(Reason::Config);
                            continue;
                        }
                        Err(err) => {
                            // Keep the scanner we have: a config that does
                            // not parse must not empty the dashboard. The
                            // next save is read again.
                            tracing::warn!(error = %err, "config changed but does not load; keeping the old one");
                        }
                    }
                } else {
                    // An editor may be partway through saving it in place,
                    // and a half-written config reads as an empty one (which
                    // would search somewhere else entirely). Read it once it
                    // has held still for one probe.
                    self.config_settling = Some(now.config);
                }
            }
            if requested {
                self.full(Reason::Requested);
            } else if diff.layout || diff.remembered {
                self.full(if diff.layout {
                    Reason::Layout
                } else {
                    Reason::Remembered
                });
                self.follow_up = Some(Instant::now() + LAYOUT_FOLLOW_UP);
            } else if self.follow_up.is_some_and(|at| Instant::now() >= at) {
                self.follow_up = None;
                self.full(Reason::Layout);
            } else if self.last_full.elapsed() >= FULL_RESCAN_EVERY {
                // Due anyway; name the cause a git move if there was one.
                let reason = if diff.repos.is_empty() {
                    Reason::Rescan
                } else {
                    Reason::Git
                };
                self.full(reason);
            } else if !diff.repos.is_empty() {
                self.repos(&diff.repos);
            } else if self.last_check.elapsed() >= self.rescan_interval() {
                self.worktrees(now);
            } else if diff.launchpad {
                self.launchpad =
                    launchpad_bundle(&self.snapshots, self.options.launchpad_home.as_deref());
                self.seen = now;
                self.publish(Reason::Launchpad);
            }
        }
    }

    /// Search the roots, snapshot everything found, read the launchpad.
    fn full(&mut self, reason: Reason) {
        let started = Instant::now();
        lock(&self.shared.inner).scans_started += 1;
        let found = self.scanner.discover(false);
        // The fingerprint is taken before the snapshots, so a change that
        // lands while they are read is seen by the next probe, not lost.
        self.probe = Probe::new(
            &self.scanner,
            &found.repos,
            self.options.launchpad_home.as_deref(),
        );
        let fingerprint = self.probe.read();
        // The config is judged on its own (see `run`); the first scan is
        // what it is compared with.
        self.config_seen.get_or_insert(fingerprint.config);
        // A folder moved but the same repositories are there (or a follow-up
        // search found nothing new): whatever changed is a rescan's finding.
        let same_repos = found.repos.len() == self.snapshots.len()
            && found
                .repos
                .iter()
                .zip(&self.snapshots)
                .all(|(path, snap)| snap.path.as_ref() == Some(path));
        let reason = if reason == Reason::Layout && same_repos {
            Reason::Rescan
        } else {
            reason
        };
        // Someone is waiting on the first scan and on one they asked for:
        // use every core. Otherwise read one repository at a time, which
        // costs well under half the CPU (see `snapshot_repos_serially`).
        self.snapshots = if matches!(reason, Reason::Start | Reason::Requested) {
            self.scanner.snapshot_repos(&found.repos)
        } else {
            self.scanner.snapshot_repos_serially(&found.repos)
        };
        self.launchpad = launchpad_bundle(&self.snapshots, self.options.launchpad_home.as_deref());
        self.seen = fingerprint;
        self.last_full = Instant::now();
        self.last_check = self.last_full;
        self.stats.full_scan = started.elapsed();
        self.stats.full_scans += 1;
        self.stats.repos = found.repos.len();
        tracing::debug!(
            repos = found.repos.len(),
            took_ms = self.stats.full_scan.as_millis() as u64,
            ?reason,
            "fleet watch: full rescan"
        );
        self.publish(reason);
    }

    /// What git does not record: an edited file. Search the roots again (a
    /// repository may have appeared where the probe does not look), read each
    /// repository's worktree status, and re-read in full only those whose
    /// status moved. `now` is the probe's reading from just before.
    fn worktrees(&mut self, now: Fingerprint) {
        let started = Instant::now();
        let found = self.scanner.discover(false);
        let same_repos = found.repos.len() == self.snapshots.len()
            && found
                .repos
                .iter()
                .zip(&self.snapshots)
                .all(|(path, snap)| snap.path.as_ref() == Some(path));
        if !same_repos {
            self.full(Reason::Layout);
            return;
        }
        let moved: Vec<Utf8PathBuf> = self
            .snapshots
            .iter()
            .filter_map(|snap| {
                let path = snap.path.as_ref()?;
                let moved = match cohors_git::worktree_state(path) {
                    Some(worktree) => worktree != snap.worktree,
                    // It cannot be opened: news only if it could be before.
                    None => snap.error.is_none(),
                };
                moved.then(|| path.clone())
            })
            .collect();
        if !moved.is_empty() {
            for snap in self.scanner.snapshot_repos_serially(&moved) {
                if let Some(slot) = self.snapshots.iter_mut().find(|s| s.id == snap.id) {
                    *slot = snap;
                }
            }
            self.stats.repo_scans += moved.len() as u64;
        }
        // Manifests (`package.json`, a `Makefile`) are worktree files too,
        // and the launchpad reads them for detected flows.
        self.launchpad = launchpad_bundle(&self.snapshots, self.options.launchpad_home.as_deref());
        self.seen = now;
        self.last_check = Instant::now();
        self.stats.worktree_check = started.elapsed();
        self.stats.worktree_checks += 1;
        self.publish(Reason::Rescan);
    }

    /// Re-read only `changed` (whose git state moved), keeping the rest.
    fn repos(&mut self, changed: &[Utf8PathBuf]) {
        // A repository whose git folder vanished has been deleted or turned
        // into a plain folder: the search decides what is left.
        if changed.iter().any(|repo| !self.probe.has_git_dir(repo)) {
            self.full(Reason::Layout);
            return;
        }
        // List their `refs/` folders again (a new branch folder), and take
        // the new baseline before reading them, so nothing in between is lost.
        self.probe.relist(changed);
        let now = self.probe.read();
        let fresh = self.scanner.snapshot_repos_serially(changed);
        for snap in fresh {
            if let Some(slot) = self.snapshots.iter_mut().find(|s| s.id == snap.id) {
                *slot = snap;
            }
        }
        self.launchpad = launchpad_bundle(&self.snapshots, self.options.launchpad_home.as_deref());
        self.seen = now;
        self.stats.repo_scans += changed.len() as u64;
        self.publish(Reason::Git);
    }

    /// Publish the current snapshots and launchpad if they differ from the
    /// last published state; record the stats either way.
    fn publish(&mut self, reason: Reason) {
        // Every path here re-reads some or all of the repos, and a fresh
        // snapshot lists every linked worktree again. Drop the ones that are
        // rows of the fleet, which needs the whole fleet, so it happens here.
        crate::scanner::dedupe_worktrees(&mut self.snapshots);
        self.stats.probe_every = self.probe_interval();
        self.stats.rescan_every = self.rescan_interval();
        let mut inner = lock(&self.shared.inner);
        inner.stats = self.stats;
        let previous = inner.latest.clone();
        let changed = match &previous {
            None => self.snapshots.iter().map(|s| s.id.0.clone()).collect(),
            Some(old) if old.snapshots == self.snapshots && old.launchpad == self.launchpad => {
                // Nothing a front-end shows has moved. A full rescan still
                // counts as finished for anyone waiting on one.
                self.shared.published.notify_all();
                return;
            }
            Some(old) => changed_ids(old, &self.snapshots, &self.launchpad),
        };
        self.seq += 1;
        let state = Arc::new(FleetState {
            seq: self.seq,
            snapshots: self.snapshots.clone(),
            launchpad: self.launchpad.clone(),
            scanner: self.scanner.clone(),
            reason,
            changed,
        });
        tracing::debug!(
            seq = self.seq,
            ?reason,
            changed = state.changed.len(),
            "fleet watch: published"
        );
        inner.latest = Some(state.clone());
        // A subscriber that went away (its receiver dropped) is forgotten.
        inner
            .subscribers
            .retain(|tx| tx.send(state.clone()).is_ok());
        self.shared.published.notify_all();
    }

    /// Sleep until the next probe is due, a rescan is requested, or stop.
    fn sleep(&mut self, wait: Duration) -> Wake {
        let deadline = Instant::now() + wait;
        self.stats.probe_every = self.probe_interval();
        self.stats.rescan_every = self.rescan_interval();
        let mut inner = lock(&self.shared.inner);
        inner.stats = self.stats;
        loop {
            if inner.stop {
                return Wake::Stop;
            }
            if inner.rescan_requested {
                inner.rescan_requested = false;
                return Wake::Rescan;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return Wake::Tick;
            };
            inner = match self.shared.wake.wait_timeout(inner, left) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    fn probe_interval(&self) -> Duration {
        self.options
            .probe_every
            .max(self.stats.probe * PROBE_COST_FACTOR)
    }

    fn rescan_interval(&self) -> Duration {
        let floor = self.options.rescan_every;
        let ceiling = self.options.rescan_at_most.max(floor);
        let cost = if self.stats.worktree_checks > 0 {
            self.stats.worktree_check
        } else {
            self.stats.full_scan
        };
        (cost * RESCAN_COST_FACTOR).clamp(floor, ceiling)
    }
}

/// The repositories that differ between `old` and the new snapshots and
/// launchpad: added, removed, or changed in any field.
fn changed_ids(
    old: &FleetState,
    snapshots: &[RepoSnapshot],
    launchpad: &LaunchpadBundle,
) -> Vec<String> {
    let before: HashMap<&str, &RepoSnapshot> =
        old.snapshots.iter().map(|s| (s.id.0.as_str(), s)).collect();
    let after: HashMap<&str, &RepoSnapshot> =
        snapshots.iter().map(|s| (s.id.0.as_str(), s)).collect();
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for (id, snap) in &after {
        if before.get(id) != Some(snap) {
            ids.insert(id.to_string());
        }
    }
    for id in before.keys() {
        if !after.contains_key(id) {
            ids.insert(id.to_string());
        }
    }
    let entries: HashMap<&str, _> = old
        .launchpad
        .entries
        .iter()
        .map(|e| (e.id.as_str(), e))
        .collect();
    for entry in &launchpad.entries {
        if entries.get(entry.id.as_str()) != Some(&entry) {
            ids.insert(entry.id.clone());
        }
    }
    ids.into_iter().collect()
}

// ── The probe ───────────────────────────────────────────────────────────────

/// What is stamped in a repository's own git folder, besides the folder
/// itself. Git replaces `HEAD`, `index`, `packed-refs`, `config` and
/// `ORIG_HEAD` by writing a lock file and renaming it, and creates or removes
/// `MERGE_HEAD`, `rebase-merge/` and the like as an operation starts and ends:
/// all of that changes the folder's own modification time, so one `stat` of
/// the folder covers them. These are the files written in place (or
/// appended to), which do not touch the folder; `HEAD` and `index` are
/// stamped too, so a file system that keeps whole seconds still sees their
/// new inode.
const GIT_DIR_FILES: &[&str] = &["HEAD", "index", "FETCH_HEAD", "logs/HEAD"];

/// The same, in the folder a repository shares with its linked worktrees
/// (the stash log is appended to on `stash push`).
const COMMON_DIR_FILES: &[&str] = &["packed-refs", "logs/refs/stash"];

/// How deep the walk under `refs/` goes (`refs/remotes/origin/team/x` is 4).
const REFS_DEPTH: usize = 8;

/// Where one repository's state lives on disk, and what the probe stamps.
struct RepoFiles {
    id: Utf8PathBuf,
    git_dir: PathBuf,
    /// Everything stamped for this repository, in a fixed order: the git
    /// folder(s), [`GIT_DIR_FILES`], [`COMMON_DIR_FILES`] and every folder
    /// under `refs/` (listed when the probe is built, and again whenever
    /// this repository's state moves, so a new branch folder joins).
    stamps: Vec<PathBuf>,
}

/// The `stat`s that tell whether anything the fleet shows may have changed.
/// Built from a scanner and the repositories it found; [`Probe::read`] takes
/// a [`Fingerprint`] and reads no file contents (one small `commondir` or
/// `.git` pointer file per worktree is read when the probe is built).
pub struct Probe {
    config: Option<PathBuf>,
    known: Option<PathBuf>,
    /// Folders whose entries decide which repositories exist: each root and
    /// each repository's parent. A repository created or deleted beside the
    /// others, or directly under a root, changes one of them.
    folders: Vec<PathBuf>,
    repos: Vec<RepoFiles>,
    launchpad_home: Option<PathBuf>,
}

/// One probe's reading. Compare two with [`Fingerprint::diff`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fingerprint {
    config: u64,
    layout: u64,
    remembered: u64,
    launchpad: u64,
    repos: Vec<(Utf8PathBuf, u64)>,
}

/// What moved between two fingerprints.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// The config file.
    pub config: bool,
    /// The roots or the folders holding repositories: the set of
    /// repositories may have changed.
    pub layout: bool,
    /// The file a deep search remembers repositories in.
    pub remembered: bool,
    /// A repository's `.cohors/repo.json`, the focus board or the activity log.
    pub launchpad: bool,
    /// Repositories whose git state moved.
    pub repos: Vec<Utf8PathBuf>,
}

impl Diff {
    /// True when nothing moved.
    pub fn is_empty(&self) -> bool {
        !self.config && !self.layout && !self.remembered && !self.launchpad && self.repos.is_empty()
    }
}

impl Probe {
    /// A probe for `repos` (as found by `scanner`), the scanner's roots,
    /// config and remembered repositories, and the launchpad in
    /// `launchpad_home`.
    pub fn new(scanner: &Scanner, repos: &[Utf8PathBuf], launchpad_home: Option<&Path>) -> Self {
        let mut folders: BTreeSet<PathBuf> = scanner
            .root_dirs()
            .into_iter()
            .map(Utf8PathBuf::into_std_path_buf)
            .collect();
        // A root given as a glob (`~/code/*`): its fixed part is the folder
        // where a newly matching directory would appear.
        for root in scanner.roots() {
            if root.contains(['*', '?', '[']) {
                let fixed: PathBuf = Path::new(&root)
                    .components()
                    .take_while(|c| !c.as_os_str().to_string_lossy().contains(['*', '?', '[']))
                    .collect();
                folders.insert(fixed);
            }
        }
        for repo in repos {
            if let Some(parent) = repo.parent() {
                folders.insert(parent.as_std_path().to_path_buf());
            }
        }
        Self {
            config: scanner.config_file().map(|p| p.as_std_path().to_path_buf()),
            known: scanner.known_file().map(|p| p.as_std_path().to_path_buf()),
            folders: folders.into_iter().collect(),
            repos: repos.iter().map(repo_files).collect(),
            launchpad_home: launchpad_home.map(Path::to_path_buf),
        }
    }

    /// Take a reading.
    pub fn read(&self) -> Fingerprint {
        let config = {
            let mut h = DefaultHasher::new();
            if let Some(path) = &self.config {
                stamp(path, &mut h);
            }
            h.finish()
        };
        let layout = {
            let mut h = DefaultHasher::new();
            for folder in &self.folders {
                stamp(folder, &mut h);
            }
            h.finish()
        };
        let remembered = {
            let mut h = DefaultHasher::new();
            if let Some(known) = &self.known {
                stamp(known, &mut h);
            }
            h.finish()
        };
        let launchpad = {
            let mut h = DefaultHasher::new();
            for repo in &self.repos {
                stamp(&repo.id.as_std_path().join(".cohors/repo.json"), &mut h);
            }
            if let Some(home) = &self.launchpad_home {
                stamp(&home.join("board.json"), &mut h);
                let history = home.join("history");
                stamp(&history, &mut h);
                // Appending an event to today's log does not touch the
                // folder; the newest log file does move.
                if let Some(newest) = newest_entry(&history) {
                    stamp(&newest, &mut h);
                }
            }
            h.finish()
        };
        let repos = self
            .repos
            .iter()
            .map(|repo| (repo.id.clone(), repo_fingerprint(repo)))
            .collect();
        Fingerprint {
            config,
            layout,
            remembered,
            launchpad,
            repos,
        }
    }

    /// The repositories this probe covers.
    pub fn len(&self) -> usize {
        self.repos.len()
    }

    /// True when the probe covers no repository.
    pub fn is_empty(&self) -> bool {
        self.repos.is_empty()
    }

    /// List these repositories' git folders again: a new branch can add a
    /// folder under `refs/` (`team/` for `team/feature`) to stamp from now on.
    pub fn relist(&mut self, repos: &[Utf8PathBuf]) {
        for files in self.repos.iter_mut() {
            if repos.contains(&files.id) {
                *files = repo_files(&files.id);
            }
        }
    }

    /// Whether `repo`'s git folder still exists.
    fn has_git_dir(&self, repo: &Utf8PathBuf) -> bool {
        self.repos
            .iter()
            .find(|r| &r.id == repo)
            .is_some_and(|r| r.git_dir.join("HEAD").exists())
    }
}

impl Fingerprint {
    /// What moved between `self` (older) and `newer`.
    pub fn diff(&self, newer: &Fingerprint) -> Diff {
        let same_repos = self.repos.len() == newer.repos.len()
            && self
                .repos
                .iter()
                .zip(&newer.repos)
                .all(|((a, _), (b, _))| a == b);
        let repos = if same_repos {
            self.repos
                .iter()
                .zip(&newer.repos)
                .filter(|((_, a), (_, b))| a != b)
                .map(|((id, _), _)| id.clone())
                .collect()
        } else {
            Vec::new()
        };
        Diff {
            config: self.config != newer.config,
            // A different set of repositories is a layout change by definition.
            layout: self.layout != newer.layout || !same_repos,
            remembered: self.remembered != newer.remembered,
            launchpad: self.launchpad != newer.launchpad,
            repos,
        }
    }
}

/// Where `repo` keeps its git state: `.git` (a folder, or a file pointing at
/// one, for a linked worktree or a submodule), or the folder itself for a bare
/// repository. Linked worktrees share refs with their main repository, named
/// by a `commondir` file.
fn repo_files(repo: &Utf8PathBuf) -> RepoFiles {
    let path = repo.as_std_path();
    let dot_git = path.join(".git");
    let git_dir = match std::fs::metadata(&dot_git) {
        Ok(meta) if meta.is_dir() => dot_git,
        Ok(meta) if meta.is_file() => gitdir_pointer(&dot_git, path).unwrap_or(dot_git),
        _ => path.to_path_buf(),
    };
    let common_dir = std::fs::read_to_string(git_dir.join("commondir"))
        .ok()
        .map(|text| {
            let target = PathBuf::from(text.trim());
            if target.is_absolute() {
                target
            } else {
                git_dir.join(target)
            }
        })
        .unwrap_or_else(|| git_dir.clone());

    let mut stamps = vec![git_dir.clone()];
    stamps.extend(GIT_DIR_FILES.iter().map(|name| git_dir.join(name)));
    if common_dir != git_dir {
        stamps.push(common_dir.clone());
    }
    stamps.extend(COMMON_DIR_FILES.iter().map(|name| common_dir.join(name)));
    // The newer ref store keeps every ref in a few files under `reftable/`.
    let reftable = common_dir.join("reftable");
    if reftable.is_dir() {
        stamps.push(reftable);
    }
    // A loose ref is updated by writing `<ref>.lock` and renaming it over
    // the ref, which changes the folder holding it. So the folders under
    // `refs/` are stamped, not the (possibly thousands of) ref files.
    list_folders(&common_dir.join("refs"), REFS_DEPTH, &mut stamps);
    RepoFiles {
        id: repo.clone(),
        git_dir,
        stamps,
    }
}

/// The folder a `.git` file points at (`gitdir: <path>`), relative to `repo`.
fn gitdir_pointer(dot_git: &Path, repo: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(dot_git).ok()?;
    let target = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    let target = PathBuf::from(target);
    Some(if target.is_absolute() {
        target
    } else {
        repo.join(target)
    })
}

fn repo_fingerprint(repo: &RepoFiles) -> u64 {
    let mut h = DefaultHasher::new();
    for path in &repo.stamps {
        stamp(path, &mut h);
    }
    h.finish()
}

/// Push `dir` and every folder under it, `depth` levels down, onto `out`.
fn list_folders(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    out.push(dir.to_path_buf());
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut folders: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    // `read_dir` order is up to the file system; sort so the order is stable.
    folders.sort();
    for folder in folders {
        list_folders(&folder, depth - 1, out);
    }
}

/// The last entry of `dir` by name (the activity log names files by day).
fn newest_entry(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .max()
}

/// Hash what `stat` says about `path`: whether it exists, its size, its
/// modification time and (on Unix) its inode and change time. A file git
/// replaces by renaming gets a new inode, so even a file system that keeps
/// whole seconds sees the change.
fn stamp(path: &Path, h: &mut DefaultHasher) {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            1u8.hash(h);
            meta.len().hash(h);
            if let Ok(modified) = meta.modified() {
                modified.hash(h);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                meta.ino().hash(h);
                meta.ctime().hash(h);
                meta.ctime_nsec().hash(h);
            }
        }
        Err(_) => 0u8.hash(h),
    }
}
