//! Turning CLI + config into repo snapshots.
//!
//! [`Scanner`] bundles the resolved config and the local git provider so both
//! `cohors scan` and the dashboard share one path: discover → snapshot (in
//! parallel) → apply aliases. It's cheaply [`Clone`]-free and `Send + Sync`, so
//! the TUI can run [`Scanner::scan`] on a blocking worker thread.

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use cohors_config::{Config, expand_tilde};
use cohors_core::RepoSnapshot;
use cohors_git::{DiscoveryOptions, LocalGitProvider};

use crate::cli::Cli;

pub struct Scanner {
    provider: LocalGitProvider,
    config: Config,
    home: Option<Utf8PathBuf>,
    config_path: String,
}

impl Scanner {
    /// Build a scanner from the parsed CLI: load config, resolve discovery
    /// options (applying `--root` overrides and `~` expansion).
    pub fn from_cli(cli: &Cli) -> Result<Self> {
        let config =
            Config::load(cli.config.as_deref().map(Utf8Path::new)).context("loading config")?;
        let home = cohors_config::paths::home_dir().ok();
        let options = discovery_options(&config, &cli.root, home.as_deref());
        let config_path = match &cli.config {
            Some(p) => p.clone(),
            None => cohors_config::paths::config_file()
                .map(|p| p.to_string())
                .unwrap_or_else(|_| "<unknown>".to_string()),
        };
        Ok(Self {
            provider: LocalGitProvider::new(options),
            config,
            home,
            config_path,
        })
    }

    /// Discover and snapshot every repo (in parallel), applying aliases.
    /// Best-effort: a discovery error is logged and yields an empty list rather
    /// than failing the dashboard.
    pub fn scan(&self) -> Vec<RepoSnapshot> {
        match self.provider.scan() {
            Ok(mut snapshots) => {
                apply_aliases(&self.config, self.home.as_deref(), &mut snapshots);
                snapshots
            }
            Err(err) => {
                tracing::error!(error = %err, "discovery failed");
                Vec::new()
            }
        }
    }

    /// The (expanded) roots being searched, for the empty/loading states.
    pub fn roots(&self) -> Vec<String> {
        self.provider.options().roots.clone()
    }

    /// The config file path, for the help overlay.
    pub fn config_path(&self) -> String {
        self.config_path.clone()
    }

    /// The editor command for the "open" action (config, else `$EDITOR`/`$VISUAL`).
    pub fn editor_command(&self) -> Option<String> {
        self.config.editor_command()
    }
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
