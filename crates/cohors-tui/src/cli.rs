//! Command-line interface (clap).

use clap::{Parser, Subcommand};

/// A fast dashboard for all your git repos.
#[derive(Debug, Parser)]
#[command(name = "cohors", version, about, long_about = None)]
pub struct Cli {
    /// Override the config file location.
    #[arg(long, value_name = "PATH", global = true)]
    pub config: Option<String>,

    /// Override the configured roots; repeatable (e.g. `--root ~/work --root ~/oss`).
    #[arg(long, value_name = "DIR", global = true)]
    pub root: Vec<String>,

    /// Force a fresh scan, ignoring the cache.
    #[arg(long, global = true)]
    pub no_cache: bool,

    /// Keep the dashboard live: re-scan automatically every few seconds.
    #[arg(long, global = true)]
    pub watch: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Write a starter config file and print its path.
    Init {
        /// Overwrite an existing config file.
        #[arg(long)]
        force: bool,
    },
    /// Discover and snapshot all repos, printing JSON to stdout (no TUI).
    Scan {
        /// Only include repos matching a selector — JSON (`'{"behind":true}'`)
        /// or shorthand (`dirty`, `behind`, `ahead`, `attention`, `name:pay*`,
        /// `ci:failing`, comma-separated to AND them). Results print in
        /// dirty-first order. Omit to print the whole fleet.
        #[arg(long, value_name = "QUERY")]
        select: Option<String>,
    },
    /// Launch the dashboard on a built-in sample fleet — no config, no scanning.
    ///
    /// A zero-setup way to try cohors: every column and view is populated with
    /// privacy-safe demo data. Actions are stubbed (nothing touches your disk).
    Demo,
    /// git fetch across the selected repos (non-destructive).
    Fetch(ActionArgs),
    /// git pull --ff-only across the selected repos — never merges or loses work.
    Pull(ActionArgs),
    /// git push across the selected repos — never force-pushes.
    Push(ActionArgs),
    /// git add -A + commit across the selected repos (stages tracked + untracked).
    Commit {
        #[command(flatten)]
        action: ActionArgs,
        /// Commit message (applied to every selected repo).
        #[arg(short, long, value_name = "MSG")]
        message: String,
    },
    /// git stash push (tracked changes) across the selected repos.
    Stash(ActionArgs),
    /// Run a shell command in each selected repo (the fleet codemod/test primitive).
    Run {
        #[command(flatten)]
        action: ActionArgs,
        /// The shell command to run in each repo.
        #[arg(value_name = "COMMAND")]
        command: String,
        /// Per-repo timeout in seconds.
        #[arg(long, default_value_t = 120)]
        timeout: u64,
    },
    /// Build and serve the web dashboard, then open it in your browser.
    ///
    /// One command starts everything: it finds the `cohors-web` crate, makes sure
    /// Trunk (the WASM bundler) is installed — installing it for you the first
    /// time unless `--no-install` — then runs the dev server and prints the URL.
    /// Run it from inside the cohors repository. Ctrl-C to stop.
    Web {
        /// Port to serve on.
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// Don't open a browser window automatically.
        #[arg(long)]
        no_open: bool,
        /// Fail with instructions instead of installing Trunk when it's missing.
        #[arg(long)]
        no_install: bool,
    },
    /// Run as a Model Context Protocol server over stdio, so a coding agent can
    /// see and (opt-in) act on your fleet. Read-only by default.
    Mcp {
        /// Enable write tools (`fetch`, `pull`, `push`, `commit`, `stash`).
        #[arg(long)]
        allow_writes: bool,
        /// Enable the `run` tool (arbitrary shell across repos).
        #[arg(long)]
        allow_run: bool,
        /// Enable the local-only `open` tool (shared desktop session).
        #[arg(long)]
        allow_open: bool,
    },
}

/// Shared arguments for the bulk-action subcommands. A selector is required so an
/// action never silently hits the whole fleet; pass `--select all` to opt into it.
#[derive(Debug, clap::Args)]
pub struct ActionArgs {
    /// Which repos to act on — same selector language as `scan --select`
    /// (`dirty`, `behind`, `name:pay*`, `'{"ahead":true}'`, `all`, …).
    #[arg(long, value_name = "QUERY")]
    pub select: String,
    /// Preview the matching repos without acting.
    #[arg(long)]
    pub dry_run: bool,
}
