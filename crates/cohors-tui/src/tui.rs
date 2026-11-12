//! Terminal setup/teardown and the synchronous event loop (ADR-012).
//!
//! The loop polls input on the main thread; scans and fetch/pull run on
//! `std::thread`s that report back over an `mpsc` channel drained each tick, so
//! the UI never blocks on I/O. Interactive children (editor, lazygit) are run
//! by suspending the terminal while the loop isn't polling input — so they
//! own stdin cleanly — then resuming and refreshing.
//!
//! A panic hook restores the terminal before the panic prints.

use std::collections::VecDeque;
use std::io::{IsTerminal, Stdout};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use cohors_core::launchpad::LaunchpadEntry;
use cohors_core::{
    Overview, RemoteDetail, RepoDetail, RepoId, RepoRef, RepoSnapshot, StandupCommit,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEvent, KeyEventKind, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use crate::action;
use crate::app::{
    App, BulkVerb, Cmd, CommandRun, ConfirmAction, DetailView, LaunchpadView, Mode, OpenWith,
    Opener, PushPick, RunResult, RunState,
};
use crate::cli::Cli;
use crate::ground::{Fed, GroundReply};
use crate::scan::Scanner;
use crate::ui;

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Input poll timeout; also the spinner-animation cadence.
const POLL: Duration = Duration::from_millis(100);

/// Which background git action to run.
#[derive(Clone)]
enum ActionKind {
    Fetch,
    Pull,
    Push,
    /// Push one specific local branch (chosen in the picker) to origin, setting
    /// upstream when it was never pushed.
    PushBranch {
        branch: String,
        set_upstream: bool,
    },
    Stash,
    /// Commit with this message (the others carry no payload).
    Commit(String),
}

/// An in-flight bulk batch, for the aggregate progress/summary status line.
struct Batch {
    total: usize,
    /// Present participle for the in-progress message, e.g. "stashing".
    doing: &'static str,
    /// Past tense for the done message, e.g. "stashed".
    done: &'static str,
}

/// Messages from background threads to the loop.
enum BgMsg {
    Scanned(Vec<RepoSnapshot>),
    /// Snapshots with GitHub `remote` info filled in (v0.2). Merged by id.
    RemoteEnriched(Vec<RepoSnapshot>),
    /// The user's commits for the current standup window (grouped in the view).
    StandupReady(Vec<StandupCommit>),
    /// Every repo's launchpad entry plus the focus board.
    LaunchpadReady(Box<crate::app::LaunchpadView>),
    /// The overview model for the current snapshots, with the launchpad
    /// entries read alongside it. `seq` lets the loop drop a result that a
    /// newer request has already superseded.
    OverviewReady {
        seq: u64,
        overview: Box<Overview>,
        entries: Vec<LaunchpadEntry>,
    },
    /// The drill-in detail for a repo finished loading (matched by id).
    DetailReady {
        id: RepoId,
        detail: Box<RepoDetail>,
    },
    /// The detail pane's GitHub data (PRs + contributors) arrived (matched by id).
    RemoteDetailReady {
        id: RepoId,
        remote: Option<Box<RemoteDetail>>,
    },
    /// One repo in a command run finished (or failed to spawn). `run_id` lets
    /// the loop discard a previous run's late results.
    RunRepoDone {
        run_id: u64,
        id: RepoId,
        code: i32,
        stdout: String,
        stderr: String,
    },
    ActionDone {
        id: RepoId,
        message: String,
        // Boxed: a RepoSnapshot is large relative to the other variant.
        snapshot: Option<Box<RepoSnapshot>>,
    },
    /// A newer cohors was found, is installing, or is ready after a restart.
    Update(cohors_core::update::UpdateState),
}

/// What is known about the terminal's background.
#[derive(Debug, Clone, Copy)]
struct Ground {
    /// Whether to treat it as dark. A guess when `rgb` is `None`.
    dark: bool,
    /// Its actual colour, when the terminal told us or the user said which
    /// kind it is. `None` means "not known", and the dashboard then paints
    /// nothing that would only look right on one kind of background.
    rgb: Option<(u8, u8, u8)>,
}

/// The backgrounds assumed when the user says `dark` or `light` outright.
const ASSUMED_DARK: (u8, u8, u8) = (0x1C, 0x1B, 0x1A);
const ASSUMED_LIGHT: (u8, u8, u8) = (0xFF, 0xFF, 0xFF);

/// What the user's `theme` setting (or `COHORS_THEME`) says about the
/// background. `dark` or `light` settles it; `auto` leaves it not known until
/// the terminal answers [`ask_ground`].
fn ground_from_setting(mode: cohors_config::ThemeMode) -> Ground {
    use cohors_config::ThemeMode;
    match mode {
        ThemeMode::Dark => Ground {
            dark: true,
            rgb: Some(ASSUMED_DARK),
        },
        ThemeMode::Light => Ground {
            dark: false,
            rgb: Some(ASSUMED_LIGHT),
        },
        // Not known. `dark` is only a guess for the few colours that come as
        // a pair (a fill with its own text colour), which read either way.
        ThemeMode::Auto => Ground {
            dark: true,
            rgb: None,
        },
    }
}

/// Ask the terminal for its background colour and give it a moment to answer.
///
/// Call once the screen has been taken over (raw mode), and only when the
/// background is not already known. The answer comes back on the same stream
/// as keystrokes (see [`crate::ground`]). Most terminals answer within a few
/// milliseconds, so waiting briefly here means the first frame is already
/// drawn right. A terminal that is slower still gets its answer taken up by
/// the event loop when it arrives, and one that never answers costs only this
/// wait: silence is never read as light or as dark.
fn ask_ground(reply: &mut GroundReply) -> Option<(u8, u8, u8)> {
    use std::io::Write;
    const WAIT: Duration = Duration::from_millis(120);

    let mut out = std::io::stdout();
    if !out.is_terminal() || out.write_all(crate::ground::QUERY.as_bytes()).is_err() {
        return None;
    }
    let _ = out.flush();
    let asked = Instant::now();
    while let Some(left) = WAIT.checked_sub(asked.elapsed()) {
        if !event::poll(left).unwrap_or(false) {
            break;
        }
        if let Ok(Event::Key(key)) = event::read()
            && let Fed::Answer(colour) = reply.feed(&key)
        {
            return colour;
        }
    }
    None
}

/// Take `key` out of the stream if it is part of the terminal's (late) answer
/// about its background, and apply the answer when it completes. Returns
/// whether the key was taken, in which case it must not reach the dashboard.
fn took_ground_reply(reply: &mut GroundReply, key: &KeyEvent, app: &mut App) -> bool {
    match reply.feed(key) {
        Fed::NotPart => false,
        Fed::Taken => true,
        Fed::Answer(colour) => {
            // Only fill in what is not known: a `theme` setting stands.
            if app.ground.is_none()
                && let Some(colour) = colour
            {
                app.ground = Some(colour);
                app.dark = crate::ground::is_dark(colour);
            }
            true
        }
    }
}

/// The `theme` preference from the environment alone, for demo mode, which
/// has no config file.
fn theme_from_env() -> cohors_config::ThemeMode {
    match std::env::var("COHORS_THEME").ok().as_deref() {
        Some("dark") => cohors_config::ThemeMode::Dark,
        Some("light") => cohors_config::ThemeMode::Light,
        _ => cohors_config::ThemeMode::Auto,
    }
}

/// Run the dashboard to completion, always restoring the terminal afterward.
pub fn run(scanner: Arc<Scanner>, cli: &Cli, use_cache: bool, watch: bool) -> Result<()> {
    let ground = ground_from_setting(scanner.theme_mode());
    let mut terminal = setup_terminal().context("setting up the terminal")?;
    let result = run_loop(&mut terminal, scanner, cli, use_cache, watch, ground);
    let _ = restore_terminal(&mut terminal);
    // `R` after an update: start the new binary in this same terminal.
    if let Ok(true) = result {
        return Err(crate::update::restart());
    }
    result.map(|_| ())
}

/// How often `--watch` re-scans while the dashboard is idle.
const WATCH_INTERVAL: Duration = Duration::from_secs(5);

/// Launch the dashboard on the built-in demo fleet. Mirrors [`run`] but seeds
/// the app from `cohors_core::demo` and stubs every action, so nothing touches
/// the disk or network — a zero-config way to try cohors.
pub fn run_demo() -> Result<()> {
    let ground = ground_from_setting(theme_from_env());
    let mut terminal = setup_terminal().context("setting up the terminal")?;
    let result = run_demo_loop(&mut terminal, ground);
    let _ = restore_terminal(&mut terminal);
    result
}

fn run_demo_loop(terminal: &mut Tui, ground: Ground) -> Result<()> {
    // A fixed "now" so the sample commit ages stay exactly as designed for the
    // length of the session.
    let now = now_secs();
    let mut app = App::new(vec!["(demo)".to_string()], "(demo — no config)".to_string());
    app.dark = ground.dark;
    app.ground = ground.rgb;
    let mut ground_reply = GroundReply::default();
    if app.ground.is_none()
        && let Some(colour) = ask_ground(&mut ground_reply)
    {
        app.ground = Some(colour);
        app.dark = crate::ground::is_dark(colour);
    }
    seed_demo(&mut app, now);
    app.status = Some("demo mode — sample data; actions are stubbed".to_string());

    let mut toast_seen: Option<String> = app.status.clone();
    let mut toast_since = Instant::now();
    const TOAST_TTL: Duration = Duration::from_secs(6);

    loop {
        app.now = now;
        terminal.draw(|f| ui::render(f, &app, now))?;

        if event::poll(POLL)? {
            match event::read()? {
                // A late answer about the terminal's background arrives as
                // keys; it is taken out here and never reaches the dashboard.
                Event::Key(key)
                    if key.kind == KeyEventKind::Press
                        && took_ground_reply(&mut ground_reply, &key, &mut app) => {}
                Event::Key(key) if key.kind == KeyEventKind::Press => match app.on_key(key) {
                    Cmd::Quit => break,
                    // The demo never installs anything, so there is nothing to restart into.
                    Cmd::Restart => {}
                    // The standup works for real — seeded from demo commits.
                    Cmd::OpenStandup | Cmd::StandupNextWindow => {
                        app.show_standup(cohors_core::demo::standup(now));
                    }
                    Cmd::CopyStandup => copy_standup(&mut app),
                    Cmd::CopyStandupMarkdown => {
                        app.show_standup(cohors_core::demo::standup(now));
                        app.copy_standup_pending = false;
                        copy_standup(&mut app);
                    }
                    // The launchpad is seeded from sample data, like the standup.
                    Cmd::OpenLaunchpad => {
                        let (entries, board) = cohors_core::launchpad::demo();
                        show_launchpad(&mut app, LaunchpadView::new(entries, board));
                    }
                    // The command runner is simulated so its UI can be shown off.
                    Cmd::RunCommand => simulate_demo_run(&mut app),
                    Cmd::CopyRunOutput => copy_run_output(&mut app),
                    Cmd::CopyPath => copy_selected(&mut app),
                    // The detail pane works for real, seeded from demo data.
                    Cmd::OpenDetail => {
                        if let Some(repo) = app.selected_repo() {
                            let mut dv = DetailView::new(repo.id.clone(), repo.name.clone());
                            dv.detail = Some(cohors_core::demo::detail(now));
                            app.detail = Some(dv);
                            app.mode = Mode::Detail;
                        }
                    }
                    // Show the picker (a real feature worth demoing); set-default
                    // persists harmlessly; the launch itself is stubbed.
                    Cmd::OpenWith => {
                        open_with_picker(&mut app, cohors_config::prefs::default_editor());
                    }
                    Cmd::OpenWithSetDefault => set_open_with_default(&mut app),
                    Cmd::OpenWithAccept => {
                        app.mode = Mode::Normal;
                        app.open_with = None;
                        app.status =
                            Some("demo mode — install cohors to act on real repos".to_string());
                    }
                    // Everything that would touch real repos is a friendly no-op.
                    Cmd::Refresh
                    | Cmd::FetchSelected
                    | Cmd::FetchAll
                    | Cmd::PullSelected
                    | Cmd::PushSelected
                    | Cmd::PushBranchAccept
                    | Cmd::Act { .. }
                    | Cmd::Lazygit
                    | Cmd::ConfirmAccept => {
                        app.status =
                            Some("demo mode — install cohors to act on real repos".to_string());
                    }
                    // The demo fleet is never empty, so the rescue can't trigger.
                    Cmd::UseSuggestedRoots | Cmd::None => {}
                },
                Event::Mouse(m) => match m.kind {
                    MouseEventKind::ScrollUp => app.on_mouse_scroll(true),
                    MouseEventKind::ScrollDown => app.on_mouse_scroll(false),
                    _ => {}
                },
                _ => {}
            }
        } else {
            app.spinner = app.spinner.wrapping_add(1);
        }

        // Same auto-expiring toast behaviour as the live loop.
        if app.status != toast_seen {
            toast_seen = app.status.clone();
            toast_since = Instant::now();
        }
        if app.status.is_some() && toast_since.elapsed() >= TOAST_TTL {
            app.status = None;
            toast_seen = None;
        }
    }
    Ok(())
}

/// Fill `app` with the built-in sample fleet and its overview. Everything is
/// computed in memory by the pure core, so nothing touches the disk.
pub(crate) fn seed_demo(app: &mut App, now: i64) {
    let repos = cohors_core::demo::fleet(now);
    // The sample launchpad's entry ids match the sample fleet's repo ids
    // (a core test pins that), so the overview joins them directly.
    let (entries, board) = cohors_core::launchpad::demo();
    let overview = cohors_core::overview(&repos, &entries, &board, now);
    app.set_repos(repos);
    app.set_overview(overview, entries);
    app.scanned_at = Some(now);
}

/// Show a loaded launchpad, scrolled to the repository Enter was pressed on in
/// card 3 when there is one.
fn show_launchpad(app: &mut App, mut view: LaunchpadView) {
    if let Some(name) = app.launchpad_jump.take() {
        view.scroll = ui::launchpad_line_of(&view, &name).unwrap_or(0);
    }
    app.launchpad = Some(view);
}

/// Populate the command-run view with a fake successful run, so `cohors demo`
/// can show off the runner without executing anything.
fn simulate_demo_run(app: &mut App) {
    let cmd = app.command_input.trim().to_string();
    let results: Vec<RunResult> = app
        .action_targets()
        .into_iter()
        .map(|id| {
            let name = app
                .repos
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.name.clone())
                .unwrap_or_default();
            RunResult {
                id,
                name,
                state: RunState::Done {
                    code: 0,
                    stdout: format!("$ {cmd}\n(demo) ok — no command was actually run\n"),
                    stderr: String::new(),
                },
            }
        })
        .collect();
    app.run = Some(CommandRun::new(0, cmd, results));
    app.command_input.clear();
}

