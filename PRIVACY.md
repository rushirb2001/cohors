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
- **Where new versions of cohors come from**, to see whether a newer cohors
  exists. One of these, depending on how your copy updates:
  - cohors' own install, the shell installer, or a build from source:
    GitHub's releases API,
    `api.github.com/repos/rushirb2001/cohors/releases/latest`. A copy
    installed by Homebrew, pip, uv, npm or cargo sets cohors' own install up
    on its first run (see "What it writes") and from then on asks here too;
  - until then, or with `updates = "notify"` (which never sets it up), that
    copy asks its own registry: Homebrew's tap formula, read raw from
    `raw.githubusercontent.com/rushirb2001/homebrew-tap/main/Formula/cohors.rb`;
    PyPI's JSON API, `pypi.org/pypi/cohors/json` (pip, pipx, uv); the npm
    registry, `registry.npmjs.org/@cohors%2fcli` (npm, npx); or crates.io's
    index, `index.crates.io/co/ho/cohors` (`cargo install`).

  The dashboard asks when it opens and every few minutes while it is open; a
  command you run at a terminal asks in the background while it runs; the
  MCP server asks while it runs. Asks closer together than two minutes share
  one answer, and each is a conditional request, so while nothing has changed
  the answer is an empty "not modified". The requests carry no token, no
  cookie and nothing about you or your repositories: only cohors' version and
  its project address, in the User-Agent.
- **GitHub's release downloads** (`github.com/rushirb2001/cohors/releases/download/…`,
  which redirect to GitHub's release file servers,
  `release-assets.githubusercontent.com` or `objects.githubusercontent.com`),
  only to install a newer cohors: the release archive for your platform and
  its published SHA-256, which cohors checks before using the download. No
  token, and the same User-Agent. When updates are automatic, cohors' own
  install fetches these by itself. A Homebrew, pip or shell-installer copy
  that has not set it up yet is updated by that tool (Homebrew, pip, or the
  release's own installer, which downloads from the same place and checks the
  same checksums); cohors never runs a package manager once its own install
  is set up. A source build is never updated for you, only told the command.

Turn update checks off with `updates = "off"` in the config (or `"notify"` to
be told without anything being installed or set up), or set
`COHORS_NO_UPDATE=1`; `COHORS_DISABLE_UPDATES=1` also blocks `cohors update`
and `cohors install`. They never run in CI.

Nothing else. With no GitHub token and updates off, cohors makes no network
connections at all, apart from the git actions you start.

## What it writes, all on your machine

- A cache of the last scan, so it opens instantly next time.
- What each update check found (the newest version and the answer's ETag),
  and the last update attempt, so a repeated check costs an empty "not
  modified" and `cohors doctor` can say what happened.
- cohors' own copy, one folder per version under
  `~/.local/share/cohors/versions/` (the newest two and the one running are
  kept), and a link to the active one at `~/.local/bin/cohors` (never over a
  file cohors did not make). A copy installed by Homebrew, pip, uv, npm or
  cargo sets this up on its first run by copying its own binary there (no
  download), says so once, and from then on hands control to that copy
  whenever it is newer; `cohors install` sets it up by hand. A note of that
  setup, and of the last hand-off that could not happen, stays in
  `~/.local/state/cohors/`. `cohors uninstall` removes all of it. Not with
  `updates = "off"` or `"notify"`, `COHORS_NO_UPDATE`,
  `COHORS_DISABLE_UPDATES`, in CI, or from a build from source.
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
