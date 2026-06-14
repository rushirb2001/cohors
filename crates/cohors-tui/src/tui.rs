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
use std::io::Stdout;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use cohors_core::{RepoId, RepoRef, RepoSnapshot};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use crate::action;
use crate::app::{App, Cmd, CommandRun, ConfirmAction, Mode, RunResult, RunState};
use crate::scan::Scanner;
use crate::ui;

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Input poll timeout; also the spinner-animation cadence.
const POLL: Duration = Duration::from_millis(100);

/// Which background git action to run.
#[derive(Clone, Copy)]
enum ActionKind {
    Fetch,
    Pull,
    Stash,
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
    /// The rendered standup markdown for the current window.
    StandupReady(String),
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
}

/// Run the dashboard to completion, always restoring the terminal afterward.
pub fn run(scanner: Arc<Scanner>, use_cache: bool) -> Result<()> {
    let mut terminal = setup_terminal().context("setting up the terminal")?;
    let result = run_loop(&mut terminal, scanner, use_cache);
    let _ = restore_terminal(&mut terminal);
    result
}

fn run_loop(terminal: &mut Tui, scanner: Arc<Scanner>, use_cache: bool) -> Result<()> {
    let (tx, rx) = mpsc::channel::<BgMsg>();

    let mut app = App::new(scanner.roots(), scanner.config_path());
    // Warm start: paint cached snapshots instantly, then refresh in background.
    if use_cache
        && let Some(cached) = crate::cache::load()
        && !cached.is_empty()
    {
        app.set_repos(cached);
        app.status = Some("refreshing…".to_string());
    }
    app.scanning = true;
    spawn_scan(&scanner, tx.clone());

    // An in-flight bulk batch (fetch-all / bulk stash), for aggregate progress.
    let mut batch: Option<Batch> = None;
    // Monotonic id stamped on each command run, so a previous run's late
    // results are ignored once a new run starts.
    let mut run_seq: u64 = 0;

    loop {
        terminal.draw(|f| ui::render(f, &app, now_secs()))?;

        if event::poll(POLL)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match app.on_key(key) {
                    Cmd::Quit => break,
                    Cmd::Refresh => start_refresh(&mut app, &scanner, &tx),
                    Cmd::FetchSelected => {
                        start_action_targets(&mut app, &tx, ActionKind::Fetch, &mut batch)
                    }
                    Cmd::FetchAll => start_fetch_all(&mut app, &tx, &mut batch),
                    Cmd::PullSelected => {
                        start_action_targets(&mut app, &tx, ActionKind::Pull, &mut batch)
                    }
                    Cmd::CopyPath => copy_selected(&mut app),
                    Cmd::RevealFileManager => reveal_selected(&mut app),
                    Cmd::OpenEditor => open_editor(terminal, &mut app, &scanner, &tx)?,
                    Cmd::Lazygit => open_lazygit(terminal, &mut app, &scanner, &tx)?,
                    Cmd::OpenStandup | Cmd::StandupNextWindow => {
                        spawn_standup(&mut app, &scanner, &tx)
                    }
                    Cmd::CopyStandup => copy_standup(&mut app),
                    Cmd::RunCommand => start_command_run(&mut app, &tx, &mut run_seq),
                    Cmd::CopyRunOutput => copy_run_output(&mut app),
                    Cmd::ConfirmAccept => {
                        if let Some(pending) = app.confirm.take() {
                            match pending.action {
                                ConfirmAction::BulkStash(ids) => {
                                    start_bulk_stash(&mut app, &tx, ids, &mut batch)
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
        } else if app.scanning || !app.busy.is_empty() || run_in_progress(&app) {
            // Timed out with work in flight: animate the spinner.
            app.spinner = app.spinner.wrapping_add(1);
        }

        drain_background(&mut app, &rx, &mut batch, &scanner, &tx);
    }
    Ok(())
}

/// Apply any background results that have arrived.
fn drain_background(
    app: &mut App,
    rx: &mpsc::Receiver<BgMsg>,
    batch: &mut Option<Batch>,
    scanner: &Arc<Scanner>,
    tx: &Sender<BgMsg>,
) {
    while let Ok(msg) = rx.try_recv() {
        match msg {
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
                if app.status.as_deref() == Some("refreshing…") {
                    app.status = None;
                }
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
            BgMsg::StandupReady(markdown) => {
                app.standup = Some(markdown);
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
    let targets: Vec<(RepoId, Utf8PathBuf, String)> = app
        .action_targets()
        .iter()
        .filter_map(|id| app.repos.iter().find(|r| &r.id == id))
        .filter(|r| !r.has_error())
        .filter_map(|r| r.path.clone().map(|p| (r.id.clone(), p, r.name.clone())))
        .filter(|(id, _, _)| !app.busy.contains(id))
        .collect();
    if targets.is_empty() {
        app.status = Some("no repo selected".to_string());
        return;
    }
    let (doing, done) = match kind {
        ActionKind::Fetch => ("fetching", "fetched"),
        ActionKind::Pull => ("pulling", "pulled"),
        ActionKind::Stash => ("stashing", "stashed"),
    };
    if targets.len() > 1 {
        *batch = Some(Batch {
            total: targets.len(),
            doing,
            done,
        });
        app.status = Some(format!("{doing} {} repos…", targets.len()));
    } else {
        app.status = Some(format!("{doing} {}…", targets[0].2));
    }
    for (id, path, name) in targets {
        app.busy.insert(id.clone());
        spawn_action(tx.clone(), kind, id, path, name);
    }
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

fn open_editor(
    terminal: &mut Tui,
    app: &mut App,
    scanner: &Arc<Scanner>,
    tx: &Sender<BgMsg>,
) -> Result<()> {
    let Some(path) = selected_path(app) else {
        app.status = Some("no repo selected".to_string());
        return Ok(());
    };
    let Some(editor) = scanner.editor_command() else {
        app.status = Some("no editor configured (set `editor`, or $EDITOR/$VISUAL)".to_string());
        return Ok(());
    };
    let argv = action::editor_argv(&editor, &path);
    run_interactive_then_refresh(terminal, app, scanner, tx, &argv)
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
        cohors_github::enrich(&mut repos, Some(&token));
        let _ = tx.send(BgMsg::RemoteEnriched(repos));
    });
}

/// Collect the user's commits across all repos for the current window on a
/// worker thread, render them to markdown, and deliver the result.
fn spawn_standup(app: &mut App, scanner: &Arc<Scanner>, tx: &Sender<BgMsg>) {
    let Some(email) = scanner.author_email() else {
        app.standup = Some("Set `git config user.email` to generate a standup.".to_string());
        return;
    };
    let window = app.standup_window;
    let (since, until) = window.range(now_secs());
    let paths: Vec<Utf8PathBuf> = app.repos.iter().filter_map(|r| r.path.clone()).collect();
    let tx = tx.clone();
    std::thread::spawn(move || {
        let mut commits = Vec::new();
        for path in &paths {
            commits.extend(cohors_git::collect_commits(path, &email, since, until));
        }
        let markdown = cohors_core::to_markdown(&commits, window);
        let _ = tx.send(BgMsg::StandupReady(markdown));
    });
}

fn copy_standup(app: &mut App) {
    let Some(markdown) = app.standup.clone() else {
        return;
    };
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
    let Some(text) = app.run.as_ref().map(|r| r.copy_text()) else {
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

/// Run a git action on a worker thread, then re-snapshot the repo so the row
/// reflects the new ahead/behind, and report back.
fn spawn_action(tx: Sender<BgMsg>, kind: ActionKind, id: RepoId, path: Utf8PathBuf, name: String) {
    std::thread::spawn(move || {
        let outcome = match kind {
            ActionKind::Fetch => action::fetch(&path, &name),
            ActionKind::Pull => action::pull_ff(&path, &name),
            ActionKind::Stash => action::stash_push(&path, &name),
        };
        let message = match outcome {
            Ok(m) | Err(m) => m,
        };
        let snapshot = Some(Box::new(cohors_git::snapshot_repo(&RepoRef {
            id: id.clone(),
            path: Some(path),
        })));
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
    app.selected_repo().and_then(|r| r.path.clone())
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
