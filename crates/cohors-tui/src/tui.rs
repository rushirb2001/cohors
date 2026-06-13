//! Terminal setup/teardown and the async event loop.
//!
//! Input (a blocking reader thread), scan results (a `spawn_blocking` worker),
//! and a timer tick all feed one channel; the loop updates [`App`] and redraws.
//! Per ADR-010 the heavy work runs off the UI: the scan never blocks input.
//!
//! A panic hook restores the terminal (leaves the alternate screen and raw
//! mode) *before* the panic prints — a corrupted terminal on crash is the #1
//! TUI papercut.

use std::io::Stdout;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use cohors_core::RepoSnapshot;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::app::{App, Cmd};
use crate::scan::Scanner;
use crate::ui;

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Everything that can wake the event loop.
enum AppEvent {
    Input(KeyEvent),
    Resize,
    Scanned(Vec<RepoSnapshot>),
}

/// Run the dashboard to completion, always restoring the terminal afterward.
pub async fn run(scanner: Arc<Scanner>) -> Result<()> {
    let mut terminal = setup_terminal().context("setting up the terminal")?;
    let result = run_loop(&mut terminal, scanner).await;
    // Restore even if the loop errored; surface the loop's result.
    let _ = restore_terminal(&mut terminal);
    result
}

async fn run_loop(terminal: &mut Tui, scanner: Arc<Scanner>) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    spawn_input_reader(tx.clone());

    let mut app = App::new(scanner.roots(), scanner.config_path());
    app.scanning = true;
    spawn_scan(&scanner, tx.clone());

    let mut ticker = tokio::time::interval(Duration::from_millis(120));
    terminal.draw(|f| ui::render(f, &app, now_secs()))?;

    loop {
        tokio::select! {
            maybe = rx.recv() => {
                let Some(ev) = maybe else { break };
                match ev {
                    AppEvent::Input(key) => match app.on_key(key) {
                        Cmd::Quit => break,
                        Cmd::Refresh => {
                            if !app.scanning {
                                app.scanning = true;
                                app.status = Some("refreshing…".to_string());
                                spawn_scan(&scanner, tx.clone());
                            }
                        }
                        Cmd::None => {}
                    },
                    AppEvent::Resize => {}
                    AppEvent::Scanned(repos) => {
                        app.set_repos(repos);
                        app.scanning = false;
                        app.status = None;
                    }
                }
            }
            _ = ticker.tick() => {
                if app.scanning {
                    app.spinner = app.spinner.wrapping_add(1);
                }
            }
        }
        terminal.draw(|f| ui::render(f, &app, now_secs()))?;
    }
    Ok(())
}

/// Run a scan on a blocking worker and deliver the result to the loop.
fn spawn_scan(scanner: &Arc<Scanner>, tx: UnboundedSender<AppEvent>) {
    let scanner = Arc::clone(scanner);
    tokio::task::spawn_blocking(move || {
        let repos = scanner.scan();
        let _ = tx.send(AppEvent::Scanned(repos));
    });
}

/// Read terminal events on a dedicated thread (crossterm's `read` blocks).
fn spawn_input_reader(tx: UnboundedSender<AppEvent>) {
    std::thread::spawn(move || {
        loop {
            match event::read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    if tx.send(AppEvent::Input(key)).is_err() {
                        break; // receiver gone: app is exiting
                    }
                }
                Ok(Event::Resize(_, _)) => {
                    if tx.send(AppEvent::Resize).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
}

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
    execute!(stdout, EnterAlternateScreen)?;
    let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Tui) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Install a panic hook that restores the terminal before the default hook
/// prints the panic message.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        original(info);
    }));
}
