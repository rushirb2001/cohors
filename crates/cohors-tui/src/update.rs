//! Keeping cohors itself up to date: looking up the newest release where this
//! copy was installed from, installing it, and restarting into it.
//!
//! The pure decisions (comparing versions, recognising the install channel,
//! reading each source's answer, what to do, the notice's words) live in
//! `cohors_core::update`; this is the I/O around them. Everything is
//! best-effort: a lookup or install that fails is reported as a notice, never
//! as an error that stops cohors.
//!
//! When it looks:
//! - the dashboard, before it opens ([`start`]): a copy that updates through
//!   cohors' own install waits about a second to hear of a newer release,
//!   installs it and starts it, so the dashboard that opens is the new one;
//!   every other copy looks in the background, never delaying the dashboard;
//! - the dashboard again, every few minutes while it runs ([`watch`]);
//! - one-off commands, in the background; the notice is printed at exit
//!   ([`check_in_background`], [`print_notice`]), and never waited for;
//! - the MCP server, like the dashboard, but it only reports;
//! - `cohors update` and `cohors install`, at once.
//!
//! Where it asks is the channel's own source (the Homebrew tap, PyPI, npm,
//! crates.io, or GitHub releases for cohors' own install, the shell installer
//! and source builds), conditionally, so a look usually costs one bodiless
//! `304`. Launches closer together than [`CHECK_FLOOR`] share one answer.
//!
//! The test seam: `COHORS_UPDATE_SOURCE` points every lookup at a local file
//! or URL; `COHORS_UPDATE_RELEASES` points downloads at a local folder laid
//! out like the release host (`<base>/v<version>/<asset>`).
//!
//! A copy installed by Homebrew, pip, uv, npm, cargo or the shell installer
//! sets cohors' own install up on its first run and from then on hands
//! control to it whenever it is newer ([`start`], `handoff.rs`); such a copy
//! updates the managed copy, never its package manager.

mod doctor;
mod handoff;
mod lookup;
mod managed;
mod uninstall;

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use cohors_config::UpdatesMode;
use cohors_core::update::{
    Channel, Decision, Failure, InstallFacts, Known, Setting, Switches, UpdateState, Version,
    VersionSource, decide, detect_channel, is_newer, report_only, updates_through,
};
use serde::{Deserialize, Serialize};

pub use doctor::run as run_doctor;
use lookup::Lookup;
use managed::{Attempt, Busy, Layout, Lock, Origin};
pub use uninstall::run as uninstall_command;

/// This build's version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
pub(crate) const REPO: &str = "rushirb2001/cohors";
/// While the dashboard (or the MCP server) runs, look again this often.
const RECHECK_EVERY: Duration = Duration::from_secs(5 * 60);
/// Looks closer together than this (seconds) share one answer, so a burst of
/// one-off commands asks once.
pub(crate) const CHECK_FLOOR: i64 = 2 * 60;
/// How long the dashboard's start waits to hear whether a newer release is
/// out. Asking is usually one bodiless `304`, and launches less than
/// [`CHECK_FLOOR`] apart do not ask at all. A network slower than this is not
/// waited for: the look carries on behind the dashboard.
const ASK_WAIT: Duration = Duration::from_secs(1);
/// How long the start then waits for that release to download and pass its
/// check, before it opens the dashboard on this version instead.
const INSTALL_WAIT: Duration = Duration::from_secs(20);
/// While a release is being installed (by this cohors or another one), look
/// again this soon, so the header offers the restart as soon as it lands.
const RECHECK_WHILE_INSTALLING: Duration = Duration::from_secs(5);
/// Set in a cohors that was started right after an update at start; such a
/// run never updates at start again, so a release that somehow still reads as
/// older than the newest can never restart in a loop.
const UPDATED_AT_START_VAR: &str = "COHORS_UPDATED_AT_START";

// ── What cohors remembers between runs ──────────────────────────────────────

/// `<cache>/update.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Memo {
    /// What each source said last, keyed by where it was asked.
    #[serde(default)]
    pub sources: BTreeMap<String, Seen>,
    /// A version this machine already installed and is waiting for a restart.
    #[serde(default)]
    pub installed: Option<String>,
    /// The last install that failed. It is not tried again for a day, so a
    /// broken install never runs on every launch.
    #[serde(default)]
    pub failed: Option<Failure>,
    /// The version a restart was meant to start. If the cohors that comes up
    /// is still older, the install went somewhere this copy does not run from.
    #[serde(default)]
    pub restarted_into: Option<String>,
}

/// One source's last answer.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Seen {
    /// The newest version it listed.
    #[serde(default)]
    pub latest: Option<String>,
    /// Its ETag, sent back as `If-None-Match` next time.
    #[serde(default)]
    pub etag: Option<String>,
    /// When it was last asked (unix seconds).
    #[serde(default)]
    pub checked_at: i64,
    /// Why the last ask failed, if it did.
    #[serde(default)]
    pub error: Option<String>,
}

impl Memo {
    fn load(path: &Path) -> Memo {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) {
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = managed::write_atomically(path, &json);
        }
    }
}

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

// ── Whether updates run ─────────────────────────────────────────────────────

/// The mode in force for background checks: the config's, unless the
/// environment says to stay quiet (`COHORS_NO_UPDATE`, `COHORS_DISABLE_UPDATES`
/// or CI, each set to anything but `0`).
pub fn effective_mode(configured: UpdatesMode) -> UpdatesMode {
    mode_given(configured, |name| std::env::var(name).ok())
}

/// [`effective_mode`] with the environment passed in, so it can be tested.
fn mode_given(configured: UpdatesMode, env: impl Fn(&str) -> Option<String>) -> UpdatesMode {
    let switches = switches_given(configured, env);
    if switches.disable_updates || switches.no_update || switches.ci {
        UpdatesMode::Off
    } else {
        configured
    }
}

fn switches_given(configured: UpdatesMode, env: impl Fn(&str) -> Option<String>) -> Switches {
    let set = |name: &str| env(name).is_some_and(|v| !v.is_empty() && v != "0");
    Switches {
        setting: match configured {
            UpdatesMode::Auto => Setting::Auto,
            UpdatesMode::Notify => Setting::Notify,
            UpdatesMode::Off => Setting::Off,
        },
        disable_updates: set("COHORS_DISABLE_UPDATES"),
        no_update: set("COHORS_NO_UPDATE"),
        ci: set("CI"),
    }
}

/// `COHORS_DISABLE_UPDATES`: no updates at all, not even by hand.
fn disabled_by_env() -> bool {
    switches_given(UpdatesMode::Auto, |name| std::env::var(name).ok()).disable_updates
}

// ── This copy of cohors ─────────────────────────────────────────────────────

