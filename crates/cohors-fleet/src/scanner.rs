//! Turning config (+ overrides) into repo snapshots.
//!
//! [`Scanner`] bundles the resolved config and the local git provider so every
//! surface shares one path: discover → snapshot (in parallel) → stamp groups →
//! apply aliases. It's `Send + Sync`, so front-ends can run [`Scanner::scan`]
//! on a background thread.

use std::collections::BTreeSet;
use std::time::SystemTime;

use camino::{Utf8Path, Utf8PathBuf};
use cohors_config::{Config, expand_tilde};
use cohors_core::{RepoRef, RepoSnapshot};
use cohors_git::{DiscoveryOptions, LocalGitProvider};

/// Why a [`Scanner`] couldn't be built. (Scanning itself is best-effort and
/// never errors — see [`Scanner::scan`].)
#[derive(Debug, thiserror::Error)]
pub enum FleetError {
    #[error("loading config: {0}")]
    Config(String),
}

pub struct Scanner {
    provider: LocalGitProvider,
    config: Config,
    home: Option<Utf8PathBuf>,
    config_path: String,
    /// GitHub token (v0.2): `gh auth token` / `$GITHUB_TOKEN`, discovered once.
    token: Option<String>,
    /// The user's git email, for the standup (commits they authored).
    author_email: Option<String>,
    /// Where repos found by a deep search are remembered (see [`crate::known`]).
    known_file: Option<Utf8PathBuf>,
}

/// What a search found (`cohors discover`).
#[derive(Debug, Clone, Default)]
pub struct Discovery {
    /// Every repo, canonical, sorted by path.
    pub repos: Vec<Utf8PathBuf>,
    /// Of `repos`, the ones only a deep search reaches. After a normal search
    /// these came from memory (an earlier `--deep`); after a deep search they
    /// are the ones it just found and remembered.
    pub deep_only: Vec<Utf8PathBuf>,
}

