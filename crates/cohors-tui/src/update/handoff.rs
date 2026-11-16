//! Handing control to cohors' own install, the way Claude Code's native
//! install takes over from a package-manager one.
//!
//! The first run of a copy installed by Homebrew, pip, uv, npm, cargo or the
//! shell installer copies its own binary into `versions/<its version>/` (no
//! download, so it is instant and works offline) and links
//! `~/.local/bin/cohors` to it, unless something cohors did not make sits
//! there. Every run of such a copy then checks whether the managed copy's
//! active version is newer and, if so, runs that instead, with the same
//! arguments. The managed updater keeps that copy current, so whichever
//! `cohors` the PATH finds, the newest one runs.
//!
//! Never in a dev build, a build from source, CI, with `updates = "off"`,
//! `COHORS_NO_UPDATE` or `COHORS_DISABLE_UPDATES`; never twice in a row
//! (`COHORS_HANDOFF` is set in the handed-to process); and never to a binary
//! that fails its `--version` check (this copy runs instead, and `cohors
//! doctor` says why).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, anyhow};
use cohors_config::UpdatesMode;
use cohors_core::update::{
    Channel, HandoffFacts, Switches, hand_off_to, is_newer, set_up_notice, sets_up,
};
use serde::{Deserialize, Serialize};

use super::managed::{self, Busy, EXE, Layout, Lock};
use super::{Ctx, switches_given};

/// Set in the handed-to process; a run that sees it never hands off again.
pub(crate) const HANDOFF_VAR: &str = "COHORS_HANDOFF";
/// Where the handed-from copy lives, its version and how it was installed,
/// for `cohors doctor`.
const FROM_VAR: &str = "COHORS_HANDOFF_FROM";
const FROM_VERSION_VAR: &str = "COHORS_HANDOFF_FROM_VERSION";
const FROM_CHANNEL_VAR: &str = "COHORS_HANDOFF_FROM_CHANNEL";

/// What the hand-off remembers, in `<state>/cohors/handoff.json`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Record {
    /// The first run's setup, once it happened.
    #[serde(default)]
    pub set_up: Option<SetUp>,
    /// Whether the one-time notice about it has been shown.
    #[serde(default)]
    pub announced: bool,
    /// The last managed binary that passed its `--version` check, so the
    /// check runs once per version, not on every run.
    #[serde(default)]
    pub verified: Option<Verified>,
    /// The last time a hand-off fell back to the running copy, and why.
    #[serde(default)]
    pub fallback: Option<Fallback>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SetUp {
    pub at: i64,
    /// How the copy that did it was installed (its channel's label).
    pub from: String,
    pub version: String,
    /// The binary it copied.
    pub exe: String,
    /// Why the launcher was left alone, when it was.
    #[serde(default)]
    pub launcher_left: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Verified {
    pub version: String,
    pub size: u64,
    pub modified: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Fallback {
    pub at: i64,
    pub version: String,
    pub reason: String,
}

impl Record {
    pub fn load(layout: &Layout) -> Record {
        std::fs::read(layout.handoff_file())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, layout: &Layout) {
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = managed::write_atomically(&layout.handoff_file(), &json);
        }
    }
}

/// The circumstances of this run: from the process in normal use, made up in
/// tests.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Env {
    /// `COHORS_HANDOFF` is set: this run was handed control.
    pub handed_off: bool,
    /// A debug build.
    pub dev_build: bool,
    pub switches: Switches,
}

impl Env {
    pub fn from_process(configured: UpdatesMode) -> Env {
        Env {
            handed_off: std::env::var_os(HANDOFF_VAR).is_some_and(|v| !v.is_empty()),
            dev_build: cfg!(debug_assertions),
            switches: switches_given(configured, |name| std::env::var(name).ok()),
        }
    }
}

