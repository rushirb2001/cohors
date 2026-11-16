//! The cohors configuration model: parse `config.toml`, fall back to sensible
//! defaults, apply environment overrides, and write a starter file for
//! `cohors init`.
//!
//! Layering (lowest to highest precedence): built-in defaults → the config
//! file → `COHORS_*` environment variables. Each layer only overrides what it
//! actually sets — a file with one key, or a single env var, leaves everything
//! else untouched.

use std::collections::BTreeMap;

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

/// Resolved configuration.
///
/// Deserializes directly with `#[serde(default)]`, so a partial file overrides
/// only the keys it names (the old two-struct `RawConfig` mirror is gone — one
/// struct, one place to add a key; a drift test guards [`KNOWN_KEYS`] and the
/// starter template).
///
/// Roots, ignore globs, and alias keys are stored exactly as written (`~` and
/// globs intact). Expansion happens later, at the discovery boundary, via
/// [`expand_tilde`] — that way a `Config` value needs no home directory and
/// [`Config::default`] can't fail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Directories (or globs) to search for repos.
    pub roots: Vec<String>,
    /// How deep to descend from each root looking for `.git`.
    pub max_depth: usize,
    /// Glob patterns to skip during discovery.
    pub ignore: Vec<String>,
    /// Pretty names keyed by absolute path or repo directory name.
    pub aliases: BTreeMap<String, String>,
    /// Named groups → repo name globs (e.g. `payments = ["pay-*"]`). A
    /// repo is stamped with every group whose globs match its directory name, so
    /// a surface can target a whole cluster with one selector.
    pub groups: BTreeMap<String, Vec<String>>,
    /// Editor command; falls back to `$EDITOR`/`$VISUAL` when `None`.
    pub editor: Option<String>,
    /// Stop descending into a repo once `.git` is found (don't nest).
    pub stop_at_repo: bool,
    /// Follow symlinks during discovery.
    pub follow_symlinks: bool,
    /// Safety knobs for the machine action surfaces — the `cohors mcp` agent
    /// server AND the `cohors web` action endpoint share them (ADR-025).
    /// Accepted as `[actions]` (canonical) or `[mcp]` (the historical name).
    #[serde(alias = "mcp")]
    pub actions: ActionsConfig,
    /// How the TUI renders status glyphs (auto / ascii / unicode / nerd).
    pub icons: IconMode,
    /// What the dashboard assumes about the terminal's background
    /// (auto / dark / light).
    pub theme: ThemeMode,
    /// Keeping cohors itself up to date (auto / notify / off).
    pub updates: UpdatesMode,
    /// Top-level keys the file held that cohors does not know (typos such as
    /// `max_dept`), recorded by [`Config::parse`] so a front-end can warn about
    /// them (see [`Config::warnings`]). Not part of the file format: never read
    /// from or written to TOML.
    #[serde(skip)]
    unknown_keys: Vec<String>,
}

/// Keeping cohors itself up to date. The default `auto` looks for a new
/// release at most once a day and installs it in the background the way cohors
/// was installed (Homebrew, pip or the shell installer); it then says "Restart
/// to update". `notify` only says that a new version exists and how to get it.
/// `off` never looks. `COHORS_NO_UPDATE=1` is the same as `off`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdatesMode {
    #[default]
    Auto,
    Notify,
    Off,
}

/// What the dashboard assumes about the terminal's background. The default
/// `auto` asks the terminal for its background colour; a terminal that does
/// not answer gets a look that works on any background (no filled panels).
/// `dark` or `light` says it outright, for a terminal that cannot be asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Auto,
    Dark,
    Light,
}

/// How the TUI renders status glyphs. The default `auto` picks Unicode glyphs,
/// but falls back to ASCII when `NO_COLOR` is set (glyph meaning leans on colour,
/// so a colourless `●` is ambiguous). `nerd` opts into Nerd-Font icons — only
/// pick it if your terminal actually uses a Nerd Font, since it's never assumed.
/// `ascii` forces plain-text labels everywhere (maximum portability).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IconMode {
    #[default]
    Auto,
    Ascii,
    Unicode,
    Nerd,
}

