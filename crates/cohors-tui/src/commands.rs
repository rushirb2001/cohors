//! Implementations of the CLI subcommands and the shared config→discovery glue.

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use cohors_config::{Config, expand_tilde};
use cohors_core::RepoSnapshot;
use cohors_git::{DiscoveryOptions, LocalGitProvider};

use crate::cli::Cli;

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
    let snapshots = collect_snapshots(cli)?;
    let json = serde_json::to_string_pretty(&snapshots).context("serializing snapshots")?;
    println!("{json}");
    Ok(())
}

/// Bare `cohors` — launch the dashboard. (Lands in the next milestone step.)
pub fn run_tui(_cli: &Cli) -> Result<()> {
    println!("The cohors dashboard arrives in the next step. For now, try `cohors scan`.");
    Ok(())
}

/// Load config, discover repos, snapshot them in parallel, and apply aliases.
/// Shared by `scan` and (soon) the TUI.
pub(crate) fn collect_snapshots(cli: &Cli) -> Result<Vec<RepoSnapshot>> {
    let config = load_config(cli)?;
    let home = cohors_config::paths::home_dir().ok();
    let options = discovery_options(&config, &cli.root, home.as_deref());

    let provider = LocalGitProvider::new(options);
    let mut snapshots = provider.scan().context("scanning repositories")?;
    apply_aliases(&config, home.as_deref(), &mut snapshots);
    Ok(snapshots)
}

pub(crate) fn load_config(cli: &Cli) -> Result<Config> {
    let path = cli.config.as_deref().map(Utf8Path::new);
    Config::load(path).context("loading config")
}

/// Map a [`Config`] (plus any `--root` overrides) to [`DiscoveryOptions`],
/// expanding `~` in roots against the home directory.
pub(crate) fn discovery_options(
    config: &Config,
    root_overrides: &[String],
    home: Option<&Utf8Path>,
) -> DiscoveryOptions {
    let raw_roots = if root_overrides.is_empty() {
        config.roots.clone()
    } else {
        root_overrides.to_vec()
    };
    let roots = raw_roots
        .iter()
        .map(|r| match home {
            Some(h) => expand_tilde(r, h),
            None => r.clone(),
        })
        .collect();

    DiscoveryOptions {
        roots,
        ignore: config.ignore.clone(),
        max_depth: config.max_depth,
        stop_at_repo: config.stop_at_repo,
        follow_symlinks: config.follow_symlinks,
    }
}

/// Replace repo names with configured aliases (by absolute path or dir name).
fn apply_aliases(config: &Config, home: Option<&Utf8Path>, snapshots: &mut [RepoSnapshot]) {
    let Some(home) = home else {
        return;
    };
    for snap in snapshots.iter_mut() {
        if let Some(path) = snap.path.clone() {
            let dir_name = path.file_name().unwrap_or_default();
            if let Some(alias) = config.alias_for(&path, dir_name, home) {
                snap.name = alias;
            }
        }
    }
}
