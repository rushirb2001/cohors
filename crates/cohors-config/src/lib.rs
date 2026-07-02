//! `cohors-config` — load and write cohors's TOML configuration and resolve
//! the file locations it uses.
//!
//! - [`Config`] is the resolved settings (roots, ignore globs, aliases, editor,
//!   discovery flags), with sensible [defaults](Config::default) when no file
//!   exists.
//! - [`paths`] resolves the config/cache/log locations (XDG-style; see ADR-011).
//! - [`write_starter`] backs `cohors init`.
//! - [`editors`] detects installed editors on `PATH`; [`prefs`] persists the
//!   small picker-chosen preferences. Both are config-shaped resolution
//!   ("which editor should open this repo?") that used to live in the TUI.
//!
//! This crate is native-only: the binary loads config here and feeds the
//! resolved settings into the adapters, which never read TOML themselves.
#![forbid(unsafe_code)]

mod config;
pub mod editors;
mod error;
pub mod paths;
pub mod prefs;

pub use config::{
    Config, IconMode, McpConfig, STARTER_CONFIG, expand_tilde, starter_config, write_starter,
};
pub use error::ConfigError;