/// Everything a look needs: where things live, how this copy was installed,
/// how it updates, where to ask, and where releases come from.
pub(crate) struct Ctx {
    pub layout: Layout,
    /// How the running copy was installed.
    pub installed_as: Channel,
    /// How it updates: the same, except that a copy which hands off to cohors'
    /// own install updates that copy instead of running its package manager.
    pub channel: Channel,
    pub lookup: Lookup,
    pub origin: Origin,
    /// This build's version (a parameter so tests can pretend).
    pub current: String,
    /// The running binary, symlinks resolved.
    pub running: Option<PathBuf>,
}

impl Ctx {
    fn from_env() -> Option<Ctx> {
        let layout = Layout::from_env()?;
        let running = std::env::current_exe()
            .ok()
            .map(|path| path.canonicalize().unwrap_or(path));
        let exe = running
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let managed = std::fs::canonicalize(layout.versions())
            .unwrap_or(layout.versions())
            .to_string_lossy()
            .into_owned();
        let launcher = std::env::var("COHORS_INSTALLER").ok();
        let python = std::env::var("COHORS_PYTHON").ok();
        let installed_as = detect_channel(&InstallFacts {
            exe: &exe,
            launcher: launcher.as_deref(),
            python: python.as_deref(),
            managed_versions: Some(&managed),
            receipt: installer_owns(&layout, &exe, CURRENT),
            crates_io: cargo_installed(&layout, &exe, CURRENT),
        });
        // A copy that hands off (not a dev build, nothing in the environment
        // against it, the managed copy set up) updates the managed copy.
        let env = handoff::Env::from_process(UpdatesMode::Auto);
        let quiet = env.switches.disable_updates || env.switches.no_update || env.switches.ci;
        let handing_off =
            !env.dev_build && !quiet && !managed::installed_versions(&layout).is_empty();
        let channel = updates_through(&installed_as, handing_off);
        Some(Ctx {
            lookup: Lookup::for_source(channel.source()),
            origin: Origin::from_env(),
            current: CURRENT.to_string(),
            layout,
            installed_as,
            channel,
            running,
        })
    }
}

/// What the first lines of `main` pass to [`start`].
pub struct Start {
    /// The config's `updates` setting.
    pub configured: UpdatesMode,
    /// Whether this command may set cohors' own install up (not `doctor`,
    /// which only reads).
    pub set_up: bool,
    /// Never print the one-time notice (`cohors mcp`: stdout and stderr belong
    /// to the client).
    pub quiet: bool,
    /// The dashboard: install a newer release before it opens, so the one
    /// that opens is the new one. Never for a one-off command, a server or a
    /// hook, which must not wait on the network or print a line of their own.
    pub update_first: bool,
}

/// Before any command runs: on a copy installed by a package manager, set up
/// cohors' own install on the first run, then hand control to it when it is
/// newer. When this copy is the newest one installed and the command is the
/// dashboard, a release newer still is installed first and started in its
/// place ([`update_at_start`]). Returns only when this copy is the one to
/// run (nothing newer, a hand-off is not allowed, or the newer copy failed
/// its check or would not start, which is recorded for `cohors doctor`).
pub fn start(how: Start) {
    // Worked out here rather than through `ctx()`, which the rest of the run
    // works out after this, so it sees the copy the setup just made. In an
    // `Arc` because the look at start runs on a thread of its own.
    let Some(ctx) = Ctx::from_env() else { return };
    let ctx = Arc::new(ctx);
    let env = handoff::Env::from_process(how.configured);
    let now = now_secs();
    let target = handoff::prepare(&ctx, &env, how.set_up, now);
    if !how.quiet && std::io::stderr().is_terminal() {
        handoff::announce_once(&ctx.layout);
    }
    if let Some(binary) = target {
        // A newer copy is installed already. It runs, and it is the one that
        // looks for a release newer still: this older copy must not, or it
        // would install again what is already there.
        hand_over(&ctx, &binary, None, now);
        return;
    }
    // The line it prints and the dashboard both need a person at a terminal.
    let at_terminal = std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal();
    let again = std::env::var_os(UPDATED_AT_START_VAR).is_some_and(|v| !v.is_empty());
    if !(how.update_first && at_terminal && !again && updates_at_start(&ctx.channel, &env)) {
        return;
    }
    if update_at_start(&ctx) != AtStart::Installed {
        return;
    }
    // A package manager's copy hands off to what was just installed, checked
    // first as every hand-off is. cohors' own copy never hands off, so it
    // starts the new version itself (the install has just checked it).
    let newer = match ctx.installed_as {
        Channel::Managed => newer_active(&ctx),
        _ => handoff::prepare(&ctx, &env, false, now),
    };
    if let Some(binary) = newer {
        hand_over(&ctx, &binary, Some((UPDATED_AT_START_VAR, "1")), now);
    }
}

/// Start `binary` in this process's place. Only returns if that failed, with
/// why recorded for `cohors doctor`; this copy then runs.
fn hand_over(ctx: &Ctx, binary: &Path, also: Option<(&str, &str)>, now: i64) {
    let err = handoff::exec(ctx, binary, also);
    let version = binary
        .parent()
        .and_then(|dir| dir.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    handoff::fell_back(
        &ctx.layout,
        &version,
        &format!("starting {} failed: {err}", ctx.layout.show(binary)),
        now,
    );
}

// ── Updating before the dashboard opens ─────────────────────────────────────

/// Whether the dashboard's start brings this copy up to date first: only a
/// copy that updates through cohors' own install (one download, not a package
/// manager's upgrade, which can take minutes), with `updates = "auto"` and
/// nothing in the environment against it. Never a dev build.
fn updates_at_start(channel: &Channel, env: &handoff::Env) -> bool {
    let s = &env.switches;
    *channel == Channel::Managed
        && s.setting == Setting::Auto
        && !(s.disable_updates || s.no_update || s.ci)
        && !env.dev_build
}

/// What the look at start tells the thread that waits for it.
enum Heard {
    /// A newer release was found and its install has begun.
    Installing(String),
    /// The look is over; this is where the update stands.
    Done(Option<UpdateState>),
}

/// How bringing this copy up to date before the dashboard went.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AtStart {
    /// Nothing newer, or no answer in time: this version opens.
    Nothing,
    /// The newer version is installed, and is the one to start.
    Installed,
    /// The install failed: this version opens, and the header says so.
    Failed,
    /// The install is still going: this version opens, and the header offers
    /// the restart when it lands.
    StillInstalling,
}

