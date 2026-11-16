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
    /// The config file this scanner read (or would read: it may not exist).
    /// `None` when no path could be resolved at all.
    config_file: Option<Utf8PathBuf>,
    /// What the scanner was built from, kept so [`Scanner::reload`] can read
    /// the config again the same way (same `--config`, same `--root`s).
    config_override: Option<Utf8PathBuf>,
    root_overrides: Vec<String>,
    allow_missing_config: bool,
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

impl Discovery {
    /// The found repo that encloses `path` most closely, if any (a repo
    /// nested inside another).
    pub fn nested_in(&self, path: &Utf8Path) -> Option<&Utf8PathBuf> {
        self.repos
            .iter()
            .filter(|other| other.as_path() != path && path.starts_with(other))
            .max_by_key(|other| other.as_str().len())
    }
}

impl Scanner {
    /// Build a scanner: load config (honouring an explicit `config_path`
    /// override), resolve discovery options (applying `root_overrides` and `~`
    /// expansion), and discover the GitHub token + git identity once.
    ///
    /// The two overrides are exactly what a CLI's `--config` / `--root` flags
    /// carry — but expressed as plain values so any surface can construct one.
    ///
    /// An explicit `config_path_override` that does not exist is an error
    /// (naming the path and `cohors init --config <path>`): scanning a fleet
    /// from defaults instead would hide the typo. A missing file at the default
    /// location is fine (zero-config use). See
    /// [`Scanner::new_allowing_missing_config`] for the one surface that offers
    /// to create the file itself.
    pub fn new(
        config_path_override: Option<&Utf8Path>,
        root_overrides: &[String],
    ) -> Result<Self, FleetError> {
        let config = Config::load(config_path_override).map_err(config_error)?;
        Ok(Self::build(
            config,
            config_path_override,
            root_overrides,
            false,
            Identity::discover(),
        ))
    }

    /// [`Scanner::new`], except that a missing *explicit* config file yields
    /// defaults instead of an error. For the dashboard, whose first-run greeting
    /// and empty-state rescue write the config to that very `--config` path:
    /// refusing to start would leave nothing to create it.
    pub fn new_allowing_missing_config(
        config_path_override: Option<&Utf8Path>,
        root_overrides: &[String],
    ) -> Result<Self, FleetError> {
        let config = Config::load_or_default(config_path_override).map_err(config_error)?;
        Ok(Self::build(
            config,
            config_path_override,
            root_overrides,
            true,
            Identity::discover(),
        ))
    }

    fn build(
        config: Config,
        config_path_override: Option<&Utf8Path>,
        root_overrides: &[String],
        allow_missing_config: bool,
        identity: Identity,
    ) -> Self {
        let home = cohors_config::paths::home_dir().ok();
        let options = discovery_options(&config, root_overrides, home.as_deref());
        let config_file = match config_path_override {
            Some(p) => Some(p.to_path_buf()),
            None => cohors_config::paths::config_file().ok(),
        };
        let config_path = config_file
            .as_ref()
            .map(|p| p.to_string())
            .unwrap_or_else(|| "<unknown>".to_string());
        Self {
            provider: LocalGitProvider::new(options),
            config,
            home,
            config_path,
            config_file,
            config_override: config_path_override.map(Utf8Path::to_path_buf),
            root_overrides: root_overrides.to_vec(),
            allow_missing_config,
            token: identity.token,
            author_email: identity.author_email,
            known_file: crate::known::default_path(),
        }
    }

    /// Read the config again, the way this scanner first read it (same
    /// `--config`, same `--root`s, same memory of deep-searched repos), for a
    /// long-lived surface whose user just edited the file. The GitHub token
    /// and git identity are kept: they do not come from the config, and
    /// finding them again means running `gh` and `git`.
    ///
    /// A config that no longer parses is an error, and the caller keeps the
    /// scanner it has: a half-saved file must not empty the dashboard.
    pub fn reload(&self) -> Result<Self, FleetError> {
        let path = self.config_override.as_deref();
        let config = if self.allow_missing_config {
            Config::load_or_default(path)
        } else {
            Config::load(path)
        }
        .map_err(config_error)?;
        let identity = Identity {
            token: self.token.clone(),
            author_email: self.author_email.clone(),
        };
        let mut reloaded = Self::build(
            config,
            path,
            &self.root_overrides,
            self.allow_missing_config,
            identity,
        );
        reloaded.known_file = self.known_file.clone();
        Ok(reloaded)
    }

    /// The config file this scanner reads (it may not exist yet), so a
    /// long-lived surface can notice an edit. `None` when no location could be
    /// resolved (no home directory).
    pub fn config_file(&self) -> Option<&Utf8Path> {
        self.config_file.as_deref()
    }

    /// The file a deep search remembers repos in, if this scanner uses one.
    pub fn known_file(&self) -> Option<&Utf8Path> {
        self.known_file.as_deref()
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
        let snapshots = self.snapshot_repos(&found.repos);
        (snapshots, found)
    }

