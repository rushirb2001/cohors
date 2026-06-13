//! Where cohors keeps its files.
//!
//! Per MVP-SPEC, config lives at `~/.config/cohors/config.toml` and honors
//! `$XDG_CONFIG_HOME`; the cache/log live under `~/.cache/cohors/`. We use
//! XDG-style paths on *all* platforms (including macOS) rather than the
//! `directories` crate's platform-native locations — see ADR-011. The actual
//! XDG-vs-home decision is a pure function so it's easy to unit-test without
//! touching the environment.

use crate::error::ConfigError;
use camino::{Utf8Path, Utf8PathBuf};

/// Application subdirectory name used under the config/cache roots.
const APP: &str = "cohors";

/// Absolute path to the config file: `<config_dir>/config.toml`.
pub fn config_file() -> Result<Utf8PathBuf, ConfigError> {
    Ok(config_dir()?.join("config.toml"))
}

/// The directory holding `config.toml`.
pub fn config_dir() -> Result<Utf8PathBuf, ConfigError> {
    let home = home_dir()?;
    Ok(config_dir_from(
        &home,
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
    ))
}

/// The directory holding the snapshot cache and log.
pub fn cache_dir() -> Result<Utf8PathBuf, ConfigError> {
    let home = home_dir()?;
    Ok(cache_dir_from(
        &home,
        std::env::var("XDG_CACHE_HOME").ok().as_deref(),
    ))
}

/// Absolute path to the snapshot cache: `<cache_dir>/cache.json`.
pub fn cache_file() -> Result<Utf8PathBuf, ConfigError> {
    Ok(cache_dir()?.join("cache.json"))
}

/// Absolute path to the log file: `<cache_dir>/cohors.log`.
pub fn log_file() -> Result<Utf8PathBuf, ConfigError> {
    Ok(cache_dir()?.join("cohors.log"))
}

/// The user's home directory, as UTF-8. Uses the `directories` crate so this
/// works on Windows (`%USERPROFILE%`) as well as Unix.
fn home_dir() -> Result<Utf8PathBuf, ConfigError> {
    let base = directories::BaseDirs::new().ok_or(ConfigError::NoHome)?;
    Utf8PathBuf::from_path_buf(base.home_dir().to_path_buf()).map_err(ConfigError::NonUtf8Path)
}

/// Pure XDG-style config-dir resolution: `$XDG_CONFIG_HOME/cohors` if set and
/// non-empty, else `<home>/.config/cohors`.
fn config_dir_from(home: &Utf8Path, xdg_config_home: Option<&str>) -> Utf8PathBuf {
    match xdg_config_home.filter(|s| !s.is_empty()) {
        Some(x) => Utf8Path::new(x).join(APP),
        None => home.join(".config").join(APP),
    }
}

/// Pure XDG-style cache-dir resolution: `$XDG_CACHE_HOME/cohors` if set and
/// non-empty, else `<home>/.cache/cohors`.
fn cache_dir_from(home: &Utf8Path, xdg_cache_home: Option<&str>) -> Utf8PathBuf {
    match xdg_cache_home.filter(|s| !s.is_empty()) {
        Some(x) => Utf8Path::new(x).join(APP),
        None => home.join(".cache").join(APP),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_prefers_xdg_when_set() {
        let home = Utf8PathBuf::from("/home/dev");
        assert_eq!(
            config_dir_from(&home, Some("/custom/cfg")),
            Utf8PathBuf::from("/custom/cfg/cohors")
        );
    }

    #[test]
    fn config_dir_falls_back_to_dot_config() {
        let home = Utf8PathBuf::from("/home/dev");
        assert_eq!(
            config_dir_from(&home, None),
            Utf8PathBuf::from("/home/dev/.config/cohors")
        );
        // Empty string is treated as unset.
        assert_eq!(
            config_dir_from(&home, Some("")),
            Utf8PathBuf::from("/home/dev/.config/cohors")
        );
    }

    #[test]
    fn cache_dir_prefers_xdg_then_dot_cache() {
        let home = Utf8PathBuf::from("/home/dev");
        assert_eq!(
            cache_dir_from(&home, Some("/custom/cache")),
            Utf8PathBuf::from("/custom/cache/cohors")
        );
        assert_eq!(
            cache_dir_from(&home, None),
            Utf8PathBuf::from("/home/dev/.cache/cohors")
        );
    }
}
