//! Typed errors for config loading and writing.

use camino::Utf8PathBuf;

/// Things that can go wrong loading or writing the config. The binary maps
/// these to user-facing messages. A missing file at the *default* location is
/// not an error (callers get defaults: zero-config use); a missing file the
/// user named explicitly is [`ConfigError::NotFound`].
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not determine the home directory")]
    NoHome,

    /// An explicitly named config file (`--config <path>`) does not exist.
    /// Falling back to defaults here would hide a typo behind a different
    /// fleet, so it is an error that says how to create the file.
    #[error("no config file at {0} (create it with `cohors init --config {0}`)")]
    NotFound(Utf8PathBuf),

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

    #[error("could not edit config at {path}: {message}")]
    Edit { path: Utf8PathBuf, message: String },
}
