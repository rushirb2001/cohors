//! The cohors configuration model: parse `config.toml`, fall back to sensible
//! defaults, and write a starter file for `cohors init`.

use std::collections::BTreeMap;

use camino::Utf8Path;
use serde::Deserialize;

use crate::error::ConfigError;

/// Resolved configuration.
///
/// Roots, ignore globs, and alias keys are stored exactly as written (`~` and
/// globs intact). Expansion happens later, at the discovery boundary, via
/// [`expand_tilde`] — that way a `Config` value needs no home directory and
/// [`Config::default`] can't fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Directories (or globs) to search for repos.
    pub roots: Vec<String>,
    /// How deep to descend from each root looking for `.git`.
    pub max_depth: usize,
    /// Glob patterns to skip during discovery.
    pub ignore: Vec<String>,
    /// Pretty names keyed by absolute path or repo directory name.
    pub aliases: BTreeMap<String, String>,
    /// Editor command; falls back to `$EDITOR`/`$VISUAL` when `None`.
    pub editor: Option<String>,
    /// Stop descending into a repo once `.git` is found (don't nest).
    pub stop_at_repo: bool,
    /// Follow symlinks during discovery.
    pub follow_symlinks: bool,
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
            editor: None,
            stop_at_repo: true,
            follow_symlinks: false,
        }
    }
}

fn default_ignores() -> Vec<String> {
    ["**/node_modules/**", "**/.cargo/**", "**/vendor/**"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Mirror of [`Config`] where every field is optional, so an absent key falls
/// back to the default rather than failing to parse. Unknown keys are ignored
/// by serde; [`Config::parse`] separately warns about them.
#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    roots: Option<Vec<String>>,
    max_depth: Option<usize>,
    ignore: Option<Vec<String>>,
    aliases: Option<BTreeMap<String, String>>,
    editor: Option<String>,
    stop_at_repo: Option<bool>,
    follow_symlinks: Option<bool>,
}

/// Recognized top-level keys, used to warn about anything else.
const KNOWN_KEYS: &[&str] = &[
    "roots",
    "max_depth",
    "ignore",
    "aliases",
    "editor",
    "stop_at_repo",
    "follow_symlinks",
];

impl RawConfig {
    fn into_config(self) -> Config {
        let d = Config::default();
        Config {
            roots: self.roots.unwrap_or(d.roots),
            max_depth: self.max_depth.unwrap_or(d.max_depth),
            ignore: self.ignore.unwrap_or(d.ignore),
            aliases: self.aliases.unwrap_or(d.aliases),
            editor: self.editor.or(d.editor),
            stop_at_repo: self.stop_at_repo.unwrap_or(d.stop_at_repo),
            follow_symlinks: self.follow_symlinks.unwrap_or(d.follow_symlinks),
        }
    }
}

impl Config {
    /// Load from an explicit path, or the default location when `path` is
    /// `None`. A missing file yields [`Config::default`].
    pub fn load(path: Option<&Utf8Path>) -> Result<Config, ConfigError> {
        let path = match path {
            Some(p) => p.to_owned(),
            None => crate::paths::config_file()?,
        };
        Self::load_from(&path)
    }

    /// Load from a specific file. A missing file is not an error — the user
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

    /// Parse config text, warning about (and ignoring) unknown top-level keys.
    pub fn parse(content: &str, path: &Utf8Path) -> Result<Config, ConfigError> {
        let raw: RawConfig = toml::from_str(content).map_err(|source| ConfigError::Parse {
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

        Ok(raw.into_config())
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

# Optional pretty names, keyed by absolute path or repo dir name.
[aliases]
# "~/work/payments-service" = "payments"
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

    #[test]
    fn parse_reads_aliases_table() {
        let toml = r#"
            [aliases]
            "~/work/payments-service" = "payments"
        "#;
        let c = Config::parse(toml, Utf8Path::new("test.toml")).expect("parse");
        assert_eq!(
            c.aliases.get("~/work/payments-service").map(String::as_str),
            Some("payments")
        );
    }

    #[test]
    fn unknown_keys_do_not_fail_parsing() {
        let toml = r#"
            roots = ["~/x"]
            colour_theme = "dracula"   # not a real key
        "#;
        // Should parse fine (the key is ignored, with a warning logged).
        let c = Config::parse(toml, Utf8Path::new("test.toml")).expect("parse");
        assert_eq!(c.roots, ["~/x"]);
    }

    #[test]
    fn malformed_toml_is_an_error() {
        let err = Config::parse("this is = = not toml", Utf8Path::new("bad.toml"));
        assert!(matches!(err, Err(ConfigError::Parse { .. })));
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

        // By tilde-expanded absolute path.
        assert_eq!(
            c.alias_for(
                Utf8Path::new("/home/dev/work/payments-service"),
                "payments-service",
                home
            )
            .as_deref(),
            Some("payments")
        );
        // By directory name.
        assert_eq!(
            c.alias_for(Utf8Path::new("/somewhere/infra"), "infra", home)
                .as_deref(),
            Some("infrastructure")
        );
        // No match.
        assert_eq!(
            c.alias_for(Utf8Path::new("/x/unrelated"), "unrelated", home),
            None
        );
    }

    #[test]
    fn starter_config_is_valid_and_round_trips() {
        // The template must itself parse into a Config.
        let c =
            Config::parse(STARTER_CONFIG, Utf8Path::new("starter.toml")).expect("starter parses");
        assert_eq!(c.roots, ["~/projects"]);
        assert!(c.stop_at_repo);
    }

    #[test]
    fn write_starter_creates_then_refuses_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = Utf8PathBuf::from_path_buf(dir.path().join("nested").join("config.toml"))
            .expect("utf8 path");

        // First write succeeds and creates parent dirs.
        write_starter(&path, false, &[]).expect("first write");
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), STARTER_CONFIG);

        // Second write without force is refused.
        assert!(matches!(
            write_starter(&path, false, &[]),
            Err(ConfigError::AlreadyExists(_))
        ));

        // With force it overwrites.
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
