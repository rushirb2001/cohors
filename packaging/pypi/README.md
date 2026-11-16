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

Use absolute paths. Agents start MCP servers with a stripped `PATH`, so a bare `cohors` or `uvx` often resolves to nothing and the agent silently gets no tools. After a `pip`, `pipx` or `uv tool` install, run `cohors` once: its first run links `~/.local/bin/cohors`, the copy that keeps itself up to date (`cohors doctor` prints where it is). Register that one.

**Claude Code**

```sh
claude mcp add --scope user cohors -- "$HOME/.local/bin/cohors" mcp
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

With `pip install cohors`, use `"command": "/Users/you/.local/bin/cohors"` and `"args": ["mcp"]`; with updates turned off (no copy set up), the path `which cohors` prints.

**What the agent may change.** Reads are always on. Everything else is off until you add the flag after `mcp`:

| Flag | Turns on |
|---|---|
| `--allow-writes` | `fetch`, `pull`, `push`, `commit`, `stash`. Pull is fast-forward only and push never forces. |
| `--allow-run` | `run`: one shell command in many repos at once, capped by your config's run allowlist. |
| `--allow-launchpad` | Writing a repo's `.cohors/repo.json`, your focus board and the activity log. Never git. |

CI and pull-request status come from GitHub. cohors uses the GitHub CLI's token when `gh` is signed in, or `GITHUB_TOKEN` (add `"env": {"GITHUB_TOKEN": "…"}` to the block above).

## What you get

- **`cohors`**: the dashboard. Branch, ahead/behind, uncommitted work, stashes, CI and pull requests for every repo, with what needs you first.
- **`cohors overview`, `scan`, `standup`, `discover`**: the same answers as reports, or JSON when piped (`--report` keeps the report on a pipe).
- **`cohors fetch | pull | push | commit | stash | run --select dirty`**: act on many repos at once. Add `--dry-run` to see what would happen first.
- **`cohors web`**: the dashboard in your browser. `cohors web --demo` shows it on sample data.

## New in 0.8

- **cohors keeps itself up to date, however you installed it.** Its first run sets up cohors' own copy in `~/.local/share/cohors`, with `~/.local/bin/cohors` linked to it; that copy looks for a release when it starts and every few minutes, checks its SHA-256 and switches to it. `cohors doctor` says how your copy is set up, and `cohors install` sets it up by hand.
- **Commands named for what you want, in the same words as the MCP tools:** `cohors status`, `show`, `path`, `changes`, `search`, `prs`, `next` (what to do now, and which agent session to resume) and `watch --json` (one JSON line per change).
- **A command that changes repositories asks first.** At a terminal, `fetch`, `pull`, `push`, `commit`, `stash` and `run` list the repositories they would change and ask `[y/N]`; `--yes` skips the question, and a script is never asked.
- **Exit codes a script can branch on:** 0 done, 1 something failed or nothing matched, 2 a usage mistake, 130 interrupted.
- **Shell completions** for bash, zsh, fish, elvish and PowerShell: `cohors completions zsh > ~/.zfunc/_cohors`.
- **The dashboard** answers the mouse, fits a narrow terminal, and teaches one key a day.

Every change: [the release notes](https://github.com/rushirb2001/cohors/releases).

<!-- whats-new:start -->
## What's new in 0.8.21

### Fixed

- **The package pages say what is new in 0.8**, not 0.7.7, on npm, PyPI, crates.io and the Claude Code plugin's page.
- **Every page's MCP setup points at the copy that keeps itself up to date** (`~/.local/bin/cohors`, registered for every folder with `--scope user`), with each channel's own path for when updates are off.
- **Two names in this changelog read right again**.

Full notes: [cohors 0.8.21 on GitHub](https://github.com/rushirb2001/cohors/releases/tag/v0.8.21).
<!-- whats-new:end -->

## Links

[Documentation](https://github.com/rushirb2001/cohors#readme) · [Changelog](https://github.com/rushirb2001/cohors/blob/main/CHANGELOG.md) · [Issues](https://github.com/rushirb2001/cohors/issues) · [Privacy](https://github.com/rushirb2001/cohors/blob/main/PRIVACY.md) (no telemetry; the only connection cohors makes on its own is to GitHub's API)

MIT licensed.

<!-- Ownership marker for the official MCP Registry. Keep this line. -->
mcp-name: io.github.rushirb2001/cohors
