//! CLI → fleet Scanner shim.
//!
//! The Scanner itself (config → discovery → parallel snapshot → groups/aliases)
//! lives in `cohors-fleet`, shared by every surface. This module keeps the one
//! genuinely TUI-shaped step: translating parsed CLI flags into the Scanner's
//! plain-value overrides.

use anyhow::{Result, bail};
use camino::Utf8Path;

pub use cohors_fleet::Scanner;

use crate::cli::Cli;

/// Build the shared [`Scanner`] from the parsed CLI (`--config` / `--root`).
///
/// A `--root` naming a folder that does not exist is refused here, once for
/// every command: searching it would only end in "no repositories found",
/// which hides the typo. Globs are left to discovery.
pub fn from_cli(cli: &Cli) -> Result<Scanner> {
    let home = cohors_config::paths::home_dir().ok();
    for root in &cli.root {
        let expanded = match &home {
            Some(h) => cohors_config::expand_tilde(root, h),
            None => root.clone(),
        };
        let is_glob = expanded.contains(['*', '?', '[']);
        if !is_glob && !Utf8Path::new(&expanded).is_dir() {
            bail!("--root {root}: no such folder");
        }
    }
    // A `--config` naming a file that does not exist is an error for every
    // command (it says to run `cohors init --config <path>`), except the bare
    // dashboard: its first-run greeting and empty-state rescue write the config
    // to that very path, so it must be able to start without it.
    let config = cli.config.as_deref().map(Utf8Path::new);
    let scanner = if cli.command.is_none() {
        Scanner::new_allowing_missing_config(config, &cli.root)?
    } else {
        Scanner::new(config, &cli.root)?
    };
    // A mistyped key (`max_dept`) used to vanish into the log file. One-off
    // commands say so on stderr, where a pipe or an MCP client won't mind;
    // the dashboard is about to take the screen, so it keeps to the log.
    if cli.command.is_some() {
        for warning in scanner.config_warnings() {
            eprintln!("cohors: config: {warning}");
        }
    }
    Ok(scanner)
}
