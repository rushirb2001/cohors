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

Every report prints JSON when piped. Write commands (`fetch`, `pull`, `push`, `commit`, `stash`, `run`) exit 1 when any repo fails, so a CI step sees the failure.

## Staying up to date

An npm install is never changed in place, since a global npm folder often needs elevated rights. When a release is out, the dashboard tells you; update with:

```sh
npm install -g @cohors/cli@latest
```

`npx @cohors/cli@latest` always runs the newest release.

## Use it with your coding agent (MCP)

`cohors mcp` is a stdio MCP server: your agent gets every repo's state, a search across all of them, your recent commits and each repo's tasks.

Agents start MCP servers with a stripped `PATH`. The `cohors` that npm puts on your `PATH` is a Node script, so it needs `node` on that `PATH` too. Point the client at the binary inside the package instead, which needs nothing:

```sh
echo "$(npm root -g)/@cohors/cli/node_modules/.bin_real/cohors"
```

**Claude Code**

```sh
claude mcp add cohors -- "$(npm root -g)/@cohors/cli/node_modules/.bin_real/cohors" mcp
```

**Claude Desktop, Cursor and other clients** (`mcpServers`)

```json
{
  "mcpServers": {
    "cohors": {
      "command": "/opt/homebrew/lib/node_modules/@cohors/cli/node_modules/.bin_real/cohors",
      "args": ["mcp", "--root", "/Users/you/code"]
    }
  }
}
```

`npm root -g` prints the first part of that path on your machine. Without a global install, `"command": "npx", "args": ["-y", "@cohors/cli@latest", "mcp"]` works for a client whose `PATH` has both `npx` and `node`.

**What the agent may change.** Reads are always on. Each of these is off until you add it after `mcp`:

- `--allow-writes`: `fetch`, `pull`, `push`, `commit`, `stash`. Pull is fast-forward only and push never forces.
- `--allow-run`: one shell command in many repos at once, capped by your config's run allowlist.
- `--allow-launchpad`: writing a repo's `.cohors/repo.json`, your focus board and the activity log. Never git.

CI and pull-request status come from GitHub, through the GitHub CLI's token when `gh` is signed in, or `GITHUB_TOKEN`.

## New in 0.7.7

- Repos nested inside a repo are found, and `cohors discover` lists what a search finds without reading it. `--deep` searches every folder, and the dashboard and MCP server pick up what it finds.
- `fetch`, `pull`, `push`, `commit` and `stash` run on every selected repo at once.
- `commit` refuses a repo where a file that looks like a secret (`.env`, a private key) would go in, unless you pass `--allow-secrets`; `--staged` commits only what you staged.
- Write commands exit 1 when a repo fails, and a rejected push says why.

[All release notes](https://github.com/rushirb2001/cohors/releases) · [Documentation](https://github.com/rushirb2001/cohors#readme) · [Issues](https://github.com/rushirb2001/cohors/issues) · MIT licensed. No telemetry; the only connection cohors makes on its own is to GitHub's API.