/// Look for a newer release and install it, on a thread, and wait for it
/// within bounds: [`ASK_WAIT`] to hear, then [`INSTALL_WAIT`] to install. One
/// line on stderr says what is happening once an install begins, and how it
/// ended. Whatever is not waited for carries on behind the dashboard, whose
/// own looks ([`watch`]) pick the result up.
fn update_at_start(ctx: &Arc<Ctx>) -> AtStart {
    let (tx, heard) = mpsc::channel();
    let looking = Arc::clone(ctx);
    std::thread::spawn(move || {
        let said = tx.clone();
        let say = move |state: UpdateState| {
            if let UpdateState::Installing { version } = state {
                let _ = said.send(Heard::Installing(version));
            }
        };
        let state = look(&looking, UpdatesMode::Auto, true, now_secs(), &say);
        let _ = tx.send(Heard::Done(state));
    });
    let mut told = false;
    let outcome = await_look(&heard, ASK_WAIT, INSTALL_WAIT, &mut |version| {
        told = true;
        eprint!("cohors: updating to {version} before the dashboard opens…");
        let _ = std::io::stderr().flush();
    });
    if told {
        let current = &ctx.current;
        match outcome {
            AtStart::Installed => eprintln!(" done."),
            AtStart::Failed => {
                eprintln!(" it failed. Opening {current}; `cohors doctor` says why.")
            }
            AtStart::StillInstalling => eprintln!(
                " still downloading. Opening {current}; the dashboard offers the restart when it is ready."
            ),
            AtStart::Nothing => eprintln!(),
        }
    }
    outcome
}

/// Wait for the look at start: `ask_wait` to hear of a newer release, then
/// `install_wait` for its install. `tell` hears the version once, when an
/// install begins. Kept apart from the thread and the terminal so its timing
/// can be tested.
fn await_look(
    heard: &mpsc::Receiver<Heard>,
    ask_wait: Duration,
    install_wait: Duration,
    tell: &mut dyn FnMut(&str),
) -> AtStart {
    match heard.recv_timeout(ask_wait) {
        // Installed by an earlier run, and not the one running yet.
        Ok(Heard::Done(Some(UpdateState::Ready { .. }))) => AtStart::Installed,
        // Nothing newer, a release this copy only reports, or an install that
        // failed earlier (the header says so; no line for it here).
        Ok(Heard::Done(_)) => AtStart::Nothing,
        // No answer in time: the dashboard opens, the look carries on.
        Err(_) => AtStart::Nothing,
        Ok(Heard::Installing(version)) => {
            tell(&version);
            match heard.recv_timeout(install_wait) {
                Ok(Heard::Done(Some(UpdateState::Ready { .. }))) => AtStart::Installed,
                Ok(Heard::Done(Some(UpdateState::Failed { .. }))) => AtStart::Failed,
                // Another cohors holds the update lock, or it took too long.
                _ => AtStart::StillInstalling,
            }
        }
    }
}

/// The managed copy's active binary, when it is newer than the running one.
fn newer_active(ctx: &Ctx) -> Option<PathBuf> {
    let active = managed::active_version(&ctx.layout)?;
    if !is_newer(&active, &ctx.current) {
        return None;
    }
    let binary = ctx.layout.versions().join(&active).join(managed::EXE);
    binary.is_file().then_some(binary)
}

/// This process's context, worked out once.
fn ctx() -> Option<&'static Ctx> {
    static CTX: OnceLock<Option<Ctx>> = OnceLock::new();
    CTX.get_or_init(Ctx::from_env).as_ref()
}

/// The shell installer's receipts, newest name first. The installer was named
/// after the crate, which was `cohors-tui` up to 0.7.2 and is `cohors` since.
fn receipt_paths(layout: &Layout) -> [PathBuf; 2] {
    [
        layout
            .config_home
            .join("cohors")
            .join("cohors-receipt.json"),
        layout
            .config_home
            .join("cohors-tui")
            .join("cohors-tui-receipt.json"),
    ]
}

#[derive(Deserialize)]
struct Receipt {
    install_prefix: String,
    #[serde(default)]
    install_layout: String,
    #[serde(default)]
    version: String,
}

fn read_receipt(layout: &Layout) -> Option<Receipt> {
    receipt_paths(layout).into_iter().find_map(|path| {
        let bytes = std::fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    })
}

/// Did the shell installer put the binary at `exe`, at `version`, here? A
/// receipt alone is not enough: a source build of cohors can sit in the same
/// `~/.cargo/bin`. The binary has to be under the receipt's prefix, and be the
/// version it records.
pub(crate) fn installer_owns(layout: &Layout, exe: &str, version: &str) -> bool {
    read_receipt(layout).is_some_and(|receipt| {
        !receipt.install_prefix.is_empty()
            && exe.starts_with(&receipt.install_prefix)
            && receipt.version == version
    })
}

