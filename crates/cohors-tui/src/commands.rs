//! Implementations of the CLI subcommands.

use std::sync::Arc;

use anyhow::{Context, Result};
use camino::Utf8PathBuf;

use crate::cli::Cli;
use crate::scan::Scanner;

/// `cohors init` — write a starter config and print its path.
pub fn init(cli: &Cli, force: bool) -> Result<()> {
    let path: Utf8PathBuf = match &cli.config {
        Some(p) => Utf8PathBuf::from(p),
        None => cohors_config::paths::config_file().context("resolving the config path")?,
    };
    cohors_config::write_starter(&path, force)
        .with_context(|| format!("writing config to {path}"))?;
    println!("Wrote starter config to {path}");
    println!("Edit it, then run `cohors`.");
    Ok(())
}

/// `cohors scan` — discover + snapshot all repos and print JSON to stdout.
pub fn scan(cli: &Cli) -> Result<()> {
    let scanner = Scanner::from_cli(cli)?;
    let snapshots = scanner.scan();
    let json = serde_json::to_string_pretty(&snapshots).context("serializing snapshots")?;
    println!("{json}");
    Ok(())
}

/// Bare `cohors` — launch the interactive dashboard.
pub fn run_tui(cli: &Cli) -> Result<()> {
    let scanner = Arc::new(Scanner::from_cli(cli)?);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the tokio runtime")?;
    runtime.block_on(crate::tui::run(scanner))
}