/// Machine action-surface safety settings (the `[actions]` table; `[mcp]` is
/// accepted as an alias since these knobs predate the web action endpoint).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActionsConfig {
    /// Glob patterns restricting which commands `run` may execute. Empty allows
    /// any command (still gated by `--allow-run` + `confirm`).
    pub run_allowlist: Vec<String>,
    /// Cap on how many repos an action may target *unless* the selector
    /// explicitly says `{all: true}`. `0` disables the cap.
    pub max_action_targets: usize,
}

/// The historical name, kept so downstream code and docs migrate gradually.
pub type McpConfig = ActionsConfig;

impl Default for ActionsConfig {
    fn default() -> Self {
        Self {
            run_allowlist: Vec::new(),
            // A guard against an accidental broad selector fanning out across a
            // large fleet; `{all: true}` is the explicit escape hatch.
            max_action_targets: 50,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // No config yet: leave roots empty so the Scanner auto-detects where
            // the user actually keeps repos (the "super fleet") rather than
            // guessing a single directory. `cohors init` persists the detection.
            roots: Vec::new(),
            max_depth: 4,
            ignore: default_ignores(),
            aliases: BTreeMap::new(),
            groups: BTreeMap::new(),
            editor: None,
            stop_at_repo: true,
            follow_symlinks: false,
            actions: ActionsConfig::default(),
            icons: IconMode::default(),
            theme: ThemeMode::default(),
            updates: UpdatesMode::default(),
            unknown_keys: Vec::new(),
        }
    }
}

fn default_ignores() -> Vec<String> {
    ["**/node_modules/**", "**/.cargo/**", "**/vendor/**"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Recognized top-level keys, used to warn about anything else. `mcp` is the
/// accepted alias for `actions`. A test derives the canonical set from
/// `Config::default()` serialization, so this list can't silently drift.
const KNOWN_KEYS: &[&str] = &[
    "roots",
    "max_depth",
    "ignore",
    "aliases",
    "groups",
    "editor",
    "stop_at_repo",
    "follow_symlinks",
    "actions",
    "mcp",
    "icons",
    "theme",
    "updates",
];

/// Read a config file: `Ok(None)` when it does not exist (the caller decides
/// whether that is fine), a typed error for anything else (permissions, a
/// directory in its place).
fn read_config_file(path: &Utf8Path) -> Result<Option<String>, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ConfigError::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

/// The config for a file that is not there yet: defaults, noted in the log.
fn defaults_for_missing(path: &Utf8Path) -> Config {
    tracing::info!(
        %path,
        "no config file; using defaults (run `cohors init` to create one)"
    );
    Config::default()
}

/// The recognised key an unknown one is most likely a typo of, if any: the
/// nearest by edit distance, allowing one edit per four characters (at least
/// one), so `max_dept` → `max_depth` and `thme` → `theme`, but `colour_theme`
/// suggests nothing.
fn closest_known_key(unknown: &str) -> Option<&'static str> {
    let budget = (unknown.chars().count() / 4).max(1);
    KNOWN_KEYS
        .iter()
        .map(|known| (edit_distance(unknown, known), *known))
        .filter(|(distance, _)| *distance <= budget)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, known)| known)
}

/// Levenshtein distance (insertions, deletions, substitutions), by characters.
/// Keys are short, so the plain two-row dynamic programme is plenty.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut row = Vec::with_capacity(b.len() + 1);
        row.push(i + 1);
        for (j, cb) in b.iter().enumerate() {
            let substitute = prev[j] + usize::from(ca != *cb);
            row.push(substitute.min(prev[j + 1] + 1).min(row[j] + 1));
        }
        prev = row;
    }
    prev[b.len()]
}

impl Config {
    /// Load from an explicit path, the `COHORS_CONFIG` env var, or the default
    /// location — in that precedence order. Environment overrides (`COHORS_*`)
    /// apply on top.
    ///
    /// A missing file at the default location (or at `COHORS_CONFIG`) yields
    /// [`Config::default`]: zero-config use. A missing file at an *explicit*
    /// `path` is [`ConfigError::NotFound`], because the user named that file and
    /// silently scanning a different fleet would hide the typo. A front-end that
    /// is about to offer to create the file uses [`Config::load_or_default`].
    pub fn load(path: Option<&Utf8Path>) -> Result<Config, ConfigError> {
        Self::load_resolved(path, true)
    }