fn run_loop(
    terminal: &mut Tui,
    mut scanner: Arc<Scanner>,
    cli: &Cli,
    use_cache: bool,
    watch: bool,
    ground: Ground,
) -> Result<bool> {
    let (tx, rx) = mpsc::channel::<BgMsg>();
    // Set by `R`: the caller restarts cohors once the terminal is restored.
    let mut restart = false;

    let mut app = App::new(scanner.roots(), scanner.config_path());
    app.icons = scanner.icon_mode();
    app.dark = ground.dark;
    app.ground = ground.rgb;
    let mut ground_reply = GroundReply::default();
    if app.ground.is_none()
        && let Some(colour) = ask_ground(&mut ground_reply)
    {
        app.ground = Some(colour);
        app.dark = crate::ground::is_dark(colour);
    }
    // Stamped on every overview request, so a result that a newer request has
    // superseded is dropped when it arrives.
    let mut overview_seq: u64 = 0;
    // Warm start: paint cached snapshots instantly, then refresh in background.
    if use_cache
        && let Some(cached) = crate::cache::load()
        && !cached.is_empty()
    {
        app.set_repos(cached);
        app.status = Some("refreshing…".to_string());
        spawn_overview(&app, &tx, &mut overview_seq);
    }
    app.scanning = true;
    spawn_scan(&scanner, tx.clone());
    spawn_update_check(&scanner, tx.clone());

    // An in-flight bulk batch (fetch-all / bulk stash), for aggregate progress.
    let mut batch: Option<Batch> = None;
    // Monotonic id stamped on each command run, so a previous run's late
    // results are ignored once a new run starts.
    let mut run_seq: u64 = 0;

    // Track the status toast so it can auto-clear after a few idle seconds:
    // remember the last text we saw and when it appeared.
    let mut toast_seen: Option<String> = None;
    let mut toast_since = Instant::now();
    const TOAST_TTL: Duration = Duration::from_secs(4);

    // `--watch`: re-scan on an interval while idle, so the board stays current.
    let mut last_watch = Instant::now();

    loop {
        app.now = now_secs();
        terminal.draw(|f| ui::render(f, &app, app.now))?;

        if event::poll(POLL)? {
            match event::read()? {
                // A late answer about the terminal's background arrives as
                // keys; it is taken out here and never reaches the dashboard.
                Event::Key(key)
                    if key.kind == KeyEventKind::Press
                        && took_ground_reply(&mut ground_reply, &key, &mut app) => {}
                Event::Key(key) if key.kind == KeyEventKind::Press => match app.on_key(key) {
                    Cmd::Quit => break,
                    Cmd::Restart => {
                        restart = true;
                        break;
                    }
                    Cmd::Refresh => start_refresh(&mut app, &scanner, &tx),
                    Cmd::FetchSelected => {
                        start_action_targets(&mut app, &tx, ActionKind::Fetch, &mut batch)
                    }
                    Cmd::FetchAll => start_fetch_all(&mut app, &tx, &mut batch),
                    Cmd::PullSelected => {
                        start_action_targets(&mut app, &tx, ActionKind::Pull, &mut batch)
                    }
                    Cmd::PushSelected => start_push(&mut app, &tx, &mut batch),
                    // A "Do next" step: the overview already chose the targets.
                    Cmd::Act { verb, ids } => {
                        let kind = match verb {
                            BulkVerb::Fetch => ActionKind::Fetch,
                            BulkVerb::Pull => ActionKind::Pull,
                            BulkVerb::Push => ActionKind::Push,
                        };
                        start_action_on(&mut app, &tx, kind, ids, Targets::Exact, &mut batch)
                    }
                    Cmd::CopyStandupMarkdown => spawn_standup(&mut app, &scanner, &tx),
                    Cmd::PushBranchAccept => accept_push_branch(&mut app, &tx),
                    Cmd::CopyPath => copy_selected(&mut app),
                    Cmd::OpenDetail => open_detail(&mut app, &scanner, &tx),
                    Cmd::Lazygit => open_lazygit(terminal, &mut app, &scanner, &tx)?,
                    Cmd::OpenWith => open_with_picker(&mut app, resolve_default_editor(&scanner)),
                    Cmd::OpenWithAccept => accept_open_with(terminal, &mut app, &scanner, &tx)?,
                    Cmd::OpenWithSetDefault => set_open_with_default(&mut app),
                    Cmd::OpenStandup | Cmd::StandupNextWindow => {
                        spawn_standup(&mut app, &scanner, &tx)
                    }
                    Cmd::CopyStandup => copy_standup(&mut app),
                    Cmd::OpenLaunchpad => spawn_launchpad(&app, &tx),
                    Cmd::RunCommand => start_command_run(&mut app, &tx, &mut run_seq),
                    Cmd::CopyRunOutput => copy_run_output(&mut app),
                    Cmd::UseSuggestedRoots => use_suggested_roots(&mut app, &mut scanner, cli, &tx),
                    Cmd::ConfirmAccept => {
                        if let Some(pending) = app.confirm.take() {
                            match pending.action {
                                ConfirmAction::BulkStash(ids) => {
                                    start_bulk_stash(&mut app, &tx, ids, &mut batch)
                                }
                                ConfirmAction::BulkCommit { ids, message } => {
                                    start_bulk_commit(&mut app, &tx, ids, message, &mut batch)
                                }
                                ConfirmAction::AgentSetup(id) => {
                                    run_agent_setup(terminal, &mut app, &scanner, &tx, &id)?
                                }
                            }
                        }
                    }
                    Cmd::None => {}
                },
                // Mouse wheel / trackpad scroll (we capture the mouse).
                Event::Mouse(m) => match m.kind {
                    MouseEventKind::ScrollUp => app.on_mouse_scroll(true),
                    MouseEventKind::ScrollDown => app.on_mouse_scroll(false),
                    _ => {}
                },
                _ => {}
            }
        } else {
            // Timed out: advance the animation tick. Drives the busy spinner and
            // the blinking "synced" dot, so the dot blinks even when idle.
            app.spinner = app.spinner.wrapping_add(1);
        }

        drain_background(&mut app, &rx, &mut batch, &scanner, &tx, &mut overview_seq);

        // Auto-expire a result toast once nothing is in flight, so a stale
        // "fetched 3 repos" doesn't sit there forever. In-progress messages are
        // refreshed every frame, so their timer keeps resetting and they stay.
        if app.status != toast_seen {
            toast_seen = app.status.clone();
            toast_since = Instant::now();
        }
        let idle = !app.scanning && app.busy.is_empty() && !run_in_progress(&app);
        if idle && app.status.is_some() && toast_since.elapsed() >= TOAST_TTL {
            app.status = None;
            toast_seen = None;
        }

        // `--watch`: while idle and not mid-overlay, re-scan on the interval.
        if watch && idle && app.mode == Mode::Normal && last_watch.elapsed() >= WATCH_INTERVAL {
            start_refresh(&mut app, &scanner, &tx);
            last_watch = Instant::now();
        }
    }
    Ok(restart)
}