pub(crate) fn facts<'a>(ctx: &'a Ctx, env: &Env, active: Option<&'a str>) -> HandoffFacts<'a> {
    HandoffFacts {
        channel: &ctx.installed_as,
        dev_build: env.dev_build,
        switches: env.switches,
        handed_off: env.handed_off,
        current: &ctx.current,
        managed_active: active,
    }
}

/// Set this copy up as cohors' own install on its first run (when
/// `may_set_up`), then say which binary to hand control to, if any. A managed
/// binary that fails its check is recorded and not handed to.
pub(crate) fn prepare(ctx: &Ctx, env: &Env, may_set_up: bool, now: i64) -> Option<PathBuf> {
    let layout = &ctx.layout;
    let already = Record::load(layout).set_up.is_some();
    if may_set_up
        && sets_up(&facts(ctx, env, None), already)
        && let Err(err) = set_up(ctx, now)
    {
        // Another cohors holds the lock, or the copy failed: the next run
        // tries again.
        tracing::debug!("setting up cohors' own install: {err:#}");
    }
    let active = managed::active_version(layout);
    let version = hand_off_to(&facts(ctx, env, active.as_deref()))?;
    let binary = layout.versions().join(&version).join(EXE);
    match verify(layout, &version, &binary) {
        Ok(()) => Some(binary),
        Err(reason) => {
            fell_back(layout, &version, &reason, now);
            None
        }
    }
}

/// The first run's setup: copy the running binary into `versions/`, link the
/// launcher unless something else sits there, and remember it. Once, under
/// the update lock. `Ok(None)` when it had already been done.
pub(crate) fn set_up(ctx: &Ctx, now: i64) -> Result<Option<SetUp>> {
    let layout = &ctx.layout;
    let _lock = Lock::take(layout).ok_or(Busy)?;
    let mut record = Record::load(layout);
    if record.set_up.is_some() {
        return Ok(None);
    }
    let running = ctx
        .running
        .as_deref()
        .ok_or_else(|| anyhow!("cannot tell where this cohors is"))?;
    let version = ctx.current.as_str();
    let mut binary = layout.versions().join(version).join(EXE);
    if !binary.is_file() {
        binary = managed::place(layout, version, running)?;
    }
    // Link the launcher to this version, unless it is ours and already as new.
    let launcher_left = match managed::linked_version(layout) {
        Some(linked) if !is_newer(version, &linked) => None,
        _ => managed::link_unless_foreign(layout, &binary)?,
    };
    let done = SetUp {
        at: now,
        from: ctx.installed_as.label().to_string(),
        version: version.to_string(),
        exe: running.display().to_string(),
        launcher_left,
    };
    record.set_up = Some(done.clone());
    record.save(layout);
    Ok(Some(done))
}

/// Check the managed binary once per version (and per file: a replaced binary
/// is checked again). `Err` says why it may not be handed control.
fn verify(layout: &Layout, version: &str, binary: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(binary)
        .map_err(|err| format!("{} cannot be read: {err}", layout.show(binary)))?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    let stamp = Verified {
        version: version.to_string(),
        size: meta.len(),
        modified,
    };
    let mut record = Record::load(layout);
    if record.verified.as_ref() == Some(&stamp) {
        return Ok(());
    }
    managed::smoke_test(binary, version).map_err(|err| format!("{err:#}"))?;
    record.verified = Some(stamp);
    record.fallback = None;
    record.save(layout);
    Ok(())
}

/// Remember that a hand-off to `version` fell back to the running copy.
pub(crate) fn fell_back(layout: &Layout, version: &str, reason: &str, now: i64) {
    let mut record = Record::load(layout);
    record.fallback = Some(Fallback {
        at: now,
        version: version.to_string(),
        reason: reason.to_string(),
    });
    record.save(layout);
}

/// The one-time notice after the setup, on stderr.
pub(crate) fn announce_once(layout: &Layout) {
    let mut record = Record::load(layout);
    if record.set_up.is_some() && !record.announced {
        eprintln!("{}", set_up_notice(&layout.show(&layout.data)));
        record.announced = true;
        record.save(layout);
    }
}