    /// [`Config::load`], except that a missing explicit file also yields
    /// defaults. For a surface that offers to *create* that very file (the
    /// dashboard's first-run greeting and empty-state rescue write to
    /// `--config`), where refusing to start would leave nothing to create it.
    pub fn load_or_default(path: Option<&Utf8Path>) -> Result<Config, ConfigError> {
        Self::load_resolved(path, false)
    }

    fn load_resolved(
        path: Option<&Utf8Path>,
        explicit_must_exist: bool,
    ) -> Result<Config, ConfigError> {
        let env_path = std::env::var("COHORS_CONFIG")
            .ok()
            .filter(|p| !p.is_empty());
        let (path, explicit) = match (path, env_path) {
            (Some(p), _) => (p.to_owned(), true),
            (None, Some(p)) => (p.into(), false),
            (None, None) => (crate::paths::config_file()?, false),
        };
        let mut config = match read_config_file(&path)? {
            Some(content) => Self::parse(&content, &path)?,
            None if explicit && explicit_must_exist => return Err(ConfigError::NotFound(path)),
            None => defaults_for_missing(&path),
        };
        config.apply_env(|key| std::env::var(key).ok());
        Ok(config)
    }

    /// Load from a specific file (no env overrides — callers that want them go
    /// through [`Config::load`]). A missing file is not an error — the user
    /// simply hasn't run `cohors init` yet, so they get defaults.
    pub fn load_from(path: &Utf8Path) -> Result<Config, ConfigError> {
        match read_config_file(path)? {
            Some(content) => Self::parse(&content, path),
            None => Ok(defaults_for_missing(path)),
        }
    }