/// Roots auto-detected elsewhere that aren't already being searched — the
/// candidates the empty-state rescue offers. Bounded and shallow (see `detect`).
fn suggest_roots(current: &[String]) -> Vec<String> {
    let home = cohors_config::paths::home_dir().ok();
    cohors_fleet::detect_roots(home.as_deref())
        .into_iter()
        .filter(|cand| {
            let expanded = match &home {
                Some(h) => cohors_config::expand_tilde(cand, h),
                None => cand.clone(),
            };
            !current.iter().any(|c| c == &expanded || c == cand)
        })
        .collect()
}

/// Empty-state rescue: write the detected roots to the config (a confirmed
/// `init`), rebuild the scanner from it, and rescan — so the fleet appears
/// without restarting. On failure, keep the prompt so the user can retry.
fn use_suggested_roots(app: &mut App, scanner: &mut Arc<Scanner>, cli: &Cli, tx: &Sender<BgMsg>) {
    let roots = std::mem::take(&mut app.suggested_roots);
    let path = cli
        .config
        .clone()
        .map(camino::Utf8PathBuf::from)
        .or_else(|| cohors_config::paths::config_file().ok());
    let Some(path) = path else {
        app.status = Some("couldn't resolve the config path".to_string());
        app.suggested_roots = roots;
        return;
    };
    // Merge (never clobber): seeds the starter when no config exists, and
    // otherwise appends the detected roots while preserving the user's
    // comments, aliases, and groups.
    if let Err(err) = cohors_config::edit::add_roots(&path, &roots) {
        app.status = Some(format!("couldn't write config: {err}"));
        app.suggested_roots = roots;
        return;
    }
    match crate::scan::from_cli(cli) {
        Ok(rebuilt) => {
            *scanner = Arc::new(rebuilt);
            app.roots = scanner.roots();
            app.status = Some("config updated — scanning…".to_string());
            app.scanning = true;
            spawn_scan(scanner, tx.clone());
        }
        Err(err) => {
            app.status = Some(format!("rescan failed: {err}"));
            app.suggested_roots = roots;
        }
    }
}

