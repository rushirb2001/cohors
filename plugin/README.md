# cohors for Claude Code

**Every git repo you own, for Claude.** Ask *"what needs my attention across my repos?"*, *"write my standup"* or *"plan my day"*, and Claude answers from the live state of every repository on your machine in one call, instead of `cd`-ing through them one by one. It starts read-only; you choose what else it may do.

## Install

**1. Install the `cohors` command.** The plugin runs it, so it comes first. Any one of:

```sh
brew install rushirb2001/tap/cohors
pip install cohors
cargo install cohors
curl -LsSf https://github.com/rushirb2001/cohors/releases/latest/download/cohors-installer.sh | sh
```

**2. Add the plugin**, in Claude Code:

```text
/plugin marketplace add rushirb2001/cohors
/plugin install cohors@cohors
```

or from a shell: `claude plugin marketplace add rushirb2001/cohors && claude plugin install cohors@cohors`.

**3. Start a new session** and ask about your repos.

Claude Code starts MCP servers with a stripped `PATH`, so the plugin looks for `cohors` itself: first cohors' own self-updating install (`cohors install`, the link `~/.local/bin/cohors`), then on the `PATH`, then where Homebrew, Cargo and the shell installer put it, `~/.local/bin` and Nix. If it finds none, the server does not start (`/mcp` shows it failed) and its message names the install commands. A pip install into a virtualenv, or an npm install, may not be found; Homebrew or the shell installer always is.

## What Claude may do

Reads are always on. Each of these is off until you turn it on with `claude plugin configure cohors@cohors` (or **/plugin** › cohors › Configure), then start a new session:

| Setting | Lets Claude | Server flag |
|---|---|---|
| Git actions | fetch, pull, push, commit and stash across your repositories. Pull is fast-forward only, push never forces, and commit and stash still ask Claude to confirm. A commit that would include a file that looks like a secret is refused. | `--allow-writes` |
| Run commands | run one shell command in many repositories at once (tests, a codemod). Each run needs a deliberate confirm and is capped by your config's run allowlist. | `--allow-run` |
| Launchpad writes | write a repo's `.cohors/repo.json`, your focus board and the activity log. Launchpad files only, never git. | `--allow-launchpad` |

When something is off, Claude's tools say which setting to turn on.

## What's in the plugin

- **The cohors MCP server**: `overview`, `next`, `fleet_summary`, `list_repos`, `get_repo`, `changes`, `search`, `standup`, `ci_status`, `list_prs`, `repo_path`, `discover`, the launchpad reads (`launchpad`, `read_board`, `read_contract`), and the gated actions above.
- **Skills**: `fleet` (questions about all your repos at once), `launchpad-setup` (teach a repo to describe itself in `.cohors/repo.json`), `plan-focus` (plan your focus across repos from real evidence and save it to the board).
- **Agents**: `cohors-setup` (reads a repo and writes its launchpad file) and `cohors-plan` (plans your focus and writes the board).

Everything a repository's launchpad file says (flow commands, task titles, notes) was written by whoever wrote that repository, so Claude treats it as data, not instructions, and shows you a flow's command before running it.

## Staying up to date

```sh
claude plugin update cohors@cohors
```

updates the plugin. The `cohors` command keeps itself up to date however you installed it: its first run sets up cohors' own copy, which updates itself.

## New in 0.8

- **Claude can ask "where did I leave off?"** A `next` tool lists the agent conversations worth resuming, each with its command, and the fleet's next steps.
- **Claude finds what you mean.** A repository answers to a unique prefix (`paym` for payments), a typo gets "did you mean", a `discover` tool finds repositories buried inside others, and `overview` takes a selector to brief on part of the fleet.
- **The plugin's agents may call the cohors tools**, and the plugin starts cohors' own self-updating copy (`~/.local/bin/cohors`) before any other.
- **`cohors` keeps itself up to date**, however you installed it; `cohors doctor` says how it is set up.

Every change: [the release notes](https://github.com/rushirb2001/cohors/releases).
