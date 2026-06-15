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
mod detect;
mod editors;
mod logging;
mod mcp;
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
        Some(Command::Mcp {
            allow_writes,
            allow_run,
            allow_open,
        }) => commands::run_mcp(&cli, *allow_writes, *allow_run, *allow_open),
        None => commands::run_tui(&cli),
    }
}