    /// Parse config text, recording (and ignoring) unknown top-level keys and
    /// logging them plus any semantic validation warnings. Front-ends get the
    /// same list as sentences from [`Config::warnings`].
    pub fn parse(content: &str, path: &Utf8Path) -> Result<Config, ConfigError> {
        let mut config: Config = toml::from_str(content).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })?;

        // Re-parse as a generic table to spot unknown keys: serde silently
        // drops them, but the user probably made a typo worth flagging.
        if let Ok(table) = toml::from_str::<toml::Table>(content) {
            config.unknown_keys = table
                .keys()
                .filter(|k| !KNOWN_KEYS.contains(&k.as_str()))
                .cloned()
                .collect();
            if !config.unknown_keys.is_empty() {
                let unknown = &config.unknown_keys;
                tracing::warn!(?unknown, %path, "ignoring unknown config keys");
            }
        }

        for warning in config.validate() {
            tracing::warn!(%path, "{warning}");
        }
        Ok(config)
    }

    /// Top-level keys the loaded file held that cohors does not recognise
    /// (they were ignored). Empty for defaults and for a clean file.
    pub fn unknown_keys(&self) -> &[String] {
        &self.unknown_keys
    }

    /// Everything worth telling the user about this config, one sentence each:
    /// every unknown key (with a "did you mean" when it is a near miss of a real
    /// one, e.g. `max_dept` → `max_depth`), then the [`Config::validate`]
    /// warnings. Empty for a clean config. Front-ends print these; the same
    /// facts also go to the log file.
    pub fn warnings(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .unknown_keys
            .iter()
            .map(|key| match closest_known_key(key) {
                Some(known) => {
                    format!("unknown config key `{key}` (ignored; did you mean `{known}`?)")
                }
                None => format!("unknown config key `{key}` (ignored)"),
            })
            .collect();
        out.extend(self.validate());
        out
    }

    /// Semantic sanity checks that parsing can't catch. Warnings, never errors:
    /// a questionable config still runs, but the user is told why it may
    /// misbehave (the `cohors.log` shows these).
    pub fn validate(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if self.max_depth == 0 {
            warnings.push(
                "max_depth = 0 only ever inspects the roots themselves; repos in subdirectories won't be found".into(),
            );
        }
        for (name, globs) in &self.groups {
            if globs.is_empty() {
                warnings.push(format!("group `{name}` has no globs and will never match"));
            }
            if globs.iter().any(|g| g.trim().is_empty()) {
                warnings.push(format!("group `{name}` contains an empty glob"));
            }
        }
        if self.editor.as_deref().is_some_and(|e| e.trim().is_empty()) {
            warnings.push("`editor` is set to an empty string; it will be ignored".into());
        }
        if self.roots.iter().any(|r| r.trim().is_empty()) {
            warnings.push("`roots` contains an empty entry".into());
        }
        warnings
    }

    /// Apply `COHORS_*` environment overrides. `vars` is injected (a closure
    /// over `std::env::var` in production) so this stays directly testable.
    ///
    /// Supported: `COHORS_ROOTS` (comma-separated), `COHORS_EDITOR`,
    /// `COHORS_MAX_DEPTH`, `COHORS_ICONS` (auto|ascii|unicode|nerd). These exist
    /// for the places editing a file is awkward — agents, CI, containers.
    pub fn apply_env(&mut self, vars: impl Fn(&str) -> Option<String>) {
        if let Some(roots) = vars("COHORS_ROOTS") {
            let roots: Vec<String> = roots
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            if !roots.is_empty() {
                self.roots = roots;
            }
        }
        if let Some(editor) = vars("COHORS_EDITOR")
            && !editor.trim().is_empty()
        {
            self.editor = Some(editor);
        }
        if let Some(depth) = vars("COHORS_MAX_DEPTH")
            && let Ok(depth) = depth.trim().parse()
        {
            self.max_depth = depth;
        }
        if let Some(icons) = vars("COHORS_ICONS") {
            match icons.trim().to_ascii_lowercase().as_str() {
                "auto" => self.icons = IconMode::Auto,
                "ascii" => self.icons = IconMode::Ascii,
                "unicode" => self.icons = IconMode::Unicode,
                "nerd" => self.icons = IconMode::Nerd,
                other => tracing::warn!(%other, "ignoring unknown COHORS_ICONS value"),
            }
        }
        if let Some(theme) = vars("COHORS_THEME") {
            match theme.trim().to_ascii_lowercase().as_str() {
                "auto" => self.theme = ThemeMode::Auto,
                "dark" => self.theme = ThemeMode::Dark,
                "light" => self.theme = ThemeMode::Light,
                other => tracing::warn!(%other, "ignoring unknown COHORS_THEME value"),
            }
        }
    }

    /// The editor command to open repos with: config `editor`, then `$EDITOR`,
    /// then `$VISUAL`. Empty values are ignored.
    pub fn editor_command(&self) -> Option<String> {
        self.editor
            .clone()
            .or_else(|| std::env::var("EDITOR").ok())
            .or_else(|| std::env::var("VISUAL").ok())
            .filter(|s| !s.trim().is_empty())
    }

    /// Look up a configured alias for a repo, by absolute path (alias keys may
    /// use `~`, expanded via `home`) or by directory name. Returns the first
    /// match, or `None`.
    pub fn alias_for(&self, path: &Utf8Path, dir_name: &str, home: &Utf8Path) -> Option<String> {
        self.aliases.iter().find_map(|(key, alias)| {
            let matches = expand_tilde(key, home) == path.as_str() || key == dir_name;
            matches.then(|| alias.clone())
        })
    }
}

/// Expand a leading `~` or `~/...` to the home directory. Other forms (e.g.
/// `~user`) and inputs without a leading tilde are returned unchanged.
pub fn expand_tilde(pattern: &str, home: &Utf8Path) -> String {
    if pattern == "~" {
        return home.as_str().to_string();
    }
    if let Some(rest) = pattern.strip_prefix("~/") {
        return home.join(rest).as_str().to_string();
    }
    pattern.to_string()
}

/// Commented starter config written by `cohors init`.
pub const STARTER_CONFIG: &str = r#"# cohors configuration.
# Edit this file, then run `cohors`. All paths support `~` and globs.
# Env overrides for agents/CI: COHORS_CONFIG, COHORS_ROOTS (comma-separated),
# COHORS_EDITOR, COHORS_MAX_DEPTH, COHORS_ICONS, COHORS_THEME.

# Where cohors looks for git repos.
roots = ["~/projects"]

# How deep to descend from each root when searching for `.git`.
max_depth = 4

