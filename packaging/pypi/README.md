# cohors

**A governed control plane for all your git repositories: for you, and your coding agent.**

Coding agents made single-repo work cheap. The bottleneck moved to the *fleet*: an agent is blind across your 20 repos, and so are you past `cd`-ing through a dozen terminal tabs. cohors is one fast view of every repo, a search that indexes the whole fleet, and bulk actions an agent can preview before you ever grant write access. In the terminal, in the browser, and over MCP. One Rust core.

## Install

```sh
pip install cohors   # prebuilt binary, no Rust toolchain
cohors               # auto-detects your repos and launches the dashboard
```

With no config, `cohors` scans every folder it finds git repos in (your "super fleet"). Point it at a single folder with `cohors --root ~/code`, or run `cohors init` to save an editable config.

`pip install cohors` bundles a prebuilt binary for macOS (Apple Silicon / Intel) and Linux (x86_64 / ARM64). On other platforms, build from source: `cargo install --git https://github.com/rushirb2001/cohors cohors-tui`.

## What you get

- **Dashboard** (`cohors`): every repo on one screen (branch, ahead/behind, dirty state, stashes, CI, open PRs), sorted by what needs you. Bulk fetch / pull / run a command across the ones you pick, with a dry-run preview first.
- **Browser** (`cohors web`): the same scan, in your browser. Try it instantly with `cohors web --demo`.
- **MCP server** (`cohors mcp`): gives your coding agent (Claude Code, Cursor, ...) the same fleet-wide sight. Read-only by default; every write is gated and previewable, so it can't fan out across your machine on a fumbled command.

One core powers all three, so the terminal, the browser, and your agent always agree.

## Links

Full docs, the web dashboard, and MCP setup: **https://github.com/rushirb2001/cohors**

MIT licensed.

<!-- Ownership marker for the official MCP Registry. -->
mcp-name: io.github.rushirb2001/cohors
