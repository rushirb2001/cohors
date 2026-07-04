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
    /// Named groups → repo name globs (e.g. `payments = ["sushrutalgs-*"]`). A
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
            // `~` (the whole home) is too broad; default to ~/projects and nudge
            // the user toward `cohors init`.
            roots: vec!["~/projects".to_string()],
            max_depth: 4,
            ignore: default_ignores(),
            aliases: BTreeMap::new(),
            groups: BTreeMap::new(),
            editor: None,
            stop_at_repo: true,
            follow_symlinks: false,
            actions: ActionsConfig::default(),
            icons: IconMode::default(),
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
];

impl Config {
    /// Load from an explicit path, the `COHORS_CONFIG` env var, or the default
    /// location — in that precedence order. A missing file yields
    /// [`Config::default`]. Environment overrides (`COHORS_*`) apply on top.
    pub fn load(path: Option<&Utf8Path>) -> Result<Config, ConfigError> {
        let env_path = std::env::var("COHORS_CONFIG")
            .ok()
            .filter(|p| !p.is_empty());
        let path = match (path, env_path) {
            (Some(p), _) => p.to_owned(),
            (None, Some(p)) => p.into(),
            (None, None) => crate::paths::config_file()?,
        };
        let mut config = Self::load_from(&path)?;
        config.apply_env(|key| std::env::var(key).ok());
        Ok(config)
    }

    /// Load from a specific file (no env overrides — callers that want them go
    /// through [`Config::load`]). A missing file is not an error — the user
    /// simply hasn't run `cohors init` yet, so they get defaults.
    pub fn load_from(path: &Utf8Path) -> Result<Config, ConfigError> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(
                    %path,
                    "no config file; using defaults (run `cohors init` to create one)"
                );
                return Ok(Config::default());
            }
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        Self::parse(&content, path)
    }

    /// Parse config text, warning about (and ignoring) unknown top-level keys
    /// and logging any semantic validation warnings.
    pub fn parse(content: &str, path: &Utf8Path) -> Result<Config, ConfigError> {
        let config: Config = toml::from_str(content).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })?;

        // Re-parse as a generic table to spot unknown keys: serde silently
        // drops them, but the user probably made a typo worth flagging.
        if let Ok(table) = toml::from_str::<toml::Table>(content) {
            let unknown: Vec<&str> = table
                .keys()
                .map(String::as_str)
                .filter(|k| !KNOWN_KEYS.contains(k))
                .collect();
            if !unknown.is_empty() {
                tracing::warn!(?unknown, %path, "ignoring unknown config keys");
            }
        }

        for warning in config.validate() {
            tracing::warn!(%path, "{warning}");
        }
        Ok(config)
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
# COHORS_EDITOR, COHORS_MAX_DEPTH, COHORS_ICONS.

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

# Optional pretty names, keyed by absolute path or repo dir name.
[aliases]
# "~/work/payments-service" = "payments"

# Optional groups: a name -> list of repo-name globs. cohors stamps each repo
# with every group it matches (by directory name), so any surface (TUI/MCP/CLI)
# can target a whole cluster at once.
[groups]
# payments = ["sushrutalgs-*", "billing"]

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
        assert_eq!(c.roots, ["~/projects"]);
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
            payments = ["sushrutalgs-*", "billing"]
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
    }

    #[test]
    fn malformed_toml_is_an_error() {
        let err = Config::parse("this is = = not toml", Utf8Path::new("bad.toml"));
        assert!(matches!(err, Err(ConfigError::Parse { .. })));
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
