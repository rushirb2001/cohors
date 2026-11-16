# cohors

**Every git repo you own, on one screen, and an MCP server that gives your coding agent the same view.** This package installs the prebuilt `cohors` binary, so there is no Rust toolchain to set up.

## Install

```sh
pip install cohors          # or: pipx install cohors
cohors                      # finds your repos and opens the dashboard
```

Run it without installing, with [uv](https://docs.astral.sh/uv/):

```sh
uvx cohors@latest
```

Wheels exist for macOS (Apple Silicon and Intel) and Linux with glibc 2.35 or newer (x86_64 and ARM64). Elsewhere pip finds no wheel; build it with `cargo install cohors` instead. cohors needs `git`.

With no config, `cohors` finds the folders that hold your repos by itself. Point it at one folder with `cohors --root ~/code`, or run `cohors init` to write a config you can edit. To try it on sample repositories first, run `cohors demo`.

## Staying up to date

| Installed with | What happens when a release comes out |
|---|---|
| `pip install cohors` | The first run sets up cohors' own copy in `~/.local/share/cohors` and hands over to it from then on. That copy looks for a release when it starts and every few minutes, downloads it, checks its SHA-256 and switches to it; the dashboard says **Updated to X — restart to use it**. `cohors update` does it now, and `cohors doctor` says how your copy is set up. |
| `pipx install cohors` or `uv tool install cohors` | The same as pip. |
| `uvx` | `uvx cohors@latest` always runs the newest release. |

Turn it off with `updates = "notify"` (tell only, set nothing up) or `updates = "off"` in the config, or `COHORS_NO_UPDATE=1`; `COHORS_DISABLE_UPDATES=1` blocks every update.

## Use it with your coding agent (MCP)

`cohors mcp` is a stdio MCP server: your agent gets every repo's state, a search across all of them, your commits for a standup, and each repo's tasks. Run `cohors mcp` once in a terminal and it prints the exact lines to register it, with this install's absolute path filled in.

Use absolute paths. Agents start MCP servers with a stripped `PATH`, so a bare `cohors` or `uvx` often resolves to nothing and the agent silently gets no tools. `which cohors` or `which uvx` prints the path to use.

**Claude Code**

```sh
claude mcp add cohors -- "$(which cohors)" mcp
```

or, with nothing installed:

```sh
claude mcp add cohors -- "$(which uvx)" cohors@latest mcp
```

**Claude Desktop, Cursor and other clients** (`mcpServers` in `claude_desktop_config.json`, `~/.cursor/mcp.json`, …)

```json
{
  "mcpServers": {
    "cohors": {
      "command": "/Users/you/.local/bin/uvx",
      "args": ["cohors@latest", "mcp", "--root", "/Users/you/code"]
    }
  }
}
```

With `pip install cohors`, use `"command": "/path/from/which/cohors"` and `"args": ["mcp"]`.

**What the agent may change.** Reads are always on. Everything else is off until you add the flag after `mcp`:

| Flag | Turns on |
|---|---|
| `--allow-writes` | `fetch`, `pull`, `push`, `commit`, `stash`. Pull is fast-forward only and push never forces. |
| `--allow-run` | `run`: one shell command in many repos at once, capped by your config's run allowlist. |
| `--allow-launchpad` | Writing a repo's `.cohors/repo.json`, your focus board and the activity log. Never git. |

CI and pull-request status come from GitHub. cohors uses the GitHub CLI's token when `gh` is signed in, or `GITHUB_TOKEN` (add `"env": {"GITHUB_TOKEN": "…"}` to the block above).

## What you get

- **`cohors`**: the dashboard. Branch, ahead/behind, uncommitted work, stashes, CI and pull requests for every repo, with what needs you first.
- **`cohors overview`, `scan`, `standup`, `discover`**: the same answers as reports, or JSON when piped.
- **`cohors fetch | pull | push | commit | stash | run --select dirty`**: act on many repos at once. Add `--dry-run` to see what would happen first.
- **`cohors web`**: the dashboard in your browser. `cohors web --demo` shows it on sample data.

<!-- whats-new:start -->
## What's new in 0.7.7

### Added

- **Repos nested inside a repo are found.**
- **`cohors discover`** lists every repo a search finds without reading them.
- **`--deep` on `cohors discover` and `cohors scan`** searches every folder at any depth, inside repos too.
- **`commit` shows what it will commit and won't commit a secret.**
- **A finished look for the start screen and every plain-terminal view**.
- **A Docker image, `ghcr.io/rushirb2001/cohors`**, for linux/amd64 and linux/arm64.

### Changed

- **`fetch`, `pull`, `push`, `commit` and `stash` work on every selected repo at once** (the bounded pool the MCP server and the web already use), each line printed as its repo finishes.

### Fixed

- **Write commands report failure in their exit status.**
- **A rejected push says why**.
- **`run --timeout` stops the whole job**, including what the command started (`a; b`, `npm test`, a backgrounded process), and Ctrl-C stops every running job instead of leaving them behind.
- **Config mistakes are no longer silent.**
- **Selectors**: a typo lists the whole vocabulary, and `branch:` takes globs (`branch:feat/*`).
- **`scan --select`** says how many repos were scanned and how many matched, and lists clean matches by name.
- **`cohors agent command`** no longer edits the repo's tracked `.gitignore`.
- `overview` figures read in the singular for one.
- Discovery never lists a bare repository found inside another repo's working tree (a committed one is attacker-controlled), and `cohors discover` keeps control characters in folder names off the terminal.

Full notes: [cohors 0.7.7 on GitHub](https://github.com/rushirb2001/cohors/releases/tag/v0.7.7).
<!-- whats-new:end -->

## Links

[Documentation](https://github.com/rushirb2001/cohors#readme) · [Changelog](https://github.com/rushirb2001/cohors/blob/main/CHANGELOG.md) · [Issues](https://github.com/rushirb2001/cohors/issues) · [Privacy](https://github.com/rushirb2001/cohors/blob/main/PRIVACY.md) (no telemetry; the only connection cohors makes on its own is to GitHub's API)

MIT licensed.

<!-- Ownership marker for the official MCP Registry. Keep this line. -->
mcp-name: io.github.rushirb2001/cohors
