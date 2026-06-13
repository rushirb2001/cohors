//! Typed errors for config loading and writing.

use camino::Utf8PathBuf;

/// Things that can go wrong loading or writing the config. The binary maps
/// these to user-facing messages; a missing config file is *not* an error
/// (callers get defaults instead).
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not determine the home directory")]
    NoHome,

    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(std::path::PathBuf),

    #[error("config file already exists at {0} (use --force to overwrite)")]
    AlreadyExists(Utf8PathBuf),

    #[error("failed to read config at {path}")]
    Read {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write config at {path}")]
    Write {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid TOML in {path}")]
    Parse {
        path: Utf8PathBuf,
        #[source]
        source: toml::de::Error,
    },
}