/// Apply any background results that have arrived.
fn drain_background(
    app: &mut App,
    rx: &mpsc::Receiver<BgMsg>,
    batch: &mut Option<Batch>,
    scanner: &Arc<Scanner>,
    tx: &Sender<BgMsg>,
    overview_seq: &mut u64,
) {
    while let Ok(msg) = rx.try_recv() {
        match msg {
            BgMsg::Update(state) => app.update = Some(state),
            BgMsg::Scanned(mut repos) => {
                // Carry over already-fetched remote info so the Remote column
                // stays put across a refresh instead of blanking to "—" until
                // re-enrichment completes (enrichment will refresh it).
                for repo in &mut repos {
                    if repo.remote.is_none()
                        && let Some(old) = app.repos.iter().find(|r| r.id == repo.id)
                    {
                        repo.remote = old.remote.clone();
                    }
                }
                app.set_repos(repos);
                app.scanning = false;
                app.scanned_at = Some(now_secs());
                if app.status.as_deref() == Some("refreshing…") {
                    app.status = None;
                }
                // The overview is derived from the snapshots, so every scan
                // needs a new one.
                spawn_overview(app, tx, overview_seq);
                // Empty fleet? Offer a first-run rescue: repos detected elsewhere
                // that aren't already being searched (the empty-state picker).
                app.suggested_roots = if app.repos.is_empty() {
                    suggest_roots(&scanner.roots())
                } else {
                    Vec::new()
                };
                // Local data is painted; now fill in GitHub PR/CI in the
                // background (cached + rate-limit-aware — never blocks local).
                if let Some(token) = scanner.github_token() {
                    spawn_enrich(app.repos.clone(), token, tx.clone());
                }
            }
            BgMsg::RemoteEnriched(enriched) => {
                for snap in enriched {
                    if let Some(local) = app.repos.iter_mut().find(|r| r.id == snap.id) {
                        local.remote = snap.remote;
                    }
                }
                // Persist the enriched set so a warm start shows remote state
                // immediately instead of "—" until the next enrichment.
                crate::cache::save(&app.repos);
            }
            BgMsg::StandupReady(commits) => {
                app.show_standup(commits);
                // `y` in the commits card asked for the markdown, not the view.
                if app.copy_standup_pending {
                    app.copy_standup_pending = false;
                    copy_standup(app);
                }
            }
            BgMsg::LaunchpadReady(view) => {
                // Ignore a late result if the user already closed the view.
                if app.mode == Mode::Launchpad {
                    show_launchpad(app, *view);
                }
            }
            BgMsg::OverviewReady {
                seq,
                overview,
                entries,
            } => {
                // Only the newest request's answer describes the current fleet.
                if seq == *overview_seq {
                    app.set_overview(*overview, entries);
                }
            }
            BgMsg::DetailReady { id, detail } => {
                // Only apply if it's still the repo the pane is showing.
                if let Some(d) = &mut app.detail
                    && d.repo_id == id
                {
                    d.detail = Some(*detail);
                }
            }
            BgMsg::RemoteDetailReady { id, remote } => {
                if let Some(d) = &mut app.detail
                    && d.repo_id == id
                {
                    d.remote = remote.map(|b| *b);
                    d.remote_pending = false;
                }
            }
            BgMsg::RunRepoDone {
                run_id,
                id,
                code,
                stdout,
                stderr,
            } => {
                // Ignore results from a superseded run.
                if let Some(run) = &mut app.run
                    && run.run_id == run_id
                    && let Some(slot) = run.results.iter_mut().find(|r| r.id == id)
                {
                    slot.state = RunState::Done {
                        code,
                        stdout: cap_output(stdout),
                        stderr: cap_output(stderr),
                    };
                }
            }
            BgMsg::ActionDone {
                id,
                message,
                snapshot,
            } => {
                app.busy.remove(&id);
                if let Some(new_snapshot) = snapshot {
                    replace_snapshot(app, &id, *new_snapshot);
                    // The repository changed, so "Do next" and the figures did
                    // too. Wait for the last action of a batch before asking.
                    if app.busy.is_empty() {
                        spawn_overview(app, tx, overview_seq);
                    }
                }
                if let Some(b) = batch.as_ref() {
                    if app.busy.is_empty() {
                        app.status = Some(format!("{} {} repos", b.done, b.total));
                        *batch = None;
                    } else {
                        app.status = Some(format!("{}… {} left", b.doing, app.busy.len()));
                    }
                } else {
                    app.status = Some(message);
                }
            }
        }
    }
}