/// Did `cargo install cohors` put the binary at `exe`, at `version`, here,
/// from crates.io? Cargo records every install in `$CARGO_HOME/.crates2.json`,
/// keyed like `cohors 0.7.3 (registry+https://github.com/rust-lang/crates.io-index)`.
pub(crate) fn cargo_installed(layout: &Layout, exe: &str, version: &str) -> bool {
    let home = std::fs::canonicalize(&layout.cargo_home).unwrap_or(layout.cargo_home.clone());
    if !Path::new(exe).starts_with(home.join("bin")) {
        return false;
    }
    std::fs::read(home.join(".crates2.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|record| crates_io_install_of(&record, version))
}

/// Does cargo's install record list `cohors` at `version` from crates.io?
fn crates_io_install_of(record: &serde_json::Value, version: &str) -> bool {
    let wanted = format!("cohors {version} (registry+");
    record["installs"]
        .as_object()
        .is_some_and(|installs| installs.keys().any(|key| key.starts_with(&wanted)))
}

// ── One look ────────────────────────────────────────────────────────────────

/// One look: ask the channel's source, decide, and, when `installs`, install.
/// `say` hears "updating…" before an install starts; the outcome is returned.
/// Without `installs` (one-off commands, the MCP server) nothing is installed
/// and a version cohors would install is reported as `cohors update`'s to do.
pub(crate) fn look(
    ctx: &Ctx,
    mode: UpdatesMode,
    installs: bool,
    now: i64,
    say: &dyn Fn(UpdateState),
) -> Option<UpdateState> {
    if mode == UpdatesMode::Off {
        return None;
    }
    let memo_path = ctx.layout.memo();
    let mut memo = Memo::load(&memo_path);
    let answer = lookup::latest(&ctx.lookup, &mut memo, now, false);
    // A restart that came up new, or an installed version that is now the
    // one running, needs nothing more.
    let current = ctx.current.as_str();
    if memo
        .restarted_into
        .as_deref()
        .is_some_and(|v| !is_newer(v, current))
    {
        memo.restarted_into = None;
    }
    if memo
        .installed
        .as_deref()
        .is_some_and(|v| !is_newer(v, current))
    {
        memo.installed = None;
    }
    let decision = decide(
        &ctx.channel,
        mode == UpdatesMode::Auto,
        &Known {
            current,
            latest: answer.latest.as_deref(),
            installed: memo.installed.as_deref(),
            failed: memo.failed.as_ref(),
            restarted_into: memo.restarted_into.as_deref(),
            now,
        },
    );
    if let Decision::RestartMissed(failure) = &decision {
        memo.restarted_into = None;
        memo.failed = Some(failure.clone());
    }
    memo.save(&memo_path);
    if !installs {
        return report_only(decision, &ctx.channel);
    }
    match decision {
        Decision::Nothing => None,
        Decision::Report(state) => Some(state),
        Decision::RestartMissed(failure) => Some(UpdateState::Failed {
            version: failure.version,
            command: ctx.channel.manual_command().to_string(),
            reason: failure.reason,
        }),
        Decision::Install(version) => {
            say(UpdateState::Installing {
                version: version.clone(),
            });
            Some(install(ctx, &version, now))
        }
    }
}

/// Install `version` the way this copy was installed, remember how it went,
/// and say where it stands.
fn install(ctx: &Ctx, version: &str, now: i64) -> UpdateState {
    let result = match &ctx.channel {
        Channel::Managed => {
            managed::update_to(&ctx.layout, &ctx.origin, version, ctx.running.as_deref())
                .map(|_| ())
        }
        other => install_with_manager(&ctx.layout, other, version),
    };
    if let Err(err) = &result
        && err.downcast_ref::<Busy>().is_some()
    {
        // Another cohors is on it; its result shows up on the next look.
        return UpdateState::Installing {
            version: version.to_string(),
        };
    }
    let mut memo = Memo::load(&ctx.layout.memo());
    let reason = result.as_ref().err().map(|err| format!("{err:#}"));
    let state = match &reason {
        None => {
            memo.installed = Some(version.to_string());
            memo.failed = None;
            UpdateState::Ready {
                version: version.to_string(),
            }
        }
        Some(reason) => {
            memo.failed = Some(Failure {
                version: version.to_string(),
                at: now,
                reason: reason.clone(),
            });
            UpdateState::Failed {
                version: version.to_string(),
                command: ctx.channel.manual_command().to_string(),
                reason: reason.clone(),
            }
        }
    };
    memo.save(&ctx.layout.memo());
    managed::record(
        &ctx.layout,
        &Attempt {
            at: now,
            from: ctx.current.clone(),
            to: version.to_string(),
            channel: ctx.channel.label().to_string(),
            ok: reason.is_none(),
            reason,
        },
    );
    state
}

// ── When it looks ───────────────────────────────────────────────────────────

/// Look now and every few minutes after, on a background thread, for as long
/// as `report` accepts what it hears (the dashboard hangs up when it closes).
/// With `installs`, newer releases are installed (the dashboard); without, they
/// are only reported (the MCP server). Every change of state goes to `report`.
pub fn watch(
    mode: UpdatesMode,
    installs: bool,
    report: impl Fn(UpdateState) -> bool + Send + 'static,
) {
    if mode == UpdatesMode::Off {
        return;
    }
    let Some(ctx) = ctx() else { return };
    std::thread::spawn(move || {
        let mut last: Option<UpdateState> = None;
        loop {
            let say = |state: UpdateState| {
                let _ = report(state);
            };
            if let Some(state) = look(ctx, mode, installs, now_secs(), &say)
                && last.as_ref() != Some(&state)
            {
                if !report(state.clone()) {
                    return;
                }
                last = Some(state);
            }
            // An install under way (here, at start, or in another cohors)
            // lands in seconds, not minutes.
            let wait = match &last {
                Some(UpdateState::Installing { .. }) => RECHECK_WHILE_INSTALLING,
                _ => RECHECK_EVERY,
            };
            std::thread::sleep(wait);
        }
    });
}

/// A one-off command's look, running while the command does.
pub struct Pending {
    answer: Option<mpsc::Receiver<Option<UpdateState>>>,
    mode: UpdatesMode,
}

/// Start a one-off command's look in the background. Nothing starts when
/// updates are off or stderr is not a terminal (a script or an agent is
/// running the command, and the notice would land in its output).
pub fn check_in_background(configured: UpdatesMode) -> Pending {
    let mode = effective_mode(configured);
    let quiet = Pending { answer: None, mode };
    if mode == UpdatesMode::Off || !std::io::stderr().is_terminal() {
        return quiet;
    }
    let Some(ctx) = ctx() else { return quiet };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(look(ctx, mode, false, now_secs(), &|_| {}));
    });
    Pending {
        answer: Some(rx),
        mode,
    }
}

/// At a one-off command's end, one line on stderr when a newer cohors is
/// known: this run's answer if it came in time, otherwise what the last look
/// found. It never waits for the network.
pub fn print_notice(pending: Pending) {
    if let Some(state) = pending.state() {
        eprintln!("\n  {}", state.notice());
    }
}

impl Pending {
    /// Where the update stands: this run's answer if it came in time,
    /// otherwise what the last look found. It never waits for the network.
    pub fn state(self) -> Option<UpdateState> {
        let answer = self.answer?;
        answer.try_recv().unwrap_or_else(|_| remembered(self.mode))
    }
}

/// What the last look found, without asking anyone: for the MCP server's
/// answers, which never wait on the network for this.
pub fn remembered(mode: UpdatesMode) -> Option<UpdateState> {
    if mode == UpdatesMode::Off {
        return None;
    }
    let ctx = ctx()?;
    let memo = Memo::load(&ctx.layout.memo());
    let latest = memo
        .sources
        .get(&ctx.lookup.location)
        .and_then(|seen| seen.latest.clone());
    let decision = decide(
        &ctx.channel,
        mode == UpdatesMode::Auto,
        &Known {
            current: &ctx.current,
            latest: latest.as_deref(),
            installed: memo.installed.as_deref(),
            failed: memo.failed.as_ref(),
            restarted_into: memo.restarted_into.as_deref(),
            now: now_secs(),
        },
    );
    report_only(decision, &ctx.channel)
}

// ── The commands ────────────────────────────────────────────────────────────

/// `cohors update`: look now, and install a newer release unless `check_only`.
/// Run by hand, so it ignores the `updates` setting and `COHORS_NO_UPDATE`;
/// only `COHORS_DISABLE_UPDATES` stops it.
pub fn run_command(check_only: bool, json: bool) -> Result<()> {
    if disabled_by_env() {
        bail!("updates are switched off: COHORS_DISABLE_UPDATES is set");
    }
    let ctx = ctx().ok_or_else(|| anyhow!("could not find your home folder"))?;
    let now = now_secs();
    let memo_path = ctx.layout.memo();
    let mut memo = Memo::load(&memo_path);
    let answer = lookup::latest(&ctx.lookup, &mut memo, now, true);
    memo.save(&memo_path);
    let newer = answer.latest.clone().filter(|v| is_newer(v, &ctx.current));
    let command = ctx.channel.manual_command();
    if json {
        // `--check --json`: the answer for a script, nothing installed.
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "current": ctx.current,
                "channel": ctx.channel.label(),
                "source": ctx.lookup.label(),
                "newer": newer,
                "reached": answer.reached,
                "error": answer.error,
                "command": command,
            }))?
        );
        return Ok(());
    }
    println!(
        "cohors {}, installed with {}.",
        ctx.current,
        ctx.installed_as.label()
    );
    if ctx.channel != ctx.installed_as {
        println!(
            "It hands off to cohors' own install ({}), which is what updates.",
            ctx.layout.show(&ctx.layout.data)
        );
    }
    let Some(version) = newer else {
        match answer.error {
            Some(error) if !answer.reached => {
                println!(
                    "Could not ask {}: {error}. Try again later.",
                    ctx.lookup.label()
                )
            }
            _ => println!("This is the newest release on {}.", ctx.lookup.label()),
        }
        return Ok(());
    };
    // Why the dashboard's last background attempt failed, in full: the header
    // has room only for the short notice.
    if let Some(failure) = memo.failed.as_ref().filter(|f| f.version == version) {
        println!(
            "The last try to install {version} failed: {}",
            failure.reason
        );
    }
    if check_only || !ctx.channel.installs_itself() {
        let available = UpdateState::Available {
            version,
            command: command.to_string(),
        };
        println!("{}", available.notice());
        return Ok(());
    }
    let installing = UpdateState::Installing {
        version: version.clone(),
    };
    println!("{}", installing.notice());
    match install(ctx, &version, now) {
        UpdateState::Ready { version } => {
            // Nothing is left to restart: the next `cohors` is the new one.
            let mut memo = Memo::load(&memo_path);
            memo.installed = None;
            memo.save(&memo_path);
            println!("{}", UpdateState::Ready { version }.notice());
            Ok(())
        }
        UpdateState::Installing { .. } => {
            println!("Another cohors is updating right now; try again in a minute.");
            Ok(())
        }
        failed => Err(anyhow!(
            "{}",
            failed.detail().unwrap_or_else(|| failed.notice())
        )),
    }
}