impl Scanner {
    /// Build a scanner: load config (honouring an explicit `config_path`
    /// override), resolve discovery options (applying `root_overrides` and `~`
    /// expansion), and discover the GitHub token + git identity once.
    ///
    /// The two overrides are exactly what a CLI's `--config` / `--root` flags
    /// carry — but expressed as plain values so any surface can construct one.
    pub fn new(
        config_path_override: Option<&Utf8Path>,
        root_overrides: &[String],
    ) -> Result<Self, FleetError> {
        let config =
            Config::load(config_path_override).map_err(|e| FleetError::Config(e.to_string()))?;
        let home = cohors_config::paths::home_dir().ok();
        let options = discovery_options(&config, root_overrides, home.as_deref());
        let config_path = match config_path_override {
            Some(p) => p.to_string(),
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
            known_file: crate::known::default_path(),
        })
    }

    /// Keep the remembered repos somewhere else (`None`: remember nothing).
    /// For tests, and for surfaces that must not share the user's memory.
    pub fn with_known_file(mut self, path: Option<Utf8PathBuf>) -> Self {
        self.known_file = path;
        self
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
    ///
    /// Includes the repos remembered from an earlier deep search that are
    /// still valid, so every surface shows them without walking deep.
    pub fn scan(&self) -> Vec<RepoSnapshot> {
        self.scan_with(false)
    }

    /// [`scan`](Self::scan), optionally as the deep search (no depth limit,
    /// nested repos at every level). A deep scan remembers what only it could
    /// find, for every later normal scan; see [`Scanner::discover`].
    pub fn scan_with(&self, deep: bool) -> Vec<RepoSnapshot> {
        self.scan_discovered(deep).0
    }

    /// [`scan_with`](Self::scan_with), also returning what the search found
    /// (so a report can say which repos only a deep search reaches).
    pub fn scan_discovered(&self, deep: bool) -> (Vec<RepoSnapshot>, Discovery) {
        let found = self.discover(deep);
        let refs: Vec<RepoRef> = found
            .repos
            .iter()
            .cloned()
            .map(|path| RepoRef {
                id: cohors_core::RepoId(path.to_string()),
                path: Some(path),
            })
            .collect();
        let mut snapshots = cohors_git::snapshot_all(&refs);
        // Groups first (match the raw directory name), then aliases
        // (a cosmetic rename that must not change group membership).
        apply_groups(&self.config, &mut snapshots);
        apply_aliases(&self.config, self.home.as_deref(), &mut snapshots);
        (snapshots, found)
    }

    /// Find the repos without reading them: the normal search plus the
    /// remembered ones, or (`deep`) the exhaustive search, whose finds beyond
    /// a normal search's reach are then remembered. Best-effort like
    /// [`scan`](Self::scan): a discovery error is logged and yields nothing.
    pub fn discover(&self, deep: bool) -> Discovery {
        match self.try_discover(deep) {
            Ok(found) => found,
            Err(err) => {
                tracing::error!(error = %err, "discovery failed");
                Discovery::default()
            }
        }
    }

    fn try_discover(&self, deep: bool) -> Result<Discovery, cohors_git::GitError> {
        let paths = |refs: Vec<RepoRef>| -> BTreeSet<Utf8PathBuf> {
            refs.into_iter().filter_map(|r| r.path).collect()
        };
        // The normal walk, without memory: what any refresh reaches by itself.
        let mut normal_opts = self.provider.options().clone();
        normal_opts.known.clear();
        let walked = paths(cohors_git::discover(&normal_opts)?);

        if deep {
            let mut deep_opts = normal_opts.clone();
            deep_opts.deep = true;
            let all = paths(cohors_git::discover(&deep_opts)?);
            let extra: Vec<Utf8PathBuf> = all.difference(&walked).cloned().collect();
            if let Some(file) = &self.known_file {
                let covered = cohors_git::root_dirs(&deep_opts);
                if let Err(err) = crate::known::remember(file, &covered, &extra) {
                    tracing::warn!(error = %err, %file, "could not remember deep-search repos");
                }
            }
            return Ok(Discovery {
                repos: all.into_iter().collect(),
                deep_only: extra,
            });
        }

        let mut known_opts = normal_opts;
        known_opts.known = self
            .known_file
            .as_deref()
            .map(crate::known::load)
            .unwrap_or_default();
        let remembered: BTreeSet<Utf8PathBuf> = cohors_git::admit_known_repos(&known_opts)?
            .into_iter()
            .filter(|p| !walked.contains(p))
            .collect();
        Ok(Discovery {
            repos: walked.union(&remembered).cloned().collect(),
            deep_only: remembered.into_iter().collect(),
        })
    }

    /// When the remembered repos last changed. A long-lived surface (the
    /// dashboard) compares this between ticks and rescans when it moves, so a
    /// deep search run elsewhere shows up without a restart.
    pub fn known_stamp(&self) -> Option<SystemTime> {
        self.known_file.as_deref().and_then(crate::known::stamp)
    }

    /// The roots as the folders they name: globs expanded, missing ones left
    /// out, canonical (so they compare with discovered repo paths).
    pub fn root_dirs(&self) -> Vec<Utf8PathBuf> {
        cohors_git::root_dirs(self.provider.options())
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

    /// Machine action-surface safety settings (`run` allowlist, target cap) —
    /// shared by the MCP server and the web action endpoint.
    pub fn actions_config(&self) -> cohors_config::ActionsConfig {
        self.config.actions.clone()
    }

    /// The configured TUI icon mode (auto / ascii / unicode / nerd).
    pub fn icon_mode(&self) -> cohors_config::IconMode {
        self.config.icons
    }

    /// What the user said about the terminal's background (auto / dark / light).
    pub fn theme_mode(&self) -> cohors_config::ThemeMode {
        self.config.theme
    }

    /// The configured way of keeping cohors itself up to date.
    pub fn updates_mode(&self) -> cohors_config::UpdatesMode {
        self.config.updates
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
fn discovery_options(
    config: &Config,
    root_overrides: &[String],
    home: Option<&Utf8Path>,
) -> DiscoveryOptions {
    let mut raw_roots = if root_overrides.is_empty() {
        config.roots.clone()
    } else {
        root_overrides.to_vec()
    };

    // Zero-config default: with no roots set, auto-detect where the user keeps
    // repos (common code dirs + the current directory), so `cohors` "just works"
    // anywhere instead of showing an empty fleet. Explicit `--root`/config roots
    // always win. Every surface resolves through here.
    if raw_roots.is_empty() {
        raw_roots = crate::detect::detect_roots(home);
    }

    let mut roots: Vec<String> = raw_roots
        .iter()
        .map(|r| match home {
            Some(h) => expand_tilde(r, h),
            None => r.clone(),
        })
        .collect();

    // Last-ditch fallback when nothing was detected: the current directory.
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
        deep: false,
        known: Vec::new(),
    }
}

/// Stamp each repo with the config groups whose name globs match it (matched
/// against the repo's directory name, using the same glob matcher selectors use).
/// Runs before aliasing so a cosmetic rename can't change membership.
fn apply_groups(config: &Config, snapshots: &mut [RepoSnapshot]) {
    if config.groups.is_empty() {
        return;
    }
    for snap in snapshots.iter_mut() {
        // The directory name is the stable identity to match globs against;
        // fall back to the current name for a path-less repo.
        let key = snap
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .unwrap_or(snap.name.as_str())
            .to_string();
        // BTreeMap iterates in name order, so `groups` comes out sorted.
        snap.groups = config
            .groups
            .iter()
            .filter(|(_, globs)| globs.iter().any(|g| cohors_core::glob_name(g, &key)))
            .map(|(name, _)| name.clone())
            .collect();
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
