//! `cohors-config` — load and write cohors's TOML configuration and resolve
//! the file locations it uses.
//!
//! - [`Config`] is the resolved settings (roots, ignore globs, aliases, editor,
//!   discovery flags), with sensible [defaults](Config::default) when no file
//!   exists.
//! - [`paths`] resolves the config/cache/log locations (XDG-style; see ADR-011).
//! - [`write_starter`] backs `cohors init`.
//!
//! This crate is native-only: the binary loads config here and feeds the
//! resolved settings into the adapters, which never read TOML themselves.
#![forbid(unsafe_code)]

mod config;
mod error;
pub mod paths;

pub use config::{Config, STARTER_CONFIG, expand_tilde, starter_config, write_starter};
pub use error::ConfigError;
