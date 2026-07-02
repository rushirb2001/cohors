//! CLI → fleet Scanner shim.
//!
//! The Scanner itself (config → discovery → parallel snapshot → groups/aliases)
//! lives in `cohors-fleet`, shared by every surface. This module keeps the one
//! genuinely TUI-shaped step: translating parsed CLI flags into the Scanner's
//! plain-value overrides.

use anyhow::Result;
use camino::Utf8Path;

pub use cohors_fleet::Scanner;

use crate::cli::Cli;

/// Build the shared [`Scanner`] from the parsed CLI (`--config` / `--root`).
pub fn from_cli(cli: &Cli) -> Result<Scanner> {
    Ok(Scanner::new(
        cli.config.as_deref().map(Utf8Path::new),
        &cli.root,
    )?)
}
