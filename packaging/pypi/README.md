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

## New in 0.9

- **One repository, with its worktrees as the rows.** Inside a repository, `cohors --root .` lists its checkouts: the main one and each linked worktree, with what each holds that is on no remote, the agent session that ran there, and `P` to push its branch. A first run typed inside such a repository opens it.
- **Work a coding agent left in a worktree counts.** Commits and files sitting in a linked worktree are read with their repository, count as work at risk, and get a step of their own. In a fleet, the repository under the cursor lists its worktrees beneath its row.
- **In the dashboard, fetch, pull and push ask before they run**, as commit and stash do: what will happen, to how many repositories, and `y` to go ahead.
- **Smaller answers for a coding agent.** The MCP tools answer in brief by default (`detail: "full"` for every field), and `--report` prints the plain report to a pipe, with nothing cut.
- **A note when a Claude Code session starts** in a repository with work left behind, from the plugin's one hook. One setting turns it off.
- **Opening the dashboard installs a newer release first**, and the notice about one shows on every screen.

## New in 0.8

- **cohors keeps itself up to date, however you installed it.** Its first run sets up cohors' own copy in `~/.local/share/cohors`, with `~/.local/bin/cohors` linked to it; that copy looks for a release when it starts and every few minutes, checks its SHA-256 and switches to it. `cohors doctor` says how your copy is set up, and `cohors install` sets it up by hand.
- **Commands named for what you want, in the same words as the MCP tools:** `cohors status`, `show`, `path`, `changes`, `search`, `prs`, `next` (what to do now, and which agent session to resume) and `watch --json` (one JSON line per change).
- **A command that changes repositories asks first.** At a terminal, `fetch`, `pull`, `push`, `commit`, `stash` and `run` list the repositories they would change and ask `[y/N]`; `--yes` skips the question, and a script is never asked.
- **Exit codes a script can branch on:** 0 done, 1 something failed or nothing matched, 2 a usage mistake, 130 interrupted.
- **Shell completions** for bash, zsh, fish, elvish and PowerShell: `cohors completions zsh > ~/.zfunc/_cohors`.
- **The dashboard** answers the mouse, fits a narrow terminal, and teaches one key a day.

Every change: [the release notes](https://github.com/rushirb2001/cohors/releases).

<!-- whats-new:start -->
## What's new in 0.9.2

### Fixed

- **A brief MCP answer left out what a worktree's row holds.**
- **The package pages say what is new in 0.9**, on npm, PyPI, crates.io, the container registry, the editor extension's page and the Claude Code plugin's.
- The README said a step's letter acts on every repository in it, and that only commit and stash ask first in the dashboard.
- A linked worktree listed as a row read "never fetched" beside a main checkout fetched a minute earlier.
- A session held in a worktree kept outside its repository's folder matched no repository and was left out of `cohors next`.
- **`cohors next` broke its lines in the middle of words.**
- **A report read through a pipe lost reasons.**
- **Failures did not all look like cohors'.**
- **One long step squeezed every step's reason.**
- **A repository with several reasons lost its path.**
- **`cohors doctor` had a line nobody could read.**
- **Commits shared by two worktrees were counted twice.**
- **`cohors demo` did not show work left in worktrees.**
- **"pass --root" was the advice to someone who had just passed it.**
- **`cohors --help` on a pipe carried colour codes.**
- The step about worktrees said that removing one deletes what was never pushed.
- **The dashboard opened on the old version when a newer one was out.**
- **The notice about a newer cohors was missing from two screens.**
- **An update being installed took up to five minutes to offer the restart.**
- The notice in the header uses plain characters on a plain-text terminal.
- **The dashboard repairs its own screen.**
- **Git and ssh can no longer ask a question across the dashboard.**

### Added

- **On a fleet, the repository under the cursor opens in place.**
- **One repository, with its worktrees as the rows.**
- **A first run inside a repository opens that repository.**
- **The agent session behind a worktree.**
- **A note when a session starts where work was left behind.**
- **"Session notes"** in the plugin's settings turns it off (`claude plugin configure cohors@cohors`).
- **`touched`** on each linked worktree in the scan JSON.
- **cohors sees the work left in linked worktrees.**
- **`cohors show`** lists a repo's worktrees with what each holds and its folder.
- **`--report` prints the report a person reads even when the output is piped**, as plain text with no colour codes.

### Changed

- When the rows are one repository's checkouts, every surface reads them that way.
- A worktree's row does not repeat what belongs to its repository.
- **In the dashboard, fetch, pull and push ask before they run.**
- **A worktree's folder is no longer counted as an uncommitted file of the repo around it.**
- **A worktree stays a part of its repo**, not a repo of its own.
- **What `cohors mcp` tells an agent is several times smaller.**
- **`detail: "full"` on `list_repos`, `overview`, `standup` and `next` returns everything**, field for field what `cohors scan --json` and `cohors overview --json` print, and `fields` on `list_repos` now takes `branches`.

Full notes: [cohors 0.9.2 on GitHub](https://github.com/rushirb2001/cohors/releases/tag/v0.9.2).
<!-- whats-new:end -->

## Links

[Documentation](https://github.com/rushirb2001/cohors#readme) · [Changelog](https://github.com/rushirb2001/cohors/blob/main/CHANGELOG.md) · [Issues](https://github.com/rushirb2001/cohors/issues) · [Privacy](https://github.com/rushirb2001/cohors/blob/main/PRIVACY.md) (no telemetry; the only connection cohors makes on its own is to GitHub's API)

MIT licensed.

<!-- Ownership marker for the official MCP Registry. Keep this line. -->
mcp-name: io.github.rushirb2001/cohors