fn start_refresh(app: &mut App, scanner: &Arc<Scanner>, tx: &Sender<BgMsg>) {
    if app.scanning {
        return;
    }
    app.scanning = true;
    app.status = Some("refreshing…".to_string());
    spawn_scan(scanner, tx.clone());
}

/// Start a fetch/pull on the selected repo.
/// Run `kind` across the action target set (the marked selection, or the repo
/// under the cursor when nothing is marked) — so `f`/`p` act on the whole
/// selection. With a single target it keeps the per-repo status; with several it
/// shows an aggregate count.
fn start_action_targets(
    app: &mut App,
    tx: &Sender<BgMsg>,
    kind: ActionKind,
    batch: &mut Option<Batch>,
) {
    let ids = app.action_targets();
    start_action_on(app, tx, kind, ids, Targets::Selection, batch);
}

/// Where a list of action targets came from, which decides how much the loop
/// second-guesses it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Targets {
    /// The marked selection (or the cursor). A bulk push leaves out the
    /// repositories with nothing to push rather than reporting a failure each.
    Selection,
    /// Chosen by the overview for a "Do next" step: exactly the repositories
    /// that need the action, so none is left out. This is what lets a push
    /// publish branches that have no upstream yet.
    Exact,
}

/// Run `kind` over `ids` on the shared action pool, with the per-repo busy
/// markers and the aggregate status line.
fn start_action_on(
    app: &mut App,
    tx: &Sender<BgMsg>,
    kind: ActionKind,
    ids: Vec<RepoId>,
    source: Targets,
    batch: &mut Option<Batch>,
) {
    // Pull/stash/commit mutate the worktree, so a conflicted or mid-operation
    // repo is skipped with a reason (the same `worktree_block` rule the
    // MCP/web dispatch applies); fetch/push never touch the worktree.
    let guard_worktree = matches!(
        kind,
        ActionKind::Pull | ActionKind::Stash | ActionKind::Commit(_)
    );
    let mut targets: Vec<(RepoId, Utf8PathBuf, String)> = Vec::new();
    let mut skipped: Option<String> = None;
    let mut skipped_count = 0usize;
    // "Push all": on a big fleet most checked-out branches are local-only or up to
    // date, so pushing every one just fails with "no upstream". A *bulk* push targets
    // only repos whose current branch is ahead of its upstream and counts the rest
    // as "nothing to push", instead of firing a `git push` at each that reports back
    // a failure. A single-repo push still attempts, so its real git result shows
    // (and a specific branch is chosen through the picker, not here).
    let bulk_push =
        source == Targets::Selection && ids.len() > 1 && matches!(kind, ActionKind::Push);
    let mut nothing_count = 0usize;
    for id in ids {
        let Some(r) = app.repos.iter().find(|r| r.id == id) else {
            continue;
        };
        if r.has_error() || app.busy.contains(&r.id) {
            continue;
        }
        let Some(path) = r.path.clone() else { continue };
        if guard_worktree && let Some(reason) = cohors_actions::worktree_block(r) {
            skipped_count += 1;
            skipped = Some(format!("{}: {reason}", r.name));
            continue;
        }
        if bulk_push && r.upstream.as_ref().is_none_or(|u| u.ahead == 0) {
            nothing_count += 1;
            continue;
        }
        targets.push((r.id.clone(), path, r.name.clone()));
    }
    if targets.is_empty() {
        app.status = Some(match (skipped, nothing_count) {
            (Some(reason), _) => format!("skipped — {reason}"),
            (None, n) if n > 0 => format!("nothing to push — {n} up to date or local-only"),
            _ => "no repo selected".to_string(),
        });
        return;
    }
    let (doing, done) = match &kind {
        ActionKind::Fetch => ("fetching", "fetched"),
        ActionKind::Pull => ("pulling", "pulled"),
        ActionKind::Push | ActionKind::PushBranch { .. } => ("pushing", "pushed"),
        ActionKind::Stash => ("stashing", "stashed"),
        ActionKind::Commit(_) => ("committing", "committed"),
    };
    // Surface any guarded skips alongside the progress note.
    let skip_note = if skipped_count > 0 {
        format!("  ·  {skipped_count} skipped (conflict/operation)")
    } else if nothing_count > 0 {
        format!("  ·  {nothing_count} nothing to push")
    } else {
        String::new()
    };
    if targets.len() > 1 {
        *batch = Some(Batch {
            total: targets.len(),
            doing,
            done,
        });
        app.status = Some(format!("{doing} {} repos…{skip_note}", targets.len()));
    } else {
        app.status = Some(format!("{doing} {}…{skip_note}", targets[0].2));
    }
    for (id, path, name) in targets {
        app.busy.insert(id.clone());
        spawn_action(tx.clone(), kind.clone(), id, path, name);
    }
}

/// Push: a single multi-branch target asks *which* branch first (a repo can have
/// several local branches, and `git push` only pushes the checked-out one); a
/// single-branch repo or a bulk selection pushes each target's current branch
/// directly, as before.
fn start_push(app: &mut App, tx: &Sender<BgMsg>, batch: &mut Option<Batch>) {
    let targets = app.action_targets();
    if let [only] = targets.as_slice()
        && let Some(repo) = app.repos.iter().find(|r| &r.id == only)
        && repo.branches.len() > 1
        && !repo.has_error()
        && !app.busy.contains(&repo.id)
    {
        // Current branch pre-selected (Enter pushes the checkout by default).
        let cursor = repo.branches.iter().position(|b| b.is_current).unwrap_or(0);
        app.push_pick = Some(PushPick {
            id: repo.id.clone(),
            name: repo.name.clone(),
            branches: repo.branches.clone(),
            cursor,
        });
        app.mode = Mode::PushBranch;
        return;
    }
    start_action_targets(app, tx, ActionKind::Push, batch);
}

/// Push the branch highlighted in the picker. The checked-out branch (with an
/// upstream) goes through plain `git push` so it respects its configured remote;
/// any other branch — or a never-pushed one — pushes explicitly to origin, setting
/// upstream when needed.
fn accept_push_branch(app: &mut App, tx: &Sender<BgMsg>) {
    app.mode = Mode::Normal;
    let Some(pick) = app.push_pick.take() else {
        return;
    };
    let Some(branch) = pick.branches.get(pick.cursor).cloned() else {
        return;
    };
    let Some((id, name, path)) = app
        .repos
        .iter()
        .find(|r| r.id == pick.id)
        .and_then(|r| r.path.clone().map(|p| (r.id.clone(), r.name.clone(), p)))
    else {
        app.status = Some(format!("{}: no path", pick.name));
        return;
    };
    if app.busy.contains(&id) {
        return;
    }
    let kind = if branch.is_current && branch.has_upstream {
        ActionKind::Push
    } else {
        ActionKind::PushBranch {
            branch: branch.name.clone(),
            set_upstream: !branch.has_upstream,
        }
    };
    app.status = Some(format!("pushing {name} ({})…", branch.name));
    app.busy.insert(id.clone());
    spawn_action(tx.clone(), kind, id, path, name);
}

