# @cohors/cli

**The `cohors` command, from npm: every git repo you own on one screen, and an MCP server that gives your coding agent the same view.**

The package name is `@cohors/cli` (npm refuses the bare name `cohors`); the command it installs is `cohors`. When it installs, it downloads the prebuilt binary for your machine from the matching [GitHub release](https://github.com/rushirb2001/cohors/releases), so there is nothing to compile. Prebuilt for macOS (Apple Silicon and Intel) and Linux (x86_64 and ARM64). cohors needs `git`.

## Install

```sh
npm install -g @cohors/cli
cohors                       # finds your repos and opens the dashboard
```

Run it once without installing:

```sh
npx @cohors/cli@latest               # the dashboard
npx @cohors/cli@latest overview      # what to do next, as a report
```

In a project, as a dev dependency for scripts and CI:

```sh
npm install -D @cohors/cli
npx cohors scan --select dirty --json
```

Every report prints JSON when piped (`--report` prints the report itself instead, as plain text). Write commands (`fetch`, `pull`, `push`, `commit`, `stash`, `run`) exit 1 when any repo fails, so a CI step sees the failure.

## Staying up to date

cohors keeps itself up to date however you installed it. Its first run sets up cohors' own copy in `~/.local/share/cohors` (with `~/.local/bin/cohors` linked to it) and from then on hands over to the newest version there, the way Claude Code does: it looks for a release when it starts and every few minutes, downloads it, checks its SHA-256, tries it, and switches to it. The npm package itself is never changed in place (a global npm folder often needs elevated rights); `npm install -g @cohors/cli@latest` updates it by hand, and `npx @cohors/cli@latest` always runs the newest release.

`cohors doctor` says how your copy is set up; `updates = "off"` in the config, or `COHORS_DISABLE_UPDATES=1`, turns updating off.

## Use it with your coding agent (MCP)

`cohors mcp` is a stdio MCP server: your agent gets every repo's state, a search across all of them, your recent commits and each repo's tasks.

Agents start MCP servers with a stripped `PATH`, so register cohors by its absolute path. The `cohors` that npm puts on your `PATH` is a Node script that needs `node` there too; use the copy cohors sets up for itself instead. Run `cohors` once: its first run links `~/.local/bin/cohors`, a native binary that needs no Node and keeps itself up to date (`cohors doctor` prints where it is).

**Claude Code**

```sh
claude mcp add --scope user cohors -- "$HOME/.local/bin/cohors" mcp
```

**Claude Desktop, Cursor and other clients** (`mcpServers`)

```json
{
  "mcpServers": {
    "cohors": {
      "command": "/Users/you/.local/bin/cohors",
      "args": ["mcp", "--root", "/Users/you/code"]
    }
  }
}
```

With updates turned off (`updates = "off"` or `"notify"`, or `COHORS_DISABLE_UPDATES`), no copy is set up: use the binary inside the npm package, `"$(npm root -g)/@cohors/cli/node_modules/.bin_real/cohors"`, which npm updates only when you reinstall. Without a global install, `"command": "npx", "args": ["-y", "@cohors/cli@latest", "mcp"]` works for a client whose `PATH` has both `npx` and `node`.

**What the agent may change.** Reads are always on. Each of these is off until you add it after `mcp`:

- `--allow-writes`: `fetch`, `pull`, `push`, `commit`, `stash`. Pull is fast-forward only and push never forces.
- `--allow-run`: one shell command in many repos at once, capped by your config's run allowlist.
- `--allow-launchpad`: writing a repo's `.cohors/repo.json`, your focus board and the activity log. Never git.

CI and pull-request status come from GitHub, through the GitHub CLI's token when `gh` is signed in, or `GITHUB_TOKEN`.

## New in 0.8

- **cohors keeps itself up to date, however you installed it.** Its first run sets up cohors' own copy in `~/.local/share/cohors`, with `~/.local/bin/cohors` linked to it; that copy looks for a release when it starts and every few minutes, checks its SHA-256 and switches to it. `cohors doctor` says how your copy is set up, and `cohors install` sets it up by hand.
- **Commands named for what you want, in the same words as the MCP tools:** `cohors status`, `show`, `path`, `changes`, `search`, `prs`, `next` (what to do now, and which agent session to resume) and `watch --json` (one JSON line per change).
- **A command that changes repositories asks first.** At a terminal, `fetch`, `pull`, `push`, `commit`, `stash` and `run` list the repositories they would change and ask `[y/N]`; `--yes` skips the question, and a script is never asked.
- **Exit codes a script can branch on:** 0 done, 1 something failed or nothing matched, 2 a usage mistake, 130 interrupted.
- **Shell completions** for bash, zsh, fish, elvish and PowerShell: `cohors completions zsh > ~/.zfunc/_cohors`.
- **The dashboard** answers the mouse, fits a narrow terminal, and teaches one key a day.

Every change: [the release notes](https://github.com/rushirb2001/cohors/releases).