/// Where this run was handed control from: the path, version and channel of
/// the copy the person ran.
pub(crate) fn handed_from() -> Option<(String, String, String)> {
    let var = |name| std::env::var(name).ok().filter(|v| !v.is_empty());
    Some((
        var(FROM_VAR)?,
        var(FROM_VERSION_VAR).unwrap_or_default(),
        var(FROM_CHANNEL_VAR).unwrap_or_default(),
    ))
}

/// Run `binary` in this process's place with the same arguments. On unix it
/// replaces this process; elsewhere it runs as a child with this process's
/// input and output, and its exit code becomes this one's. `also` is one more
/// variable for the started process (how an update at start says it happened).
/// Only returns if starting it failed.
pub(crate) fn exec(ctx: &Ctx, binary: &Path, also: Option<(&str, &str)>) -> std::io::Error {
    let mut command = Command::new(binary);
    command.args(std::env::args_os().skip(1));
    // A hand-off says where it came from. cohors' own copy starting a newer
    // one (after an update at start) is not a hand-off: it keeps whatever it
    // was itself handed from.
    if ctx.installed_as != Channel::Managed {
        command
            .env(HANDOFF_VAR, "1")
            .env(FROM_VERSION_VAR, &ctx.current)
            .env(FROM_CHANNEL_VAR, ctx.installed_as.label());
        if let Some(running) = &ctx.running {
            command.env(FROM_VAR, running);
        }
    }
    if let Some((name, value)) = also {
        command.env(name, value);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.exec()
    }
    #[cfg(not(unix))]
    {
        // The child inherits stdin, stdout and stderr.
        match command.status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(1)),
            Err(err) => err,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::update::lookup::Lookup;
    use crate::update::managed::Origin;
    use crate::update::managed::tests::installed;
    use cohors_core::update::Setting;
    use std::os::unix::fs::PermissionsExt;

    /// A Homebrew copy of cohors 0.7.7 in a temporary home.
    fn brew(root: &Path) -> Ctx {
        let layout = Layout::under(&root.join("home"));
        let exe = root.join("brew/Cellar/cohors/0.7.7/bin/cohors");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "#!/bin/sh\necho 'cohors 0.7.7'\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        Ctx {
            layout,
            installed_as: Channel::Homebrew,
            channel: Channel::Homebrew,
            lookup: Lookup {
                location: root.join("latest.txt").display().to_string(),
                source: None,
            },
            origin: Origin::Dir(root.join("releases")),
            current: "0.7.7".into(),
            running: Some(exe),
        }
    }

    fn env() -> Env {
        Env {
            handed_off: false,
            dev_build: false,
            switches: Switches {
                setting: Setting::Auto,
                disable_updates: false,
                no_update: false,
                ci: false,
            },
        }
    }

    #[test]
    fn the_first_run_sets_up_and_copies_once() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = brew(tmp.path());
        // Nothing newer to hand to: this copy runs, after setting up.
        assert_eq!(prepare(&ctx, &env(), true, 1_000), None);
        let copy = ctx.layout.versions().join("0.7.7").join(EXE);
        assert_eq!(
            std::fs::read(&copy).unwrap(),
            std::fs::read(ctx.running.as_ref().unwrap()).unwrap()
        );
        assert_eq!(
            managed::linked_version(&ctx.layout).as_deref(),
            Some("0.7.7")
        );
        let record = Record::load(&ctx.layout);
        let done = record.set_up.clone().unwrap();
        assert_eq!(
            (done.from.as_str(), done.version.as_str()),
            ("Homebrew", "0.7.7")
        );
        assert_eq!(done.launcher_left, None);
        assert!(!record.announced);
        // A second run copies nothing again, even if the binary changed.
        std::fs::write(ctx.running.as_ref().unwrap(), "changed").unwrap();
        assert_eq!(prepare(&ctx, &env(), true, 2_000), None);
        assert_ne!(std::fs::read(&copy).unwrap(), b"changed");
        assert_eq!(Record::load(&ctx.layout).set_up.unwrap().at, 1_000);
        // The notice is said once.
        announce_once(&ctx.layout);
        assert!(Record::load(&ctx.layout).announced);
    }

    #[test]
    fn a_launcher_that_is_not_ours_is_left_alone_and_remembered() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = brew(tmp.path());
        std::fs::create_dir_all(&ctx.layout.bin).unwrap();
        std::fs::write(ctx.layout.launcher(), "#!/usr/bin/env python3\n").unwrap();
        prepare(&ctx, &env(), true, 1_000);
        let done = Record::load(&ctx.layout).set_up.unwrap();
        assert!(done.launcher_left.unwrap().contains("did not put there"));
        assert_eq!(
            std::fs::read_to_string(ctx.layout.launcher()).unwrap(),
            "#!/usr/bin/env python3\n"
        );
        // The copy is still set up, and still the active version.
        assert_eq!(
            managed::active_version(&ctx.layout).as_deref(),
            Some("0.7.7")
        );
    }

    #[test]
    fn a_newer_managed_version_is_handed_to_and_an_older_one_is_not() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = brew(tmp.path());
        let older = installed(&ctx.layout, "0.7.6");
        managed::point_launcher(&ctx.layout, &older).unwrap();
        assert_eq!(prepare(&ctx, &env(), false, 1_000), None);
        let newer = installed(&ctx.layout, "0.7.8");
        managed::point_launcher(&ctx.layout, &newer).unwrap();
        assert_eq!(prepare(&ctx, &env(), false, 1_000), Some(newer.clone()));
        // Checked once; the check is remembered for that file.
        let verified = Record::load(&ctx.layout).verified.unwrap();
        assert_eq!(verified.version, "0.7.8");
        assert_eq!(prepare(&ctx, &env(), false, 2_000), Some(newer));
    }

    #[test]
    fn the_loop_guard_dev_builds_and_ci_never_hand_off_or_set_up() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = brew(tmp.path());
        let newer = installed(&ctx.layout, "0.7.8");
        managed::point_launcher(&ctx.layout, &newer).unwrap();
        let handed = Env {
            handed_off: true,
            ..env()
        };
        let dev = Env {
            dev_build: true,
            ..env()
        };
        let ci = Env {
            switches: Switches {
                ci: true,
                ..env().switches
            },
            ..env()
        };
        for blocked in [handed, dev, ci] {
            assert_eq!(prepare(&ctx, &blocked, true, 1_000), None, "{blocked:?}");
        }
        // None of them set anything up.
        assert!(Record::load(&ctx.layout).set_up.is_none());
        assert!(!ctx.layout.versions().join("0.7.7").exists());
    }

    #[test]
    fn a_bad_managed_binary_falls_back_and_says_why() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = brew(tmp.path());
        let bad = installed(&ctx.layout, "0.7.8");
        // It says it is another version.
        std::fs::write(&bad, "#!/bin/sh\necho 'cohors 0.6.0'\n").unwrap();
        managed::point_launcher(&ctx.layout, &bad).unwrap();
        assert_eq!(prepare(&ctx, &env(), false, 1_000), None);
        let fallback = Record::load(&ctx.layout).fallback.unwrap();
        assert_eq!(fallback.version, "0.7.8");
        assert!(fallback.reason.contains("0.6.0"), "{}", fallback.reason);
        // A good binary in its place is handed to, and the fallback is cleared.
        std::fs::write(&bad, "#!/bin/sh\necho 'cohors 0.7.8'\n").unwrap();
        assert_eq!(prepare(&ctx, &env(), false, 2_000), Some(bad));
        assert!(Record::load(&ctx.layout).fallback.is_none());
    }
}