/// Start a fetch on every (readable) repo.
fn start_fetch_all(app: &mut App, tx: &Sender<BgMsg>, batch: &mut Option<Batch>) {
    let targets: Vec<(RepoId, Utf8PathBuf, String)> = app
        .repos
        .iter()
        .filter(|r| !r.has_error())
        .filter_map(|r| r.path.clone().map(|p| (r.id.clone(), p, r.name.clone())))
        .filter(|(id, _, _)| !app.busy.contains(id))
        .collect();

    if targets.is_empty() {
        app.status = Some("no repos to fetch".to_string());
        return;
    }
    *batch = Some(Batch {
        total: targets.len(),
        doing: "fetching",
        done: "fetched",
    });
    app.status = Some(format!("fetching {} repos…", targets.len()));
    for (id, path, name) in targets {
        app.busy.insert(id.clone());
        spawn_action(tx.clone(), ActionKind::Fetch, id, path, name);
    }
}

/// Stash changes across the given repos (post-confirmation) on the per-repo
/// busy/`ActionDone` path, so each row re-snapshots when done.
fn start_bulk_stash(
    app: &mut App,
    tx: &Sender<BgMsg>,
    ids: Vec<RepoId>,
    batch: &mut Option<Batch>,
) {
    let targets: Vec<(RepoId, Utf8PathBuf, String)> = ids
        .iter()
        .filter_map(|id| app.repos.iter().find(|r| &r.id == id))
        .filter(|r| !r.has_error())
        .filter_map(|r| r.path.clone().map(|p| (r.id.clone(), p, r.name.clone())))
        .filter(|(id, _, _)| !app.busy.contains(id))
        .collect();

    if targets.is_empty() {
        app.status = Some("no repos to stash".to_string());
        return;
    }
    *batch = Some(Batch {
        total: targets.len(),
        doing: "stashing",
        done: "stashed",
    });
    app.status = Some(format!("stashing {} repos…", targets.len()));
    for (id, path, name) in targets {
        app.busy.insert(id.clone());
        spawn_action(tx.clone(), ActionKind::Stash, id, path, name);
    }
}

/// Commit changes across the given repos (post-confirmation) on the per-repo
/// busy/`ActionDone` path, so each row re-snapshots when done. Mirrors
/// [`start_bulk_stash`]; every repo gets the same `message`.
fn start_bulk_commit(
    app: &mut App,
    tx: &Sender<BgMsg>,
    ids: Vec<RepoId>,
    message: String,
    batch: &mut Option<Batch>,
) {
    let targets: Vec<(RepoId, Utf8PathBuf, String)> = ids
        .iter()
        .filter_map(|id| app.repos.iter().find(|r| &r.id == id))
        .filter(|r| !r.has_error())
        .filter_map(|r| r.path.clone().map(|p| (r.id.clone(), p, r.name.clone())))
        .filter(|(id, _, _)| !app.busy.contains(id))
        .collect();

    if targets.is_empty() {
        app.status = Some("no repos to commit".to_string());
        return;
    }
    *batch = Some(Batch {
        total: targets.len(),
        doing: "committing",
        done: "committed",
    });
    app.status = Some(format!("committing {} repos…", targets.len()));
    for (id, path, name) in targets {
        app.busy.insert(id.clone());
        spawn_action(
            tx.clone(),
            ActionKind::Commit(message.clone()),
            id,
            path,
            name,
        );
    }
}

fn copy_selected(app: &mut App) {
    let Some(path) = selected_path(app) else {
        app.status = Some("no repo selected".to_string());
        return;
    };
    app.status = Some(match action::copy_to_clipboard(path.as_str()) {
        Ok(()) => format!("copied {path}"),
        Err(e) => format!("copy failed: {e}"),
    });
}

fn reveal_selected(app: &mut App) {
    let Some(path) = selected_path(app) else {
        app.status = Some("no repo selected".to_string());
        return;
    };
    if let Err(e) = action::reveal(&path) {
        app.status = Some(format!("open failed: {e}"));
    }
}

/// Open the drill-in detail pane for the current repo and kick off its loads:
/// local git facts always, plus GitHub PRs/contributors when the repo has a
/// GitHub remote and we have a token.
fn open_detail(app: &mut App, scanner: &Arc<Scanner>, tx: &Sender<BgMsg>) {
    let Some(repo) = app.selected_repo() else {
        app.status = Some("no repo selected".to_string());
        return;
    };
    let id = repo.id.clone();
    let name = repo.name.clone();
    let path = repo.path.clone();
    let remote_url = repo.remote_url.clone();
    let mut dv = DetailView::new(id.clone(), name);

    // Read the git facts off-thread; the pane shows a "loading" state until they
    // arrive (matched back by id, so a fast repo-switch can't show stale data).
    if let Some(path) = path {
        let tx = tx.clone();
        let id = id.clone();
        std::thread::spawn(move || {
            let detail = cohors_fleet::repo_detail(&path);
            let _ = tx.send(BgMsg::DetailReady {
                id,
                detail: Box::new(detail),
            });
        });
    }

    // Fetch GitHub PRs + contributors when there's a remote and a token.
    if let (Some(url), Some(token)) = (remote_url, scanner.github_token()) {
        dv.remote_pending = true;
        let tx = tx.clone();
        std::thread::spawn(move || {
            let remote = cohors_fleet::fetch_repo_detail(&token, &url).map(Box::new);
            let _ = tx.send(BgMsg::RemoteDetailReady { id, remote });
        });
    }

    app.detail = Some(dv);
    app.mode = Mode::Detail;
}

/// The default editor command: the user's saved pick first (set via the picker),
/// then config `editor` / `$EDITOR` / `$VISUAL`, then the first installed editor.
fn resolve_default_editor(scanner: &Arc<Scanner>) -> Option<String> {
    cohors_config::prefs::default_editor()
        .or_else(|| scanner.editor_command())
        .or_else(|| cohors_config::editors::first_detected_command().map(str::to_string))
}

/// Open the "Open with…" picker for the selected repo: detected editors, plus
/// "Reveal in folder" and lazygit (when installed). `default` is the command to
/// mark as the current default, if any.
fn open_with_picker(app: &mut App, default: Option<String>) {
    if selected_path(app).is_none() {
        app.status = Some("no repo selected".to_string());
        return;
    }
    let mut openers: Vec<Opener> = cohors_config::editors::detected()
        .into_iter()
        .map(|e| Opener::Editor {
            command: e.command.to_string(),
            label: e.label.to_string(),
        })
        .collect();
    openers.push(Opener::Reveal);
    if cohors_config::editors::installed("lazygit") {
        openers.push(Opener::Lazygit);
    }
    app.open_with = Some(OpenWith::new(openers, default));
    app.mode = Mode::OpenWith;
}

