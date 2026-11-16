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
mod mouse;
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
    let command = Cli::command()
        .before_help(before)
        .after_help(after)
        .help_template(cli::HELP_TEMPLATE);
    let grouped = commands::grouped_commands(&command);
    match command.try_get_matches() {
        Ok(matches) => Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit()),
        // `cohors --help` (and `cohors help`): the commands by what they are
        // for, each group with a line that says so, instead of one flat list.
        Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => {
            let help = e.render().ansi().to_string();
            match commands::regroup_help(&help, &grouped) {
                Some(regrouped) => {
                    print!("{regrouped}");
                    std::process::exit(0);
                }
                None => e.exit(),
            }
        }
        Err(e) => e.exit(),
    }
}

fn main() -> anyhow::Result<()> {
    let cli = parse_cli();
    logging::init();
    // `--report`: the report a person reads, even on a pipe. Chosen once.
    commands::set_report(cli.report);
    // How the plain-terminal screens look (style, ASCII, width), chosen once.
    let icons = cohors_config::Config::load(cli.config.as_deref().map(camino::Utf8Path::new))
        .map(|c| c.icons)
        .unwrap_or_default();
    commands::look::init(icons);

    let updates = || {
        cohors_config::Config::load(cli.config.as_deref().map(camino::Utf8Path::new))
            .map(|c| c.updates)
            .unwrap_or_default()
    };
    // A copy installed by Homebrew, pip, uv, npm, cargo or the shell installer
    // sets cohors' own install up on its first run, then hands control to it
    // whenever it is newer (returning here only when this copy is the one to
    // run). Not for `install` and `uninstall`, which manage that install;
    // `doctor` may be handed off but never sets anything up; `mcp` and `hook`
    // never print the one-time notice (a client reads their output). Only the
    // dashboard installs a newer release before it opens.
    if !matches!(
        cli.command,
        Some(Command::Install { .. } | Command::Uninstall { .. })
    ) {
        update::start(update::Start {
            configured: updates(),
            // A hook, like `doctor`, never sets anything up: a session
            // starting must not be what installs a copy of cohors.
            set_up: !matches!(cli.command, Some(Command::Doctor | Command::Hook { .. })),
            quiet: matches!(
                cli.command,
                Some(Command::Mcp { .. } | Command::Hook { .. })
            ),
            update_first: cli.command.is_none(),
        });
    }
    // One-off commands end with a line about a newer cohors, when one is known.
    // Not the dashboard (it shows it in the header), the servers and `hook`
    // (their output belongs to a client, and a hook's is JSON a line more
    // would corrupt) or the update commands (they say everything). The look
    // runs while the command does and is never waited for.
    let one_off = matches!(
        cli.command,
        Some(
            Command::Init { .. }
                | Command::Discover { .. }
                | Command::Scan { .. }
                | Command::Overview { .. }
                | Command::Standup { .. }
                | Command::Fetch { .. }
                | Command::Pull { .. }
                | Command::Push { .. }
                | Command::Commit { .. }
                | Command::Stash { .. }
                | Command::Run { .. }
                | Command::Launchpad { .. }
                | Command::Next { .. }
                | Command::Path { .. }
                | Command::Show { .. }
                | Command::Changes { .. }
                | Command::Search { .. }
                | Command::Prs { .. }
        )
    );
    let pending = one_off.then(|| update::check_in_background(updates()));

    let result = match &cli.command {
        Some(Command::Init { force }) => commands::init(&cli, *force),
        Some(Command::Discover { deep, json }) => commands::discover(&cli, *deep, *json),
        Some(Command::Scan { deep, select, json }) => {
            commands::scan(&cli, select.as_deref(), *json, *deep)
        }
        Some(Command::Overview { json, select }) => {
            commands::overview(&cli, select.as_deref(), *json)
        }
        Some(Command::Standup {
            today,
            since,
            select,
            json,
        }) => commands::standup(&cli, *today, since.as_deref(), select.as_deref(), *json),
        Some(Command::Demo) => commands::run_demo(),
        Some(Command::Update { check, json }) => update::run_command(*check, *json),
        Some(Command::Install { version }) => update::install_command(version.as_deref()),
        Some(Command::Doctor) => update::run_doctor(updates()),
        Some(Command::Uninstall { dry_run }) => update::uninstall_command(*dry_run),
        Some(Command::Fetch { action: a, names }) => commands::run_action(
            &cli,
            commands::CliAction::Fetch,
            a.select.as_deref(),
            &names.repos,
            a.dry_run,
            a.json,
            a.yes,
        ),
        Some(Command::Pull { action: a, names }) => commands::run_action(
            &cli,
            commands::CliAction::Pull,
            a.select.as_deref(),
            &names.repos,
            a.dry_run,
            a.json,
            a.yes,
        ),
        Some(Command::Push { action: a, names }) => commands::run_action(
            &cli,
            commands::CliAction::Push,
            a.select.as_deref(),
            &names.repos,
            a.dry_run,
            a.json,
            a.yes,
        ),
        Some(Command::Commit {
            action,
            names,
            message,
            staged,
            allow_secrets,
        }) => commands::run_action(
            &cli,
            commands::CliAction::Commit(
                message.clone(),
                cohors_actions::CommitOptions {
                    staged_only: *staged,
                    allow_secrets: *allow_secrets,
                },
            ),
            action.select.as_deref(),
            &names.repos,
            action.dry_run,
            action.json,
            action.yes,
        ),
        Some(Command::Stash { action: a, names }) => commands::run_action(
            &cli,
            commands::CliAction::Stash,
            a.select.as_deref(),
            &names.repos,
            a.dry_run,
            a.json,
            a.yes,
        ),
        Some(Command::Run {
            action,
            command,
            timeout,
        }) => commands::run_command_action(
            &cli,
            action.select.as_deref(),
            commands::shell_line(command).as_deref(),
            *timeout,
            action.dry_run,
            action.json,
            action.yes,
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
        Some(Command::Agent(args)) => commands::agent::run(&cli, args),
        Some(Command::Completions { shell }) => {
            let mut cmd = Cli::command();
            clap_complete::generate(*shell, &mut cmd, "cohors", &mut std::io::stdout());
            Ok(())
        }
        Some(Command::Next {
            pick,
            limit,
            json,
            dismiss,
        }) => commands::next::run(&cli, *pick, *limit, *json, dismiss.as_deref()),
        Some(Command::Mcp {
            allow_writes,
            allow_run,
            allow_open,
            allow_launchpad,
        }) => {
            if *allow_open {
                // On stderr: stdout carries the protocol. An MCP client shows
                // a server's stderr in its logs, so the person who registered
                // the flag can find out it does nothing.
                eprintln!("{}", commands::ALLOW_OPEN_DEPRECATED);
            }
            commands::run_mcp(
                &cli,
                cohors_mcp::Caps {
                    allow_writes: *allow_writes,
                    allow_run: *allow_run,
                    allow_launchpad: *allow_launchpad,
                    // Set by the Claude Code plugin's launcher, so refusals
                    // name the plugin setting to turn on rather than a flag.
                    plugin: std::env::var("COHORS_MCP_FROM_PLUGIN").is_ok_and(|v| v == "1"),
                },
            )
        }
        Some(Command::Hook { event }) => commands::hook::run(event),
        Some(Command::Path { repo }) => commands::reads::path(&cli, repo),
        Some(Command::Show { repo, json }) => commands::reads::show(&cli, repo, *json),
        Some(Command::Changes {
            select,
            patch,
            max_bytes,
            json,
        }) => commands::reads::changes(&cli, select.as_deref(), *patch, *max_bytes, *json),
        Some(Command::Search {
            query,
            select,
            max_results,
            json,
        }) => commands::reads::search(&cli, query, select.as_deref(), *max_results, *json),
        Some(Command::Prs { select, json }) => commands::reads::prs(&cli, select.as_deref(), *json),
        Some(Command::Watch { json }) => commands::watch::run(&cli, *json),
        None => commands::run_tui(&cli),
    };
    if let Some(pending) = pending {
        update::print_notice(pending);
    }
    // A command that already explained itself only needs its exit status.
    if let Err(err) = &result
        && let Some(code) = err.downcast_ref::<commands::ExitCode>()
    {
        std::process::exit(code.0);
    }
    result
}
