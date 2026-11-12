# Privacy

cohors runs on your computer and keeps what it learns there. It has no
account, no server of its own, no analytics and no telemetry. It never sends
your code, your file names or your usage anywhere on its own.

This covers every part of cohors: the terminal app, the `cohors mcp` server,
the Claude Code plugin, the browser dashboard and the VS Code extension.

## What it reads

- **Your git repositories**, under the folders you point it at: branches,
  status, commits, stashes and changed files. It reads them with git, on your
  machine.
- **Each repository's `.cohors/` launchpad file**, if it has one.
- **Its own config**, at `~/.config/cohors/config.toml` (or your platform's
  equivalent).

## What it connects to

- **GitHub's API (`api.github.com`)**, only for repositories with a GitHub
  remote, and only when a token is available (from `gh auth` or
  `GITHUB_TOKEN`). It asks for CI status, open pull requests, issues,
  contributors and releases. The token is sent only to GitHub, never written to
  any file cohors creates, and never passed to the browser page.
- **Your own git remotes**, only when you, or an agent you have allowed to,
  run fetch, pull or push. Git makes these connections, with your existing
  credentials.
- **cohors' own latest release on GitHub**
  (`api.github.com/repos/rushirb2001/cohors/releases/latest`), at most once a
  day, to see whether a newer cohors exists. The request carries no token and
  nothing about you or your repositories, only cohors' version in its
  User-Agent. When a newer release exists and updates are automatic, cohors
  installs it with the tool that installed it: Homebrew, pip, or the release's
  own installer, downloaded from GitHub and checksum-verified. Turn this off
  with `updates = "off"` in the config (or `"notify"` to only be told), or set
  `COHORS_NO_UPDATE=1`. It never runs in CI.

Nothing else. With no GitHub token and updates off, cohors makes no network
connections at all, apart from the git actions you start.

## What it writes, all on your machine

- A cache of the last scan, so it opens instantly next time.
- What the last update check found, so it asks GitHub at most once a day.
- An audit log of every action it carries out (fetch, pull, push, commit,
  stash, run), with the repositories it touched.
- The launchpad's focus board and agent activity log.
- A repository's `.cohors/repo.json`, only when you, or an agent you have
  allowed to, write one.

Delete cohors' config, cache and data folders and nothing of it is left.

## The MCP server and coding agents

`cohors mcp` answers your coding agent (Claude Code or another MCP client) over
standard input and output on your machine. What your agent then does with those
answers, including sending them to its model provider, is governed by that
agent's own privacy policy, not this one. The server starts read-only. Git
actions, commands and launchpad writes each need a setting you turn on.

## The browser dashboard

`cohors web` serves the dashboard from your machine on `127.0.0.1` only. It
refuses requests that come from other websites or other hostnames, so a page
you visit elsewhere cannot read your fleet through it.

## Questions

Open an issue at <https://github.com/rushirb2001/cohors/issues>.
