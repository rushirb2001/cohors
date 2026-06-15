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
}
