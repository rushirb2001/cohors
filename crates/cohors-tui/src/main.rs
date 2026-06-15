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
        Some(Command::Scan) => commands::scan(&cli),
        Some(Command::Demo) => commands::run_demo(),
        None => commands::run_tui(&cli),
    }
}