/// Run the picker's highlighted opener, then close the picker.
fn accept_open_with(
    terminal: &mut Tui,
    app: &mut App,
    scanner: &Arc<Scanner>,
    tx: &Sender<BgMsg>,
) -> Result<()> {
    let Some(path) = selected_path(app) else {
        app.mode = Mode::Normal;
        app.open_with = None;
        return Ok(());
    };
    app.mode = Mode::Normal;
    let Some(ow) = app.open_with.take() else {
        return Ok(());
    };
    let Some(opener) = ow.openers.into_iter().nth(ow.cursor) else {
        return Ok(());
    };
    match opener {
        Opener::Editor { command, .. } => {
            let argv = action::editor_argv(&command, &path);
            run_interactive_then_refresh(terminal, app, scanner, tx, &argv)
        }
        Opener::Reveal => {
            reveal_selected(app);
            Ok(())
        }
        Opener::Lazygit => {
            let argv = vec![
                "lazygit".to_string(),
                "-p".to_string(),
                path.as_str().to_string(),
            ];
            run_interactive_then_refresh(terminal, app, scanner, tx, &argv)
        }
    }
}

/// Remember the picker's highlighted editor as the default (persisted to prefs).
fn set_open_with_default(app: &mut App) {
    let chosen = {
        let Some(ow) = app.open_with.as_ref() else {
            return;
        };
        match ow.openers.get(ow.cursor) {
            Some(Opener::Editor { command, label }) => Some((command.clone(), label.clone())),
            _ => None,
        }
    };
    match chosen {
        Some((command, label)) => {
            cohors_config::prefs::set_default_editor(&command);
            if let Some(ow) = app.open_with.as_mut() {
                ow.default_command = Some(command);
            }
            app.status = Some(format!("default editor set to {label}"));
        }
        None => app.status = Some("pick an editor to set as default".to_string()),
    }
}

fn open_lazygit(
    terminal: &mut Tui,
    app: &mut App,
    scanner: &Arc<Scanner>,
    tx: &Sender<BgMsg>,
) -> Result<()> {
    let Some(path) = selected_path(app) else {
        app.status = Some("no repo selected".to_string());
        return Ok(());
    };
    let argv = vec![
        "lazygit".to_string(),
        "-p".to_string(),
        path.as_str().to_string(),
    ];
    run_interactive_then_refresh(terminal, app, scanner, tx, &argv)
}

/// Hand the terminal to the user's coding agent with the `setup-repo` prompt,
/// then come back and rescan.
///
/// This runs `cohors agent run setup-repo --repo <path>` — this same binary —
/// rather than calling the launch code directly, so the dashboard starts an
/// agent exactly the way the command line does (runner choice, prompt file,
/// activity log) with no second implementation to keep in step.
fn run_agent_setup(
    terminal: &mut Tui,
    app: &mut App,
    scanner: &Arc<Scanner>,
    tx: &Sender<BgMsg>,
    id: &RepoId,
) -> Result<()> {
    let Some(path) = app
        .repos
        .iter()
        .find(|r| &r.id == id)
        .and_then(|r| r.path.clone())
    else {
        app.status = Some("that repo has no local path".to_string());
        return Ok(());
    };
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            app.status = Some(format!("could not find the cohors binary: {e}"));
            return Ok(());
        }
    };
    suspend_terminal(terminal)?;
    let outcome = std::process::Command::new(exe)
        .args(["agent", "run", "setup-repo", "--repo", path.as_str()])
        .status();
    let started = matches!(&outcome, Ok(status) if status.success());
    if !started {
        // The reason (no runner installed, say) is on the plain screen right
        // now and would vanish when the dashboard repaints. Let it be read.
        eprintln!("\n  press Enter to return to cohors");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }
    resume_terminal(terminal)?;
    match outcome {
        Ok(_) if started => start_refresh(app, scanner, tx),
        Ok(_) => {
            app.status = Some(
                "the agent did not start; `cohors agent runners` shows what is installed"
                    .to_string(),
            );
        }
        Err(e) => app.status = Some(format!("could not start the agent: {e}")),
    }
    Ok(())
}

/// Suspend the TUI, run an interactive child to completion, then resume and
/// refresh (the child may have changed repo state).
fn run_interactive_then_refresh(
    terminal: &mut Tui,
    app: &mut App,
    scanner: &Arc<Scanner>,
    tx: &Sender<BgMsg>,
    argv: &[String],
) -> Result<()> {
    if argv.is_empty() {
        return Ok(());
    }
    suspend_terminal(terminal)?;
    let spawned = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .status();
    resume_terminal(terminal)?;

    match spawned {
        Ok(_) => start_refresh(app, scanner, tx),
        Err(e) => app.status = Some(format!("{}: {e}", argv[0])),
    }
    Ok(())
}

// ----- background workers ---------------------------------------------------

/// Look for a newer cohors off the UI thread and, in `auto` mode, install it.
/// Each step arrives as a message, so the header can say where it stands.
fn spawn_update_check(scanner: &Arc<Scanner>, tx: Sender<BgMsg>) {
    let mode = crate::update::effective_mode(scanner.updates_mode());
    std::thread::spawn(move || {
        crate::update::run_in_background(mode, now_secs(), |state| {
            let _ = tx.send(BgMsg::Update(state));
        });
    });
}

fn spawn_scan(scanner: &Arc<Scanner>, tx: Sender<BgMsg>) {
    let scanner = Arc::clone(scanner);
    std::thread::spawn(move || {
        let repos = scanner.scan();
        crate::cache::save(&repos);
        let _ = tx.send(BgMsg::Scanned(repos));
    });
}

/// Enrich snapshots with GitHub PR/CI on a worker thread, then deliver them for
/// merging. Runs after the local scan so the dashboard never waits on network.
fn spawn_enrich(mut repos: Vec<RepoSnapshot>, token: String, tx: Sender<BgMsg>) {
    std::thread::spawn(move || {
        cohors_fleet::enrich(&mut repos, Some(&token));
        let _ = tx.send(BgMsg::RemoteEnriched(repos));
    });
}

/// Collect the user's commits across all repos for the current window on a
/// worker thread, render them to markdown, and deliver the result.
fn spawn_standup(app: &mut App, scanner: &Arc<Scanner>, tx: &Sender<BgMsg>) {
    let Some(email) = scanner.author_email() else {
        app.status = Some("set `git config user.email` to use the standup".to_string());
        app.mode = Mode::Normal;
        app.copy_standup_pending = false;
        return;
    };
    let (since, until) = app.standup_window.range(now_secs());
    // The fleet facade walks every repo and tags each commit with the repo's
    // display name (its alias when one is set), so the standup names repos the
    // same way the dashboard does.
    let repos = app.repos.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        let commits = cohors_fleet::fleet_commits(&repos, &email, since, until);
        let _ = tx.send(BgMsg::StandupReady(commits));
    });
}

/// Read every repo's launchpad contribution and the focus board on a worker
/// thread (a few small file reads per repo), then deliver the view.
fn spawn_launchpad(app: &App, tx: &Sender<BgMsg>) {
    let repos = app.repos.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        // One shared read (the same one the web and MCP use), not a local copy.
        let home = cohors_fleet::launchpad::launchpad_home();
        let bundle = cohors_fleet::launchpad_bundle(&repos, home.as_deref());
        let view = LaunchpadView::new(bundle.entries, bundle.board);
        let _ = tx.send(BgMsg::LaunchpadReady(Box::new(view)));
    });
}

