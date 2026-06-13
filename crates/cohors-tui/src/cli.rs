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
    Scan,
}
