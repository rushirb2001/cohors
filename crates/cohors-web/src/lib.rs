//! cohors-web — native server half.
//!
//! On the host this crate is the `cohors web` HTTP server (see [`server`]); the
//! `cohors` binary launches it via [`serve`], and the crate's own binary
//! ([`standalone`]) is a thin self-contained launcher. On `wasm32` the lib is
//! empty — there the crate's bin is the Leptos browser page instead.

#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
mod server;
#[cfg(not(target_arch = "wasm32"))]
pub use server::{Caps, ScanFn, serve};

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

    use crate::{Caps, serve};

    /// Parse `args` (everything after argv[0]) and run the server.
    ///
    /// Usage: `cohors-web <dist_dir> [--port N] [--allow-writes] [--allow-run] <root>...`
    pub fn main(args: Vec<String>) -> Result<()> {
        let mut dist: Option<Utf8PathBuf> = None;
        let mut port: u16 = 8787;
        let mut caps = Caps::default();
        let mut roots: Vec<String> = Vec::new();

        let mut it = args.into_iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--allow-writes" => caps.allow_writes = true,
                "--allow-run" => caps.allow_run = true,
                "--port" => {
                    port = it.next().and_then(|p| p.parse().ok()).unwrap_or(port);
                }
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
        if roots.is_empty() {
            bail!("no roots given — pass one or more directories to scan");
        }

        // The shared fleet Scanner: same config → discovery → groups/aliases
        // path as the TUI and MCP, with the given roots as explicit overrides.
        // This is the whole point of `cohors-fleet` — the standalone server no
        // longer hand-rolls a lesser scan (it now gets ignore globs, groups,
        // aliases, and the GitHub token for enrichment, exactly like `cohors web`).
        let scanner = Arc::new(
            Scanner::new(None, &roots).map_err(|e| anyhow::anyhow!("building scanner: {e}"))?,
        );
        let token = scanner.github_token();
        let served_roots = scanner.roots();
        let mcp = scanner.mcp_config();
        let scan = Arc::new(move || scanner.scan());

        serve(
            &dist,
            port,
            scan,
            served_roots,
            token,
            false,
            caps,
            mcp.run_allowlist,
            mcp.max_action_targets,
        )
    }
}