/// `cohors install [version|latest]`: set up cohors' own self-updating copy
/// (or reinstall it), from whatever copy runs this, and say whether typing
/// `cohors` will run it.
pub fn install_command(version: Option<&str>) -> Result<()> {
    if disabled_by_env() {
        bail!("updates are switched off: COHORS_DISABLE_UPDATES is set");
    }
    let ctx = ctx().ok_or_else(|| anyhow!("could not find your home folder"))?;
    let layout = &ctx.layout;
    let now = now_secs();
    let version = match version.map(str::trim) {
        None | Some("latest") | Some("stable") => {
            let lookup = Lookup::for_source(VersionSource::GithubRelease);
            let mut memo = Memo::load(&layout.memo());
            let answer = lookup::latest(&lookup, &mut memo, now, true);
            memo.save(&layout.memo());
            answer.latest.ok_or_else(|| {
                anyhow!(
                    "could not find the newest release on {}: {}",
                    lookup.label(),
                    answer.error.unwrap_or_default()
                )
            })?
        }
        Some(asked) => Version::parse(asked)
            .map(|v| v.to_string())
            .ok_or_else(|| {
                anyhow!("{asked:?} is not a release version (like 0.7.8), `latest` or `stable`")
            })?,
    };
    println!("Installing cohors {version} as its own self-updating copy…");
    let result = managed::update_to(layout, &ctx.origin, &version, ctx.running.as_deref());
    if let Err(err) = &result
        && err.downcast_ref::<Busy>().is_some()
    {
        bail!("another cohors is updating right now; try again in a minute");
    }
    managed::record(
        layout,
        &Attempt {
            at: now,
            from: ctx.current.clone(),
            to: version.clone(),
            channel: Channel::Managed.label().to_string(),
            ok: result.is_ok(),
            reason: result.as_ref().err().map(|err| format!("{err:#}")),
        },
    );
    let done = result.with_context(|| format!("installing cohors {version}"))?;
    // A running dashboard of the managed copy can offer the restart.
    if ctx.channel == Channel::Managed && is_newer(&version, &ctx.current) {
        let mut memo = Memo::load(&layout.memo());
        memo.installed = Some(version.clone());
        memo.save(&layout.memo());
    }
    match &done.launcher_left {
        None => println!(
            "  {} → {}",
            layout.show(&done.launcher),
            layout.show(&done.binary)
        ),
        Some(why) => println!(
            "  {}\n  {why}, so it was left alone; every other cohors hands control to this one anyway.",
            layout.show(&done.binary)
        ),
    }
    if !done.pruned.is_empty() {
        println!("  Removed older versions: {}", done.pruned.join(", "));
    }
    if done.launcher_left.is_none() {
        println!("{}", doctor::path_advice(layout));
    }
    Ok(())
}

// ── Starting the new version ────────────────────────────────────────────────

/// Start cohors again, the same way it was started, so the updated binary
/// runs. Call it with the terminal restored. Only returns if that failed.
pub fn restart() -> anyhow::Error {
    let mut args = std::env::args_os();
    let first = args.next().map(PathBuf::from);
    // Updates through cohors' own install: start its active version, wherever
    // the PATH puts it. Otherwise: started by a path (`/opt/homebrew/bin/
    // cohors`, the pip package's binary), use it if it still exists; started
    // as plain `cohors`, let the PATH find the new one. After a Homebrew
    // upgrade the old versioned folder is gone, so a path that vanished also
    // falls back to the PATH.
    let active = ctx()
        .filter(|ctx| ctx.channel == Channel::Managed)
        .and_then(|ctx| managed::active_binary(&ctx.layout));
    let program = match (active, first) {
        (Some(active), _) => active,
        (None, Some(path)) if path.components().count() > 1 && path.exists() => path,
        _ => PathBuf::from("cohors"),
    };
    let mut command = Command::new(&program);
    command.args(args);
    // The update has been installed, so the next run should not announce it,
    // but it can check that it really is the new version.
    if let Some(ctx) = ctx() {
        let path = ctx.layout.memo();
        let mut memo = Memo::load(&path);
        memo.restarted_into = memo.installed.take();
        memo.save(&path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        anyhow!(command.exec()).context(format!("restarting {}", program.display()))
    }
    #[cfg(not(unix))]
    {
        match command.status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(0)),
            Err(err) => anyhow!(err).context(format!("restarting {}", program.display())),
        }
    }
}

// ── Package-manager installs ────────────────────────────────────────────────

/// Install `version` with the tool that installed this copy: Homebrew, pip,
/// or the release's shell installer. Holds the update lock, so two cohors
/// processes never run two installs at once.
fn install_with_manager(layout: &Layout, channel: &Channel, version: &str) -> Result<()> {
    let _lock = Lock::take(layout).ok_or(Busy)?;
    match channel {
        Channel::Homebrew => {
            // Homebrew refreshes taps only about once a day, so a plain
            // `upgrade` can find the old formula and change nothing. Refresh
            // first; if that fails (offline), the upgrade still gets its try.
            let _ = run_quietly(
                Command::new("brew")
                    .args(["update", "--quiet"])
                    .env("HOMEBREW_NO_ENV_HINTS", "1"),
            );
            run_quietly(
                Command::new("brew")
                    .args(["upgrade", "rushirb2001/tap/cohors"])
                    .env("HOMEBREW_NO_ENV_HINTS", "1"),
            )?;
            // "Nothing to upgrade" is a success to brew, so ask what it has.
            let listed = Command::new("brew")
                .args(["list", "--versions", "cohors"])
                .output()
                .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
                .unwrap_or_default();
            if !lists_version(&listed, version) {
                bail!(
                    "Homebrew does not have cohors {version} yet; try `brew update && brew upgrade cohors` later"
                );
            }
        }
        Channel::Pip { python } => {
            let python = python
                .clone()
                .or_else(python_beside_package)
                .ok_or_else(|| anyhow!("could not find the Python that installed cohors"))?;
            run_quietly(Command::new(python).args([
                "-m",
                "pip",
                "install",
                "--upgrade",
                "--disable-pip-version-check",
                "--quiet",
                &format!("cohors=={version}"),
            ]))?
        }
        Channel::Installer => run_installer(layout, version)?,
        _ => bail!("this copy of cohors does not update itself"),
    }
    Ok(())
}

