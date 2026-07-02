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
mod editors;
mod glyphs;
mod logging;
mod prefs;
mod scan;
mod tui;
mod ui;

use clap::Parser;

use cli::{Cli, Command};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    logging::init();

    match &cli.command {
        Some(Command::Init { force }) => commands::init(&cli, *force),
        Some(Command::Scan { select }) => commands::scan(&cli, select.as_deref()),
        Some(Command::Demo) => commands::run_demo(),
        Some(Command::Fetch(a)) => {
            commands::run_action(&cli, commands::CliAction::Fetch, &a.select, a.dry_run)
        }
        Some(Command::Pull(a)) => {
            commands::run_action(&cli, commands::CliAction::Pull, &a.select, a.dry_run)
        }
        Some(Command::Push(a)) => {
            commands::run_action(&cli, commands::CliAction::Push, &a.select, a.dry_run)
        }
        Some(Command::Commit { action, message }) => commands::run_action(
            &cli,
            commands::CliAction::Commit(message.clone()),
            &action.select,
            action.dry_run,
        ),
        Some(Command::Stash(a)) => {
            commands::run_action(&cli, commands::CliAction::Stash, &a.select, a.dry_run)
        }
        Some(Command::Run {
            action,
            command,
            timeout,
        }) => commands::run_command_action(&cli, &action.select, command, *timeout, action.dry_run),
        Some(Command::Web {
            port,
            no_open,
            no_install,
            allow_writes,
            allow_run,
        }) => commands::run_web(
            &cli,
            *port,
            !no_open,
            !no_install,
            *allow_writes,
            *allow_run,
        ),
        Some(Command::Mcp {
            allow_writes,
            allow_run,
            allow_open,
        }) => commands::run_mcp(&cli, *allow_writes, *allow_run, *allow_open),
        None => commands::run_tui(&cli),
    }
}
