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

Every view is also a command that prints a report at a terminal and JSON when piped: `cohors overview`, `cohors scan --select dirty`, `cohors standup`, `cohors discover`. `--report` prints the report to a pipe too, as plain text.

Act on many repos at once with a selector, and preview first with `--dry-run`:

```sh
cohors fetch --select all
cohors pull --select behind --dry-run
cohors run --select 'name:api-*' 'cargo test'
```

Pull is fast-forward only, nothing force-pushes, and a write command exits 1 when any repo fails.

## Use it with your coding agent (MCP)

`cohors mcp` is a stdio MCP server. Register it with an absolute path: agents start servers with a stripped `PATH`, where `~/.cargo/bin` is usually missing. Run `cohors` once and its first run links `~/.local/bin/cohors`, the copy that keeps itself up to date (`cohors doctor` prints where it is); register that one.

```sh
claude mcp add --scope user cohors -- "$HOME/.local/bin/cohors" mcp
```

```json
{
  "mcpServers": {
    "cohors": { "command": "/Users/you/.local/bin/cohors", "args": ["mcp", "--root", "/Users/you/code"] }
  }
}
```

With updates turned off (no copy set up), use `$HOME/.cargo/bin/cohors` instead.

Reads are always on. `--allow-writes` (fetch, pull, push, commit, stash), `--allow-run` (one command across many repos) and `--allow-launchpad` (each repo's `.cohors/repo.json`, the focus board, the activity log) are each off until you add them after `mcp`.

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

## The crates

This crate is the `cohors` binary. The pieces it is built from are published too, versioned together with it:

| Crate | What it is |
|---|---|
| [`cohors-core`](https://crates.io/crates/cohors-core) | Pure, WASM-safe domain logic: the repo model, the selector language, attention and the overview. The one meant for reuse. |
| `cohors-config`, `cohors-git`, `cohors-github`, `cohors-actions`, `cohors-launchpad`, `cohors-fleet`, `cohors-mcp`, `cohors-web` | The adapters and front-ends this binary is assembled from. Their APIs follow the binary's needs. |

[Documentation](https://github.com/rushirb2001/cohors#readme) · [Changelog](https://github.com/rushirb2001/cohors/blob/main/CHANGELOG.md) · [Issues](https://github.com/rushirb2001/cohors/issues) · MIT licensed. No telemetry; the only connection cohors makes on its own is to GitHub's API.