# Skip these (glob patterns, matched against the path).
ignore = ["**/node_modules/**", "**/.cargo/**", "**/vendor/**"]

# How to open a repo (Enter). Falls back to $EDITOR, then $VISUAL.
# editor = "code"   # e.g. "code", "nvim", "vim", "hx"

# Stop descending into a repo once found (don't look for nested repos).
stop_at_repo = true

# Follow symlinks during discovery (devcontainers/codespaces).
follow_symlinks = false

# Status glyphs in the dashboard: "auto" (Unicode glyphs, or ASCII when NO_COLOR
# is set), "ascii" (plain text everywhere), "unicode", or "nerd" (only if your
# terminal uses a Nerd Font — it's never assumed).
# icons = "auto"

# The terminal's background: "auto" asks the terminal, and a terminal that does
# not answer gets a look that works on any background. Say "dark" or "light" if
# yours cannot be asked and you want the filled panels.
# theme = "auto"

# Keeping cohors up to date: "auto" looks for a new release at most once a day
# and installs it in the background the way cohors was installed, then asks you
# to restart; "notify" only tells you; "off" never looks.
# updates = "auto"

# Optional pretty names, keyed by absolute path or repo dir name.
[aliases]
# "~/work/payments-service" = "payments"

# Optional groups: a name -> list of repo-name globs. cohors stamps each repo
# with every group it matches (by directory name), so any surface (TUI/MCP/CLI)
# can target a whole cluster at once.
[groups]
# payments = ["pay-*", "billing"]

# Safety knobs for the machine action surfaces (`cohors mcp` and the
# `cohors web --allow-writes/--allow-run` action endpoint). `[mcp]` is accepted
# as an alias for this table.
[actions]
# Restrict the `run` tool to commands matching these globs (empty = any command,
# still gated by --allow-run + confirm).
# run_allowlist = ["pnpm *", "npm *", "cargo *", "git *", "rg *", "grep *"]
# Cap on action targets unless the selector says {all = true}; 0 disables it.
max_action_targets = 50
"#;

/// Render the starter config with a concrete `roots` line. Empty `roots` keeps
/// the commented placeholder (so a no-detection `cohors init` still writes a
/// valid, edit-me file).
pub fn starter_config(roots: &[String]) -> String {
    if roots.is_empty() {
        return STARTER_CONFIG.to_string();
    }
    let list = roots
        .iter()
        .map(|r| format!("\"{r}\""))
        .collect::<Vec<_>>()
        .join(", ");
    STARTER_CONFIG.replace("roots = [\"~/projects\"]", &format!("roots = [{list}]"))
}