    /// Snapshot exactly these repos (in parallel, in this order), stamped with
    /// groups and aliases like a full scan. The watcher uses it to re-read
    /// only the repos whose git state moved, and to snapshot what it just
    /// discovered without walking the roots twice.
    pub fn snapshot_repos(&self, repos: &[Utf8PathBuf]) -> Vec<RepoSnapshot> {
        let refs: Vec<RepoRef> = repos
            .iter()
            .cloned()
            .map(|path| RepoRef {
                id: cohors_core::RepoId(path.to_string()),
                path: Some(path),
            })
            .collect();
        let snapshots = cohors_git::snapshot_all(&refs);
        self.stamp(snapshots)
    }

    /// [`snapshot_repos`](Self::snapshot_repos), one repository at a time on
    /// the calling thread. It takes longer to finish but costs far less CPU:
    /// the parallel pool's threads spin while they wait for work, which costs
    /// more than it saves for a background rescan nobody is waiting on
    /// (measured on 24 repos: 151 ms of CPU in parallel, 61 ms serially).
    pub fn snapshot_repos_serially(&self, repos: &[Utf8PathBuf]) -> Vec<RepoSnapshot> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let snapshots = repos
            .iter()
            .map(|path| {
                cohors_git::snapshot_repo(
                    &RepoRef {
                        id: cohors_core::RepoId(path.to_string()),
                        path: Some(path.clone()),
                    },
                    now,
                )
            })
            .collect();
        self.stamp(snapshots)
    }

    /// Groups first (match the raw directory name), then aliases (a cosmetic
    /// rename that must not change group membership).
    fn stamp(&self, mut snapshots: Vec<RepoSnapshot>) -> Vec<RepoSnapshot> {
        apply_groups(&self.config, &mut snapshots);
        apply_aliases(&self.config, self.home.as_deref(), &mut snapshots);
        snapshots
    }

    /// Find the repos without reading them: the normal search plus the
    /// remembered ones, or (`deep`) the exhaustive search, whose finds beyond
    /// a normal search's reach are then remembered. Best-effort like
    /// [`scan`](Self::scan): a discovery error is logged and yields nothing.
    pub fn discover(&self, deep: bool) -> Discovery {
        self.discover_remembering(deep, true)
    }

    /// [`discover`](Self::discover) that writes nothing: a deep search reports
    /// what only it reaches but does not remember it, so later scans are
    /// unchanged. For a surface that may only look (the MCP `discover` tool).
    pub fn discover_read_only(&self, deep: bool) -> Discovery {
        self.discover_remembering(deep, false)
    }

    fn discover_remembering(&self, deep: bool, remember: bool) -> Discovery {
        match self.try_discover(deep, remember) {
            Ok(found) => found,
            Err(err) => {
                tracing::error!(error = %err, "discovery failed");
                Discovery::default()
            }
        }
    }

    fn try_discover(&self, deep: bool, remember: bool) -> Result<Discovery, cohors_git::GitError> {
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
            if remember && let Some(file) = &self.known_file {
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

    /// What a front-end should tell the user about the loaded config, one
    /// sentence each: unknown keys (typos such as `max_dept`, with a "did you
    /// mean") and semantic footguns (`max_depth = 0`, an empty group). Empty
    /// for a clean config or defaults. These used to reach only the log file.
    pub fn config_warnings(&self) -> Vec<String> {
        self.config.warnings()
    }
}

/// Flatten a config error into [`FleetError::Config`], keeping its whole
/// `source()` chain: the TOML parser's line, column and reason live in the
/// source, and `to_string()` alone would print only "invalid TOML in <path>".
fn config_error(e: cohors_config::ConfigError) -> FleetError {
    let mut message = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        let text = cause.to_string();
        let text = text.trim_end();
        if !text.is_empty() && !message.contains(text) {
            message.push_str(": ");
            message.push_str(text);
        }
        source = cause.source();
    }
    FleetError::Config(message)
}

/// Who the user is, as far as cohors needs: found once per scanner (it runs
/// `gh` and `git`) and carried over when the config is reloaded.
struct Identity {
    token: Option<String>,
    author_email: Option<String>,
}

impl Identity {
    fn discover() -> Self {
        Self {
            token: cohors_github::discover_token(),
            author_email: git_user_email(),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_in_names_the_closest_enclosing_repo() {
        let p = |s: &str| Utf8PathBuf::from(s);
        let found = Discovery {
            repos: vec![p("/c"), p("/c/a"), p("/c/a/b"), p("/c/ab")],
            deep_only: Vec::new(),
        };
        assert_eq!(found.nested_in(&p("/c/a/b")), Some(&p("/c/a")));
        assert_eq!(
            found.nested_in(&p("/c/ab")),
            Some(&p("/c")),
            "a/ab is not nesting"
        );
        assert_eq!(found.nested_in(&p("/c")), None);
    }
}
