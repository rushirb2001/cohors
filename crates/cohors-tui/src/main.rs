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
mod ground;
mod logging;
mod scan;
mod tui;
mod ui;
mod update;
/// The browser dashboard's files, compiled in by `build.rs` when a release
/// build provides them (empty otherwise).
mod web_assets {
    include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));
}

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

    let result = match &cli.command {
        Some(Command::Init { force }) => commands::init(&cli, *force),
        Some(Command::Scan { select, json }) => commands::scan(&cli, select.as_deref(), *json),
        Some(Command::Overview { json }) => commands::overview(&cli, *json),
        Some(Command::Standup { today, json }) => commands::standup(&cli, *today, *json),
        Some(Command::Demo) => commands::run_demo(),
        Some(Command::Update { check }) => update::run_command(*check),
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
        Some(Command::Launchpad {
            action,
            select,
            json,
        }) => commands::launchpad::run(&cli, action.as_ref(), select.as_deref(), *json),
        Some(Command::Agent { action }) => commands::agent::run(&cli, action),
        Some(Command::Mcp {
            allow_writes,
            allow_run,
            allow_open,
            allow_launchpad,
        }) => commands::run_mcp(
            &cli,
            cohors_mcp::Caps {
                allow_writes: *allow_writes,
                allow_run: *allow_run,
                allow_open: *allow_open,
                allow_launchpad: *allow_launchpad,
                // Set by the Claude Code plugin's launcher, so refusals name
                // the plugin setting to turn on rather than a flag.
                plugin: std::env::var("COHORS_MCP_FROM_PLUGIN").is_ok_and(|v| v == "1"),
            },
        ),
        None => commands::run_tui(&cli),
    };
    // One-off commands end with a line about a newer cohors, when one is known.
    // Not the dashboard (it shows it in the header), the servers (their output
    // belongs to a client) or `update` (it just said everything).
    let one_off = matches!(
        cli.command,
        Some(
            Command::Init { .. }
                | Command::Scan { .. }
                | Command::Overview { .. }
                | Command::Standup { .. }
                | Command::Fetch(_)
                | Command::Pull(_)
                | Command::Push(_)
                | Command::Commit { .. }
                | Command::Stash(_)
                | Command::Run { .. }
                | Command::Launchpad { .. }
        )
    );
    if one_off {
        let config = cohors_config::Config::load(cli.config.as_deref().map(camino::Utf8Path::new));
        update::print_notice(config.map(|c| c.updates).unwrap_or_default());
    }
    result
}