/// Write the starter config to `path`, seeded with `roots` (auto-detected by the
/// caller). Refuses to overwrite an existing file unless `force` is true, and
/// creates parent directories as needed.
pub fn write_starter(path: &Utf8Path, force: bool, roots: &[String]) -> Result<(), ConfigError> {
    if path.exists() && !force {
        return Err(ConfigError::AlreadyExists(path.to_owned()));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: parent.to_owned(),
            source,
        })?;
    }
    std::fs::write(path, starter_config(roots)).map_err(|source| ConfigError::Write {
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;

    #[test]
    fn defaults_are_sensible() {
        let c = Config::default();
        // Empty by design: the Scanner auto-detects roots when none are set.
        assert!(c.roots.is_empty());
        assert_eq!(c.max_depth, 4);
        assert!(c.stop_at_repo);
        assert!(!c.follow_symlinks);
        assert!(c.editor.is_none());
        assert!(!c.ignore.is_empty());
    }

    #[test]
    fn parse_overrides_only_provided_keys() {
        let toml = r#"
            roots = ["~/work", "~/oss"]
            max_depth = 2
            follow_symlinks = true
        "#;
        let c = Config::parse(toml, Utf8Path::new("test.toml")).expect("parse");
        assert_eq!(c.roots, ["~/work", "~/oss"]);
        assert_eq!(c.max_depth, 2);
        assert!(c.follow_symlinks);
        // Untouched keys keep their defaults.
        assert!(c.stop_at_repo);
        assert_eq!(c.ignore, default_ignores());
    }

    /// The drift guard: KNOWN_KEYS must exactly cover the serialized shape of
    /// `Config` (plus the `mcp` alias). Add a field without updating the list
    /// (or the starter template) and this fails.
    #[test]
    fn known_keys_match_the_config_shape_and_starter() {
        let serialized = toml::to_string(&Config::default()).expect("serialize default");
        let table: toml::Table = toml::from_str(&serialized).expect("round-trip");
        for key in table.keys() {
            assert!(
                KNOWN_KEYS.contains(&key.as_str()),
                "field `{key}` missing from KNOWN_KEYS"
            );
        }
        // Every canonical key (alias + Option fields excepted) appears in the
        // starter template so users can discover it.
        for key in KNOWN_KEYS {
            if *key == "mcp" {
                continue; // alias, deliberately not in the starter
            }
            assert!(
                STARTER_CONFIG.contains(key),
                "key `{key}` undocumented in STARTER_CONFIG"
            );
        }
    }

    #[test]
    fn actions_table_parses_under_both_names() {
        for table in ["actions", "mcp"] {
            let toml = format!("[{table}]\nrun_allowlist = [\"git *\"]\nmax_action_targets = 9\n");
            let c = Config::parse(&toml, Utf8Path::new("t.toml")).expect("parse");
            assert_eq!(c.actions.run_allowlist, ["git *"], "[{table}]");
            assert_eq!(c.actions.max_action_targets, 9, "[{table}]");
        }
    }

    #[test]
    fn parse_theme_mode() {
        let c = Config::parse("", Utf8Path::new("t.toml")).expect("parse");
        assert_eq!(c.theme, ThemeMode::Auto);
        for (s, want) in [
            ("auto", ThemeMode::Auto),
            ("dark", ThemeMode::Dark),
            ("light", ThemeMode::Light),
        ] {
            let c =
                Config::parse(&format!("theme = \"{s}\""), Utf8Path::new("t.toml")).expect("parse");
            assert_eq!(c.theme, want, "theme = {s:?}");
        }
        // The environment says it too, and nonsense changes nothing.
        let mut c = Config::default();
        c.apply_env(|key| (key == "COHORS_THEME").then(|| " Dark ".to_string()));
        assert_eq!(c.theme, ThemeMode::Dark);
        let mut c = Config::default();
        c.apply_env(|key| (key == "COHORS_THEME").then(|| "sepia".to_string()));
        assert_eq!(c.theme, ThemeMode::Auto);
    }

    #[test]
    fn parse_icon_mode() {
        let c = Config::parse("", Utf8Path::new("t.toml")).expect("parse");
        assert_eq!(c.icons, IconMode::Auto);
        for (s, want) in [
            ("auto", IconMode::Auto),
            ("ascii", IconMode::Ascii),
            ("unicode", IconMode::Unicode),
            ("nerd", IconMode::Nerd),
        ] {
            let c =
                Config::parse(&format!("icons = \"{s}\""), Utf8Path::new("t.toml")).expect("parse");
            assert_eq!(c.icons, want, "icons = {s:?}");
        }
    }

    #[test]
    fn parse_reads_aliases_and_groups() {
        let toml = r#"
            [aliases]
            "~/work/payments-service" = "payments"
            [groups]
            payments = ["pay-*", "billing"]
        "#;
        let c = Config::parse(toml, Utf8Path::new("test.toml")).expect("parse");
        assert_eq!(
            c.aliases.get("~/work/payments-service").map(String::as_str),
            Some("payments")
        );
        assert_eq!(c.groups.get("payments").map(|v| v.len()), Some(2));
    }

    #[test]
    fn unknown_keys_do_not_fail_parsing() {
        let toml = r#"
            roots = ["~/x"]
            colour_theme = "dracula"   # not a real key
        "#;
        let c = Config::parse(toml, Utf8Path::new("test.toml")).expect("parse");
        assert_eq!(c.roots, ["~/x"]);
        assert_eq!(c.unknown_keys(), ["colour_theme"]);
    }

    /// Unknown keys are kept (not just logged) so a front-end can warn, with a
    /// "did you mean" for a near miss and none for something unrelated.
    #[test]
    fn unknown_keys_become_warnings_with_suggestions() {
        let toml = "max_dept = 2\nthme = \"dark\"\ncolour_theme = \"dracula\"\nroots = []\n";
        let c = Config::parse(toml, Utf8Path::new("t.toml")).expect("parse");
        let mut keys = c.unknown_keys().to_vec();
        keys.sort();
        assert_eq!(keys, ["colour_theme", "max_dept", "thme"]);
        // The typo is ignored, not applied.
        assert_eq!(c.max_depth, Config::default().max_depth);

        let warnings = c.warnings();
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(
            warnings.contains(
                &"unknown config key `max_dept` (ignored; did you mean `max_depth`?)".to_string()
            ),
            "{warnings:?}"
        );
        assert!(
            warnings.contains(
                &"unknown config key `thme` (ignored; did you mean `theme`?)".to_string()
            ),
            "{warnings:?}"
        );
        assert!(
            warnings.contains(&"unknown config key `colour_theme` (ignored)".to_string()),
            "{warnings:?}"
        );
        // A clean config (and the defaults) have nothing to say.
        assert!(Config::default().warnings().is_empty());
        let clean = Config::parse(STARTER_CONFIG, Utf8Path::new("s.toml")).unwrap();
        assert!(clean.warnings().is_empty());
    }

    /// Semantic warnings (previously log-only) are part of the same list.
    #[test]
    fn warnings_include_validation_findings() {
        let c = Config::parse("max_depth = 0\n", Utf8Path::new("t.toml")).unwrap();
        let warnings = c.warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("max_depth = 0"), "{warnings:?}");
    }

    #[test]
    fn edit_distance_counts_single_edits() {
        assert_eq!(edit_distance("max_dept", "max_depth"), 1);
        assert_eq!(edit_distance("thme", "theme"), 1);
        assert_eq!(edit_distance("rots", "roots"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("same", "same"), 0);
        assert_eq!(closest_known_key("colour_theme"), None);
        assert_eq!(closest_known_key("icon"), Some("icons"));
    }

    #[test]
    fn malformed_toml_is_an_error() {
        let err = Config::parse("this is = = not toml", Utf8Path::new("bad.toml"));
        assert!(matches!(err, Err(ConfigError::Parse { .. })));
    }

    /// The parse error's *source* carries where and why (line, column, cause);
    /// callers that flatten the error must walk `source()` to keep it.
    #[test]
    fn malformed_toml_error_source_has_line_and_column() {
        let err = Config::parse("roots = [not valid toml", Utf8Path::new("bad.toml")).unwrap_err();
        assert_eq!(err.to_string(), "invalid TOML in bad.toml");
        let cause = std::error::Error::source(&err).expect("parse error has a source");
        let cause = cause.to_string();
        assert!(cause.contains("line 1"), "{cause}");
        assert!(cause.contains("column"), "{cause}");
    }

    /// A config file the user *named* that does not exist is an error naming
    /// the path and how to create it — not a silent fall back to defaults.
    #[test]
    fn explicit_missing_config_is_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = Utf8PathBuf::from_path_buf(dir.path().join("typo.toml")).expect("utf8 path");
        let err = Config::load(Some(&path)).unwrap_err();
        assert!(
            matches!(&err, ConfigError::NotFound(p) if *p == path),
            "{err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains(path.as_str()), "{msg}");
        assert!(
            msg.contains(&format!("cohors init --config {path}")),
            "{msg}"
        );
    }

    /// The lenient form, for a surface that offers to create the file: a
    /// missing explicit path yields defaults. A present file loads either way.
    #[test]
    fn load_or_default_tolerates_a_missing_explicit_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = Utf8PathBuf::from_path_buf(dir.path().join("new.toml")).expect("utf8 path");
        let c = Config::load_or_default(Some(&missing)).expect("missing → defaults");
        assert!(c.unknown_keys().is_empty());

        let present = Utf8PathBuf::from_path_buf(dir.path().join("cfg.toml")).expect("utf8 path");
        std::fs::write(&present, "max_dept = 3\n").unwrap();
        for c in [
            Config::load(Some(&present)).expect("strict load"),
            Config::load_or_default(Some(&present)).expect("lenient load"),
        ] {
            assert_eq!(c.unknown_keys(), ["max_dept"]);
        }
    }

    #[test]
    fn validate_flags_semantic_footguns() {
        let mut c = Config::default();
        assert!(c.validate().is_empty(), "defaults must be warning-free");
        c.max_depth = 0;
        c.groups.insert("empty".into(), Vec::new());
        c.editor = Some("  ".into());
        c.roots.push(String::new());
        let warnings = c.validate();
        assert_eq!(warnings.len(), 4, "{warnings:?}");
    }

    #[test]
    fn env_overrides_apply_and_ignore_garbage() {
        let mut c = Config::default();
        let vars = |key: &str| -> Option<String> {
            match key {
                "COHORS_ROOTS" => Some("~/a, ~/b ,".into()),
                "COHORS_EDITOR" => Some("hx".into()),
                "COHORS_MAX_DEPTH" => Some("7".into()),
                "COHORS_ICONS" => Some("ascii".into()),
                _ => None,
            }
        };
        c.apply_env(vars);
        assert_eq!(c.roots, ["~/a", "~/b"]);
        assert_eq!(c.editor.as_deref(), Some("hx"));
        assert_eq!(c.max_depth, 7);
        assert_eq!(c.icons, IconMode::Ascii);

        // Garbage values leave the config untouched.
        let mut c = Config::default();
        c.apply_env(|key| match key {
            "COHORS_MAX_DEPTH" => Some("not-a-number".into()),
            "COHORS_ICONS" => Some("sparkly".into()),
            "COHORS_ROOTS" => Some(" , ".into()),
            _ => None,
        });
        assert_eq!(c, Config::default());
    }

    #[test]
    fn editor_command_prefers_config_value() {
        let c = Config {
            editor: Some("hx".to_string()),
            ..Config::default()
        };
        assert_eq!(c.editor_command().as_deref(), Some("hx"));
    }

    #[test]
    fn expand_tilde_handles_home_forms() {
        let home = Utf8Path::new("/home/dev");
        assert_eq!(expand_tilde("~", home), "/home/dev");
        assert_eq!(expand_tilde("~/work/x", home), "/home/dev/work/x");
        assert_eq!(expand_tilde("/abs/path", home), "/abs/path");
        assert_eq!(expand_tilde("relative", home), "relative");
        assert_eq!(expand_tilde("~user/x", home), "~user/x"); // left alone
    }

    #[test]
    fn alias_for_matches_by_path_or_dir_name() {
        let mut aliases = BTreeMap::new();
        aliases.insert(
            "~/work/payments-service".to_string(),
            "payments".to_string(),
        );
        aliases.insert("infra".to_string(), "infrastructure".to_string());
        let c = Config {
            aliases,
            ..Config::default()
        };
        let home = Utf8Path::new("/home/dev");

        assert_eq!(
            c.alias_for(
                Utf8Path::new("/home/dev/work/payments-service"),
                "payments-service",
                home
            )
            .as_deref(),
            Some("payments")
        );
        assert_eq!(
            c.alias_for(Utf8Path::new("/somewhere/infra"), "infra", home)
                .as_deref(),
            Some("infrastructure")
        );
        assert_eq!(
            c.alias_for(Utf8Path::new("/x/unrelated"), "unrelated", home),
            None
        );
    }

    #[test]
    fn starter_config_is_valid_and_round_trips() {
        let c =
            Config::parse(STARTER_CONFIG, Utf8Path::new("starter.toml")).expect("starter parses");
        assert_eq!(c.roots, ["~/projects"]);
        assert!(c.stop_at_repo);
        assert!(c.validate().is_empty(), "starter must be warning-free");
    }

    #[test]
    fn write_starter_creates_then_refuses_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = Utf8PathBuf::from_path_buf(dir.path().join("nested").join("config.toml"))
            .expect("utf8 path");

        write_starter(&path, false, &[]).expect("first write");
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), STARTER_CONFIG);

        assert!(matches!(
            write_starter(&path, false, &[]),
            Err(ConfigError::AlreadyExists(_))
        ));

        write_starter(&path, true, &[]).expect("forced write");
    }

    #[test]
    fn load_from_missing_file_yields_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = Utf8PathBuf::from_path_buf(dir.path().join("absent.toml")).expect("utf8 path");
        let c = Config::load_from(&path).expect("missing → defaults");
        assert_eq!(c, Config::default());
    }
}
