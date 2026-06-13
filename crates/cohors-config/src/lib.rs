//! `cohors-config` — load and write cohors's TOML configuration and resolve
//! discovery settings: roots, ignore globs, aliases, editor, and the
//! config/cache paths (via the `directories` crate, respecting
//! `$XDG_CONFIG_HOME`).
//!
//! This crate is native-only. The binary loads config here and feeds the
//! resolved settings into the adapters; the adapters never read TOML
//! themselves.
//!
//! Scaffold for now; config loading lands in its milestone step.
#![forbid(unsafe_code)]