/// Compute the overview for the current snapshots on a worker thread. It
/// reads each repository's small launchpad file, which is why it stays off
/// the UI thread. The same thread reads the launchpad entries the detail
/// view's tasks card shows (task titles and run commands), which the overview
/// model itself only counts.
fn spawn_overview(app: &App, tx: &Sender<BgMsg>, seq: &mut u64) {
    *seq += 1;
    let seq = *seq;
    let repos = app.repos.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        let home = cohors_fleet::launchpad::launchpad_home();
        // The one overview every front-end shares; this crate only lays it out.
        let overview = cohors_fleet::overview(&repos, home.as_deref(), now_secs());
        let entries = cohors_fleet::launchpad_bundle(&repos, home.as_deref()).entries;
        let _ = tx.send(BgMsg::OverviewReady {
            seq,
            overview: Box::new(overview),
            entries,
        });
    });
}

fn copy_standup(app: &mut App) {
    let Some(view) = &app.standup else {
        return;
    };
    let markdown = cohors_core::to_markdown(&view.commits, app.standup_window);
    app.status = Some(match action::copy_to_clipboard(&markdown) {
        Ok(()) => "copied standup to clipboard".to_string(),
        Err(e) => format!("copy failed: {e}"),
    });
}

/// How many command-run worker threads run at once — bounds process/fd/RAM
/// pressure across a large fleet (ADR-020).
const RUN_CONCURRENCY: usize = 8;
/// Per-repo captured-output cap, so a noisy command can't balloon memory.
const MAX_OUTPUT: usize = 64 * 1024;

/// Dispatch the typed command across the action target set on a bounded pool of
/// worker threads pulling from a shared queue; each repo reports back with
/// `BgMsg::RunRepoDone` as its child exits (ADR-020).
fn start_command_run(app: &mut App, tx: &Sender<BgMsg>, run_seq: &mut u64) {
    let command = app.command_input.trim().to_string();
    let targets: Vec<(RepoId, Utf8PathBuf, String)> = app
        .action_targets()
        .iter()
        .filter_map(|id| app.repos.iter().find(|r| &r.id == id))
        .filter(|r| !r.has_error())
        .filter_map(|r| r.path.clone().map(|p| (r.id.clone(), p, r.name.clone())))
        .collect();
    if command.is_empty() || targets.is_empty() {
        app.status = Some("nothing to run".to_string());
        app.mode = Mode::Normal;
        return;
    }

    *run_seq += 1;
    let run_id = *run_seq;
    let results = targets
        .iter()
        .map(|(id, _, name)| RunResult {
            id: id.clone(),
            name: name.clone(),
            state: RunState::Running,
        })
        .collect();
    app.run = Some(CommandRun::new(run_id, command.clone(), results));

    let queue = Arc::new(Mutex::new(VecDeque::from(targets)));
    let workers = RUN_CONCURRENCY.min(queue.lock().unwrap().len());
    for _ in 0..workers {
        let queue = Arc::clone(&queue);
        let tx = tx.clone();
        let command = command.clone();
        std::thread::spawn(move || {
            // Pull the next repo off the shared queue until it's drained — keeps
            // all workers busy even when commands finish at different speeds.
            loop {
                let next = queue.lock().expect("run queue poisoned").pop_front();
                let Some((id, path, _name)) = next else { break };
                let (code, stdout, stderr) = action::run_command(&path, &command);
                let _ = tx.send(BgMsg::RunRepoDone {
                    run_id,
                    id,
                    code,
                    stdout,
                    stderr,
                });
            }
        });
    }
}

/// Copy the focused repo's command output to the clipboard.
fn copy_run_output(app: &mut App) {
    let Some(text) = app.run.as_ref().map(|r| r.focused_output()) else {
        return;
    };
    app.status = Some(match action::copy_to_clipboard(&text) {
        Ok(()) => "copied output to clipboard".to_string(),
        Err(e) => format!("copy failed: {e}"),
    });
}

/// Whether a command run still has repos in flight (drives the spinner).
fn run_in_progress(app: &App) -> bool {
    app.run.as_ref().is_some_and(|r| {
        r.results
            .iter()
            .any(|x| matches!(x.state, RunState::Running))
    })
}

/// Truncate captured output to `MAX_OUTPUT` bytes, on a char boundary.
fn cap_output(s: String) -> String {
    if s.len() <= MAX_OUTPUT {
        return s;
    }
    let mut end = MAX_OUTPUT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_string();
    out.push_str("\n… (truncated)");
    out
}

/// Run a git action on the shared bounded action pool (not one OS thread per
/// repo — "fetch all" on a 500-repo fleet must not open 500 concurrent
/// connections), then re-snapshot the repo so the row reflects the new
/// ahead/behind, and report back. Rows stream in as each repo finishes.
fn spawn_action(tx: Sender<BgMsg>, kind: ActionKind, id: RepoId, path: Utf8PathBuf, name: String) {
    cohors_actions::spawn_bounded(move || {
        let outcome = match kind {
            ActionKind::Fetch => action::fetch(&path, &name),
            ActionKind::Pull => action::pull_ff(&path, &name),
            ActionKind::Push => action::push(&path, &name),
            ActionKind::PushBranch {
                branch,
                set_upstream,
            } => action::push_branch(&path, &name, &branch, set_upstream),
            ActionKind::Stash => action::stash_push(&path, &name),
            ActionKind::Commit(message) => action::commit(&path, &name, &message),
        };
        let message = match outcome {
            Ok(m) | Err(m) => m,
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let snapshot = Some(Box::new(cohors_fleet::snapshot_repo(
            &RepoRef {
                id: id.clone(),
                path: Some(path),
            },
            now,
        )));
        let _ = tx.send(BgMsg::ActionDone {
            id,
            message,
            snapshot,
        });
    });
}

// ----- selection helpers ----------------------------------------------------

/// (id, path, name) of the selected repo, if it has a path.
fn selected_path(app: &App) -> Option<Utf8PathBuf> {
    app.current_repo().and_then(|r| r.path.clone())
}

/// Replace a repo's snapshot in place, preserving its (possibly aliased) name.
fn replace_snapshot(app: &mut App, id: &RepoId, mut new_snapshot: RepoSnapshot) {
    if let Some(i) = app.repos.iter().position(|r| &r.id == id) {
        new_snapshot.name = app.repos[i].name.clone();
        app.repos[i] = new_snapshot;
    }
}

// ----- terminal plumbing ----------------------------------------------------

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn setup_terminal() -> Result<Tui> {
    install_panic_hook();
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    // Capture the mouse so we handle scroll ourselves (the terminal otherwise
    // translates trackpad scroll into arrow keys, which we can't reverse).
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Tui) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// Hand the terminal back to the shell so a child process can use it.
fn suspend_terminal(terminal: &mut Tui) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    Ok(())
}

/// Re-take the terminal after a child exits, forcing a full repaint.
fn resume_terminal(terminal: &mut Tui) -> Result<()> {
    enable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    )?;
    terminal.clear()?;
    Ok(())
}

/// Restore the terminal before the default panic hook prints the message.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
        original(info);
    }));
}
