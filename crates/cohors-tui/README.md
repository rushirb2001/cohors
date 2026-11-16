# cohors

**Every git repo you own on one screen**: a terminal dashboard, a CLI for scripts, a browser dashboard and an MCP server for your coding agent, all in one binary. What's unpushed, what's behind, what's about to be lost, and what to do next, across every repository on your machine.

## Install from crates.io

```sh
cargo install cohors
```

This compiles cohors (Rust 1.96 or newer). The browser dashboard is already built into the crate, so `cohors web` works without Trunk or a WebAssembly toolchain. At run time cohors needs `git`.

Compiling takes a few minutes. Homebrew, pip, npm and a shell installer ship the same release prebuilt: see [Install](https://github.com/rushirb2001/cohors#install).

## Staying up to date

cohors keeps itself up to date however you installed it. Its first run sets up cohors' own copy in `~/.local/share/cohors` (with `~/.local/bin/cohors` linked to it) and from then on hands over to the newest version there, the way Claude Code does: it looks for a release when it starts and every few minutes, downloads it, checks its SHA-256, tries it, and switches to it. The crates.io install itself is never rebuilt for you (that means compiling); `cargo install cohors` updates it by hand.

`cohors doctor` says how your copy is set up; `updates = "off"` in the config, or `COHORS_DISABLE_UPDATES=1`, turns updating off.

## Use it

```sh
cohors                          # the dashboard; finds your repos by itself
cohors --root ~/code            # only the repos under one folder
cohors init                     # write a config you can edit
cohors demo                     # try it on sample repositories
```

Every view is also a command that prints a report at a terminal and JSON when piped: `cohors overview`, `cohors scan --select dirty`, `cohors standup`, `cohors discover`.

Act on many repos at once with a selector, and preview first with `--dry-run`:

```sh
cohors fetch --select all
cohors pull --select behind --dry-run
cohors run --select 'name:api-*' 'cargo test'
```

Pull is fast-forward only, nothing force-pushes, and a write command exits 1 when any repo fails.

## Use it with your coding agent (MCP)

`cohors mcp` is a stdio MCP server. Register it with the absolute path: agents start servers with a stripped `PATH`, where `~/.cargo/bin` is usually missing. Run `cohors mcp` in a terminal and it prints these lines with your path filled in.

```sh
claude mcp add cohors -- "$HOME/.cargo/bin/cohors" mcp
```

```json
{
  "mcpServers": {
    "cohors": { "command": "/Users/you/.cargo/bin/cohors", "args": ["mcp", "--root", "/Users/you/code"] }
  }
}
```

Reads are always on. `--allow-writes` (fetch, pull, push, commit, stash), `--allow-run` (one command across many repos) and `--allow-launchpad` (each repo's `.cohors/repo.json`, the focus board, the activity log) are each off until you add them after `mcp`.

## New in 0.7.7

- Repos nested inside a repo are found. `cohors discover` lists what a search finds without reading it, and `--deep` on `discover` and `scan` searches every folder; a running dashboard, `cohors web` and the MCP server pick up what it finds.
- `fetch`, `pull`, `push`, `commit` and `stash` run on every selected repo at once.
- `commit --dry-run` lists each repo's files, and a repo where a file that looks like a secret would go in is refused unless you pass `--allow-secrets`. `--staged` commits only what you staged.
- Write commands exit 1 when a repo fails, a rejected push says why, and config mistakes (an unknown key, a `--root` that names no folder) are reported instead of ignored.

[Full release notes](https://github.com/rushirb2001/cohors/releases)

## The crates

This crate is the `cohors` binary. The pieces it is built from are published too, versioned together with it:

| Crate | What it is |
|---|---|
| [`cohors-core`](https://crates.io/crates/cohors-core) | Pure, WASM-safe domain logic: the repo model, the selector language, attention and the overview. The one meant for reuse. |
| `cohors-config`, `cohors-git`, `cohors-github`, `cohors-actions`, `cohors-launchpad`, `cohors-fleet`, `cohors-mcp`, `cohors-web` | The adapters and front-ends this binary is assembled from. Their APIs follow the binary's needs. |

[Documentation](https://github.com/rushirb2001/cohors#readme) · [Changelog](https://github.com/rushirb2001/cohors/blob/main/CHANGELOG.md) · [Issues](https://github.com/rushirb2001/cohors/issues) · MIT licensed. No telemetry; the only connection cohors makes on its own is to GitHub's API.
