//! `cohors` — the terminal dashboard binary.
//!
//! Parses the CLI, sets up file logging, and dispatches to a subcommand (or the
//! dashboard when invoked bare).
#![forbid(unsafe_code)]

mod action;
mod app;
mod cache;
mod cli;
mod command;
mod commands;
mod glyphs;
mod logging;
mod scan;
mod tui;
mod ui;

use clap::{CommandFactory, FromArgMatches};

use cli::{Cli, Command};

/// clap's derive `parse()`, plus the branded masthead and "start here" footer.
/// Those are attached here rather than as `#[command(…)]` attributes because
/// whether to paint them depends on the terminal, which only exists at runtime.
fn parse_cli() -> Cli {
    let (before, after) = commands::help_decorations();
    let matches = Cli::command()
        .before_help(before)
        .after_help(after)
        .help_template(cli::HELP_TEMPLATE)
        .get_matches();
    Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())
}

fn main() -> anyhow::Result<()> {
    let cli = parse_cli();
    logging::init();

    match &cli.command {
        Some(Command::Init { force }) => commands::init(&cli, *force),
        Some(Command::Scan { select, json }) => commands::scan(&cli, select.as_deref(), *json),
        Some(Command::Demo) => commands::run_demo(),
        Some(Command::Fetch(a)) => commands::run_action(
            &cli,
            commands::CliAction::Fetch,
            a.select.as_deref(),
            a.dry_run,
        ),
        Some(Command::Pull(a)) => commands::run_action(
            &cli,
            commands::CliAction::Pull,
            a.select.as_deref(),
            a.dry_run,
        ),
        Some(Command::Push(a)) => commands::run_action(
            &cli,
            commands::CliAction::Push,
            a.select.as_deref(),
            a.dry_run,
        ),
        Some(Command::Commit { action, message }) => commands::run_action(
            &cli,
            commands::CliAction::Commit(message.clone()),
            action.select.as_deref(),
            action.dry_run,
        ),
        Some(Command::Stash(a)) => commands::run_action(
            &cli,
            commands::CliAction::Stash,
            a.select.as_deref(),
            a.dry_run,
        ),
        Some(Command::Run {
            action,
            command,
            timeout,
        }) => commands::run_command_action(
            &cli,
            action.select.as_deref(),
            command.as_deref(),
            *timeout,
            action.dry_run,
        ),
        Some(Command::Web {
            port,
            no_open,
            no_install,
            allow_writes,
            allow_run,
            demo,
        }) => commands::run_web(
            &cli,
            *port,
            !no_open,
            !no_install,
            *allow_writes,
            *allow_run,
            *demo,
        ),
        Some(Command::Mcp {
            allow_writes,
            allow_run,
            allow_open,
        }) => commands::run_mcp(&cli, *allow_writes, *allow_run, *allow_open),
        None => commands::run_tui(&cli),
    }
}