/// Does `brew list --versions cohors` (e.g. `cohors 0.7.2 0.7.3`) include
/// `version`?
fn lists_version(listed: &str, version: &str) -> bool {
    listed.split_whitespace().skip(1).any(|v| v == version)
}

/// For a pip install, the interpreter next to the package: the binary sits at
/// `<prefix>/lib/pythonX.Y/site-packages/cohors/cohors`, so it is
/// `<prefix>/bin/python3`.
fn python_beside_package() -> Option<String> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let prefix = exe.ancestors().nth(5)?;
    ["python3", "python"]
        .iter()
        .map(|name| prefix.join("bin").join(name))
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

/// Run the release's own shell installer for `version`, quietly, into the
/// place the first install chose. It checks the download's checksum itself.
fn run_installer(layout: &Layout, version: &str) -> Result<()> {
    let url = format!("https://github.com/{REPO}/releases/download/v{version}/cohors-installer.sh");
    let script = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build()
        .get(&url)
        .set("User-Agent", &lookup::user_agent())
        .call()
        .context("downloading the installer")?
        .into_string()
        .context("reading the installer")?;
    // The script goes to `sh` on its standard input, exactly as `curl … | sh`
    // runs it. Nothing is written to disk, so there is no file another user
    // of the machine could plant or swap before it runs.
    let mut command = Command::new("sh");
    command
        .args(["-s", "--", "--quiet"])
        // The first install already put cohors on the PATH.
        .env("COHORS_NO_MODIFY_PATH", "1");
    // A first install into a chosen folder ("flat" layout) goes back there;
    // the default layout finds its own way to the same place.
    if let Some(receipt) = read_receipt(layout)
        && receipt.install_layout == "flat"
    {
        command.env("COHORS_INSTALL_DIR", &receipt.install_prefix);
    }
    run_with_input(&mut command, Some(script.as_bytes()))
}

/// Run a command with no input and its output kept, failing with the last few
/// lines of what it printed.
pub(crate) fn run_quietly(command: &mut Command) -> Result<()> {
    run_with_input(command, None)
}

