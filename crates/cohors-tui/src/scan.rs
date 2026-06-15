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
    /// GitHub token (v0.2): `gh auth token` / `$GITHUB_TOKEN`, discovered once.
    token: Option<String>,
    /// The user's git email, for the standup (commits they authored).
    author_email: Option<String>,
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
            token: cohors_github::discover_token(),
            author_email: git_user_email(),
        })
    }

    /// The GitHub token, if one was found (enables remote PR/CI enrichment).
    pub fn github_token(&self) -> Option<String> {
        self.token.clone()
    }

    /// The user's git email, for the standup.
    pub fn author_email(&self) -> Option<String> {
        self.author_email.clone()
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

/// The user's configured git email (`git config user.email`), for the standup.
fn git_user_email() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["config", "user.email"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let email = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if email.is_empty() { None } else { Some(email) }
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
    let mut roots: Vec<String> = raw_roots
        .iter()
        .map(|r| match home {
            Some(h) => expand_tilde(r, h),
            None => r.clone(),
        })
        .collect();

    // Zero-config default: with no roots set, scan the current directory — so
    // `cohors` "just works" anywhere (like ripgrep/lazygit) instead of showing an
    // empty fleet. Explicit `--root`/config roots always win.
    if roots.is_empty()
        && let Ok(cwd) = std::env::current_dir()
        && let Some(cwd) = cwd.to_str()
    {
        roots.push(cwd.to_string());
    }

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