/// [`run_quietly`], optionally feeding `input` to the command's standard input.
fn run_with_input(command: &mut Command, input: Option<&[u8]>) -> Result<()> {
    use std::io::Write;
    let program = command.get_program().to_owned();
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {program:?}"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin
            .write_all(input)
            .with_context(|| format!("handing the script to {program:?}"))?;
        // Dropping stdin closes it, so the shell sees the end of the script.
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("waiting for {program:?}"))?;
    if output.status.success() {
        return Ok(());
    }
    let printed = String::from_utf8_lossy(&output.stderr);
    let tail: Vec<&str> = printed.lines().rev().take(3).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    Err(anyhow!("{}", tail.join(" ").trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_homebrew_upgrade_counts_only_when_brew_lists_the_version() {
        assert!(lists_version("cohors 0.7.3\n", "0.7.3"));
        assert!(lists_version("cohors 0.7.2 0.7.3", "0.7.3"));
        // A stale tap upgrades nothing, and brew still says it succeeded.
        assert!(!lists_version("cohors 0.7.0\n", "0.7.3"));
        assert!(!lists_version("", "0.7.3"));
        assert!(!lists_version("cohors 0.7.30", "0.7.3"));
    }

    #[test]
    fn a_crates_io_install_is_recognised_by_name_and_version() {
        let record = serde_json::json!({
            "installs": {
                "cohors 0.7.3 (registry+https://github.com/rust-lang/crates.io-index)": {"bins": ["cohors"]},
                "ripgrep 14.1.1 (registry+https://github.com/rust-lang/crates.io-index)": {"bins": ["rg"]}
            }
        });
        assert!(crates_io_install_of(&record, "0.7.3"));
        // Another version, a git install or another crate is not this binary.
        assert!(!crates_io_install_of(&record, "0.7.2"));
        let from_git = serde_json::json!({
            "installs": {"cohors 0.7.3 (git+https://github.com/rushirb2001/cohors#abc)": {}}
        });
        assert!(!crates_io_install_of(&from_git, "0.7.3"));
        assert!(!crates_io_install_of(&serde_json::json!({}), "0.7.3"));
    }

    #[test]
    fn quiet_environments_switch_updates_off() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        assert_eq!(mode_given(UpdatesMode::Auto, env(&[])), UpdatesMode::Auto);
        assert_eq!(
            mode_given(UpdatesMode::Auto, env(&[("COHORS_NO_UPDATE", "1")])),
            UpdatesMode::Off
        );
        assert_eq!(
            mode_given(UpdatesMode::Auto, env(&[("COHORS_DISABLE_UPDATES", "1")])),
            UpdatesMode::Off
        );
        assert_eq!(
            mode_given(UpdatesMode::Auto, env(&[("CI", "true")])),
            UpdatesMode::Off
        );
        // `0` and empty mean "not set".
        assert_eq!(
            mode_given(
                UpdatesMode::Notify,
                env(&[("COHORS_NO_UPDATE", "0"), ("CI", "")])
            ),
            UpdatesMode::Notify
        );
        let switches = switches_given(UpdatesMode::Auto, env(&[("COHORS_DISABLE_UPDATES", "1")]));
        assert!(switches.disable_updates && !switches.no_update);
    }

    #[test]
    fn a_script_runs_from_standard_input_with_its_arguments() {
        // The installer's path: the script arrives on stdin, never on disk,
        // and still sees the arguments after `--`.
        let script = b"[ \"$1\" = --quiet ] || exit 7\nexit 0\n";
        run_with_input(
            Command::new("sh").args(["-s", "--", "--quiet"]),
            Some(script),
        )
        .unwrap();
        let err = run_with_input(
            Command::new("sh").args(["-s", "--", "--loud"]),
            Some(b"[ \"$1\" = --quiet ] || { echo wrong flag >&2; exit 7; }\n"),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "wrong flag");
    }

    #[test]
    fn a_failed_command_reports_its_last_words() {
        let err =
            run_quietly(Command::new("sh").args(["-c", "echo one >&2; echo two >&2; exit 3"]))
                .unwrap_err();
        assert_eq!(err.to_string(), "one two");
    }

    #[test]
    fn an_old_memo_still_loads() {
        // What 0.7.7 and earlier wrote: the once-a-day fields are ignored, the
        // failure carries over.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.json");
        std::fs::write(
            &path,
            r#"{"checked_at": 1794802321, "latest": "0.7.6", "installed": null,
                "failed": {"version": "0.7.6", "at": 1, "reason": "x"}, "restarted_into": null}"#,
        )
        .unwrap();
        let memo = Memo::load(&path);
        assert!(memo.sources.is_empty());
        assert_eq!(memo.failed.unwrap().version, "0.7.6");
    }

    fn at_start_env() -> handoff::Env {
        handoff::Env {
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
    fn only_cohors_own_copy_in_auto_updates_before_the_dashboard() {
        let env = at_start_env();
        assert!(updates_at_start(&Channel::Managed, &env));
        // A copy that was handed control is the managed copy, and still does.
        let handed = handoff::Env {
            handed_off: true,
            ..env
        };
        assert!(updates_at_start(&Channel::Managed, &handed));
        // A package manager's upgrade can take minutes, a source build is
        // exactly what was built, and the rest only say what to run.
        for channel in [
            Channel::Homebrew,
            Channel::Installer,
            Channel::Source,
            Channel::Npm,
            Channel::Cargo,
        ] {
            assert!(!updates_at_start(&channel, &env), "{channel:?}");
        }
        let dev = handoff::Env {
            dev_build: true,
            ..env
        };
        assert!(!updates_at_start(&Channel::Managed, &dev));
        let with = |switches: Switches| handoff::Env { switches, ..env };
        for switches in [
            Switches {
                setting: Setting::Notify,
                ..env.switches
            },
            Switches {
                setting: Setting::Off,
                ..env.switches
            },
            Switches {
                no_update: true,
                ..env.switches
            },
            Switches {
                disable_updates: true,
                ..env.switches
            },
            Switches {
                ci: true,
                ..env.switches
            },
        ] {
            assert!(
                !updates_at_start(&Channel::Managed, &with(switches)),
                "{switches:?}"
            );
        }
    }

    /// What the start hears, and what it then does. The waits are generous
    /// where an answer is already there, and tiny where none ever comes.
    #[test]
    fn the_start_waits_for_an_install_it_heard_begin_and_for_nothing_else() {
        let long = Duration::from_secs(30);
        let short = Duration::from_millis(20);
        let ready = || {
            Some(UpdateState::Ready {
                version: "0.7.8".into(),
            })
        };
        let failed = || {
            Some(UpdateState::Failed {
                version: "0.7.8".into(),
                command: "cohors update".into(),
                reason: "no network".into(),
            })
        };
        let installing = || Heard::Installing("0.7.8".into());
        // What was sent, the two waits, the outcome, and what was announced.
        let run = |sent: Vec<Heard>, ask: Duration, install: Duration| {
            let (tx, heard) = mpsc::channel();
            for message in sent {
                tx.send(message).unwrap();
            }
            let mut told = Vec::new();
            let outcome = await_look(&heard, ask, install, &mut |version| {
                told.push(version.to_string())
            });
            // `tx` is still alive here: a silent look is a slow one, not a
            // finished one.
            drop(tx);
            (outcome, told)
        };
        let nothing_told = Vec::<String>::new();

        // Nothing newer: straight on, nothing said.
        assert_eq!(
            run(vec![Heard::Done(None)], long, long),
            (AtStart::Nothing, nothing_told.clone())
        );
        // A release this copy only reports is the header's to say.
        let available = Some(UpdateState::Available {
            version: "0.7.8".into(),
            command: "cohors update".into(),
        });
        assert_eq!(
            run(vec![Heard::Done(available)], long, long).0,
            AtStart::Nothing
        );
        // No answer in time: the dashboard opens on this version.
        assert_eq!(
            run(vec![], short, long),
            (AtStart::Nothing, nothing_told.clone())
        );
        // Found, installed: announced once, and the new version starts.
        assert_eq!(
            run(vec![installing(), Heard::Done(ready())], long, long),
            (AtStart::Installed, vec!["0.7.8".to_string()])
        );
        // Found, and the install failed.
        assert_eq!(
            run(vec![installing(), Heard::Done(failed())], long, long),
            (AtStart::Failed, vec!["0.7.8".to_string()])
        );
        // Found, but the download outlasts the wait: open this version.
        assert_eq!(
            run(vec![installing()], long, short),
            (AtStart::StillInstalling, vec!["0.7.8".to_string()])
        );
        // Another cohors holds the update lock.
        let busy = Some(UpdateState::Installing {
            version: "0.7.8".into(),
        });
        assert_eq!(
            run(vec![installing(), Heard::Done(busy)], long, long).0,
            AtStart::StillInstalling
        );
        // Installed by an earlier run and not started yet: start it, with no
        // line about an install that is not happening.
        assert_eq!(
            run(vec![Heard::Done(ready())], long, long),
            (AtStart::Installed, nothing_told.clone())
        );
        // An install that failed earlier is not announced again at start.
        assert_eq!(
            run(vec![Heard::Done(failed())], long, long),
            (AtStart::Nothing, nothing_told)
        );
    }

    #[cfg(unix)]
    mod flow {
        use super::super::*;
        use crate::update::managed::tests::{fake_release, installed};
        use std::cell::RefCell;

        /// A context in a temporary home: the source is a local file saying
        /// `latest`, releases come from a local folder.
        fn context(root: &Path, channel: Channel, latest: &str) -> Ctx {
            let layout = Layout::under(&root.join("home"));
            let source = root.join("latest.txt");
            std::fs::write(&source, latest).unwrap();
            let running = (channel == Channel::Managed).then(|| installed(&layout, "0.7.7"));
            Ctx {
                layout,
                installed_as: channel.clone(),
                channel,
                lookup: Lookup {
                    location: source.display().to_string(),
                    source: None,
                },
                origin: Origin::Dir(root.join("releases")),
                current: "0.7.7".into(),
                running,
            }
        }

        fn look_now(
            ctx: &Ctx,
            installs: bool,
            now: i64,
        ) -> (Vec<UpdateState>, Option<UpdateState>) {
            let said = RefCell::new(Vec::new());
            let state = look(ctx, UpdatesMode::Auto, installs, now, &|s| {
                said.borrow_mut().push(s)
            });
            (said.into_inner(), state)
        }

        #[test]
        fn the_dashboard_says_updating_then_updated_and_installs_once() {
            let tmp = tempfile::tempdir().unwrap();
            fake_release(&tmp.path().join("releases"), "0.7.8", "0.7.8");
            let ctx = context(tmp.path(), Channel::Managed, "0.7.8\n");
            let (said, state) = look_now(&ctx, true, 1_000);
            assert_eq!(
                said,
                [UpdateState::Installing {
                    version: "0.7.8".into()
                }]
            );
            assert_eq!(
                state,
                Some(UpdateState::Ready {
                    version: "0.7.8".into()
                })
            );
            assert_eq!(
                managed::active_version(&ctx.layout).as_deref(),
                Some("0.7.8")
            );
            let attempt = managed::last_attempt(&ctx.layout).unwrap();
            assert!(attempt.ok && attempt.from == "0.7.7" && attempt.to == "0.7.8");
            // The next look (a new launch of the old copy) only offers the
            // restart; nothing is installed again.
            let (said, state) = look_now(&ctx, true, 1_000 + CHECK_FLOOR + 1);
            assert!(said.is_empty());
            assert!(matches!(state, Some(UpdateState::Ready { .. })));
            assert_eq!(managed::last_attempt(&ctx.layout).unwrap().at, 1_000);
        }

        #[test]
        fn a_failed_update_says_so_and_is_not_retried_in_a_loop() {
            let tmp = tempfile::tempdir().unwrap();
            // No release folder at all: the download fails.
            let ctx = context(tmp.path(), Channel::Managed, "0.7.8\n");
            let (_, state) = look_now(&ctx, true, 1_000);
            let Some(failed @ UpdateState::Failed { .. }) = state else {
                panic!("expected a failure, got {state:?}");
            };
            assert_eq!(failed.notice(), "Auto-update failed · run cohors doctor");
            assert!(failed.detail().unwrap().contains("cohors update"));
            let attempt = managed::last_attempt(&ctx.layout).unwrap();
            assert!(!attempt.ok && attempt.reason.is_some());
            // Ten minutes later: reported again, not attempted again.
            let (said, again) = look_now(&ctx, true, 1_000 + 600);
            assert!(said.is_empty());
            assert!(matches!(again, Some(UpdateState::Failed { .. })));
            assert_eq!(managed::last_attempt(&ctx.layout).unwrap().at, 1_000);
            // The running version is still the one that runs.
            assert_eq!(managed::installed_versions(&ctx.layout), ["0.7.7"]);
        }

        #[test]
        fn a_copy_that_hands_off_updates_the_managed_copy_not_its_package_manager() {
            let tmp = tempfile::tempdir().unwrap();
            fake_release(&tmp.path().join("releases"), "0.7.8", "0.7.8");
            let mut ctx = context(tmp.path(), Channel::Homebrew, "0.7.8\n");
            // The first run set the managed copy up from this Homebrew copy.
            let copy = installed(&ctx.layout, "0.7.7");
            managed::point_launcher(&ctx.layout, &copy).unwrap();
            ctx.channel = updates_through(&ctx.installed_as, true);
            assert_eq!(ctx.channel, Channel::Managed);
            // `brew` is never run: the managed copy is downloaded into place,
            // and the next run hands off to it.
            let (said, state) = look_now(&ctx, true, 1_000);
            assert_eq!(said.len(), 1);
            assert!(
                matches!(state, Some(UpdateState::Ready { .. })),
                "{state:?}"
            );
            assert_eq!(
                managed::active_version(&ctx.layout).as_deref(),
                Some("0.7.8")
            );
            assert_eq!(
                managed::last_attempt(&ctx.layout).unwrap().channel,
                "cohors install"
            );
        }

        #[test]
        fn a_one_off_look_never_installs() {
            let tmp = tempfile::tempdir().unwrap();
            fake_release(&tmp.path().join("releases"), "0.7.8", "0.7.8");
            let ctx = context(tmp.path(), Channel::Managed, "0.7.8\n");
            let (said, state) = look_now(&ctx, false, 1_000);
            assert!(said.is_empty());
            assert_eq!(
                state.map(|s| s.notice()).as_deref(),
                Some("Update available: 0.7.8 — run cohors update")
            );
            assert_eq!(managed::installed_versions(&ctx.layout), ["0.7.7"]);
        }

        #[test]
        fn a_told_only_channel_names_its_own_command() {
            let tmp = tempfile::tempdir().unwrap();
            let ctx = context(tmp.path(), Channel::Npm, "0.7.8\n");
            let (said, state) = look_now(&ctx, true, 1_000);
            assert!(said.is_empty());
            assert_eq!(
                state.map(|s| s.notice_in_dashboard()).as_deref(),
                Some("Update available: 0.7.8 — run npm i -g @cohors/cli@latest")
            );
            assert!(managed::last_attempt(&ctx.layout).is_none());
        }

        #[test]
        fn nothing_newer_says_nothing() {
            let tmp = tempfile::tempdir().unwrap();
            let ctx = context(tmp.path(), Channel::Managed, "0.7.7\n");
            assert_eq!(look_now(&ctx, true, 1_000), (Vec::new(), None));
        }

        #[test]
        fn a_newer_release_is_installed_before_the_dashboard_and_is_what_starts() {
            let tmp = tempfile::tempdir().unwrap();
            fake_release(&tmp.path().join("releases"), "0.7.8", "0.7.8");
            let ctx = Arc::new(context(tmp.path(), Channel::Managed, "0.7.8\n"));
            // Before: this version is the newest one installed.
            assert_eq!(newer_active(&ctx), None);
            assert_eq!(update_at_start(&ctx), AtStart::Installed);
            // The binary to start is the new version's, and the launcher
            // (what typing `cohors` runs) points at it too.
            let newer = ctx.layout.versions().join("0.7.8").join(managed::EXE);
            assert_eq!(newer_active(&ctx), Some(newer));
            assert_eq!(
                managed::linked_version(&ctx.layout).as_deref(),
                Some("0.7.8")
            );
            // The copy that then starts is 0.7.8: it finds nothing to do.
            let started = Ctx {
                current: "0.7.8".into(),
                ..context(tmp.path(), Channel::Managed, "0.7.8\n")
            };
            assert_eq!(update_at_start(&Arc::new(started)), AtStart::Nothing);
            assert_eq!(managed::last_attempt(&ctx.layout).unwrap().to, "0.7.8");
        }

        #[test]
        fn a_failed_update_at_start_opens_this_version() {
            let tmp = tempfile::tempdir().unwrap();
            // No release folder: the download fails.
            let ctx = Arc::new(context(tmp.path(), Channel::Managed, "0.7.8\n"));
            assert_eq!(update_at_start(&ctx), AtStart::Failed);
            assert_eq!(newer_active(&ctx), None);
            // The next start does not try again, and has nothing to announce:
            // the header says it failed.
            assert_eq!(update_at_start(&ctx), AtStart::Nothing);
        }

        #[test]
        fn nothing_newer_at_start_opens_this_version_at_once() {
            let tmp = tempfile::tempdir().unwrap();
            let ctx = Arc::new(context(tmp.path(), Channel::Managed, "0.7.7\n"));
            let began = std::time::Instant::now();
            assert_eq!(update_at_start(&ctx), AtStart::Nothing);
            // It answered; it did not sit out the wait.
            assert!(began.elapsed() < ASK_WAIT, "{:?}", began.elapsed());
            assert_eq!(newer_active(&ctx), None);
        }
    }
}
