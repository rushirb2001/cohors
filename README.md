<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/readme/cohors-dark.png">
  <img src="assets/readme/cohors-light.png" alt="cohors" width="420">
</picture>

### Every git repo you own, on one screen.

What's unpushed, what's behind, and what to do next, across every repository on your machine.

[![CI](https://github.com/rushirb2001/cohors/actions/workflows/ci.yml/badge.svg)](https://github.com/rushirb2001/cohors/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/tag/rushirb2001/cohors?label=release&sort=semver&color=brightgreen)](https://github.com/rushirb2001/cohors/releases)
[![Homebrew](https://img.shields.io/badge/dynamic/regex?url=https%3A%2F%2Fraw.githubusercontent.com%2Frushirb2001%2Fhomebrew-tap%2Fmain%2FFormula%2Fcohors.rb&search=version%20%22(%5B%5E%22%5D%2B)%22&replace=%241&label=homebrew&color=FBB040)](https://github.com/rushirb2001/homebrew-tap)
[![PyPI](https://img.shields.io/pypi/v/cohors?label=pypi&color=3775A9)](https://pypi.org/project/cohors/)
[![npm](https://img.shields.io/npm/v/%40cohors%2Fcli?label=npm&color=CB3837)](https://www.npmjs.com/package/@cohors/cli)
[![crates.io](https://img.shields.io/crates/v/cohors?label=crates.io&color=E57324)](https://crates.io/crates/cohors)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange)](https://www.rust-lang.org/)

<kbd>&nbsp;[Install](#install)&nbsp;</kbd>&ensp;
<kbd>&nbsp;[Terminal](#use-it-in-the-terminal)&nbsp;</kbd>&ensp;
<kbd>&nbsp;[Claude Code](#use-it-with-claude-code)&nbsp;</kbd>&ensp;
<kbd>&nbsp;[Scripts](#use-it-from-scripts)&nbsp;</kbd>&ensp;
<kbd>&nbsp;[VS Code & browser](#use-it-in-vs-code-and-the-browser)&nbsp;</kbd>&ensp;
<kbd>&nbsp;[Launchpad](#the-launchpad)&nbsp;</kbd>

<img src="assets/demo.gif" alt="The cohors dashboard. Pressing P on Do next pushes every commit that exists only on this machine." width="100%">

</div>

## Install

**Homebrew**
```sh
brew install rushirb2001/tap/cohors
```

**pip**
```sh
pip install cohors
```

**npm**
```sh
npm install -g @cohors/cli
```

**Cargo**
```sh
cargo install cohors
```

**Shell installer**
```sh
curl -LsSf https://github.com/rushirb2001/cohors/releases/latest/download/cohors-installer.sh | sh
```

**Run once, without installing**
```sh
uvx cohors@latest
```
```sh
npx @cohors/cli@latest
```

Then run `cohors`. To try it on sample repositories first, run `cohors demo`.

Prebuilt for macOS and Linux; Cargo builds it anywhere Rust runs. All it needs is `git`. The examples below are real output from the `cohors demo` sample fleet, trimmed where marked `…`.

## What it answers

| You wonder | cohors shows |
|---|---|
| What exists only on this laptop? | Commits never pushed and branches never published, by repo |
| What should I do first? | Next steps, most urgent first, each with the command that does it |
| What's about to be lost? | Every repo at risk, why, and the fix |
| What did I ship this week? | Your commits across every repo, ready for standup |
| What's waiting in each repo? | Open tasks and the commands worth running |

**Where it looks.** With no config, the folders where code usually lives. Point it at yours with `cohors --root ~/code` (repeat for more), or save them with `cohors init`, which writes `~/.config/cohors/config.toml` for roots, aliases, groups and ignore rules.

## Use it in the terminal

```sh
cohors
```

<img src="assets/terminal.png" alt="The cohors terminal dashboard" width="100%">

**Do next** lists what to do, most urgent first. Press a step's letter to act on every repo in it, or mark repos with <kbd>Space</kbd> and press the letter yourself.

| Key | Does | Key | Does |
|---|---|---|---|
| <kbd>P</kbd> <kbd>p</kbd> <kbd>f</kbd> | Push, pull, fetch | <kbd>Enter</kbd> | Open a repo |
| <kbd>c</kbd> <kbd>S</kbd> | Commit, stash (asks first) | <kbd>/</kbd> | Filter as you type |
| <kbd>F</kbd> | Fetch everything | <kbd>s</kbd> <kbd>Y</kbd> | Standup, copy it |
| <kbd>1</kbd>–<kbd>5</kbd> | Jump to a card | <kbd>t</kbd> | Tasks across repos |
| <kbd>:</kbd> | Command line | <kbd>?</kbd> | Every key |

From the <kbd>:</kbd> line: `:commit wip: cart drawer` commits the marked repos, `:!npm test` runs a command in them, `:jump payments` goes to one. `cohors --watch` keeps it all live.

## Use it with Claude Code

```text
/plugin marketplace add rushirb2001/cohors
```
```text
/plugin install cohors@cohors
```

Ask *"what needs my attention across my repos?"* and Claude gets the whole fleet in one call:

```json
{
  "headline": "13 repositories. 6 have work at risk, 2 are clean.",
  "actions": [
    { "severity": "risk", "title": "Push 4 commits from 3 repositories" },
    { "severity": "warn", "title": "Reconcile 1 diverged branch" },
    { "severity": "warn", "title": "Publish 1 branch that has no upstream" },
    …
  ]
}
```

More to ask:

| Say | Claude uses |
|---|---|
| "What have I changed in web-app?" | `changes`, with the diff if you want it |
| "Where do we handle retries?" | `search`, across every repo at once |
| "Write my standup for this week." | `standup` |
| "Back up everything that's only on this laptop." | a `push` dry run first, then the push once you agree |
| "Which of my PRs are failing CI?" | `ci_status` and `list_prs` (needs a GitHub token or `gh` login) |
| "Plan my day." | the `plan-focus` skill, which saves a short list to your board |

It starts **read-only**. Turn on what Claude may do with `claude plugin configure cohors@cohors`, then start a new session:

| Setting | Allows | Still guarded by |
|---|---|---|
| Git actions | `fetch` `pull` `push` `commit` `stash` | no force push, fast-forward-only pull, `confirm` on commit and stash |
| Run commands | one command across many repos | `confirm`, a per-call cap |
| Launchpad writes | each repo's `.cohors/` file and your focus board | validated first, never touches git |

Or let cohors brief Claude for you: `cohors agent run plan-day`, or `cohors agent run task --repo payments --task idempotency-keys` to start on one task (run it again to resume the same conversation).

<details>
<summary>Other MCP clients</summary>
<br>

```json
{ "command": "uvx", "args": ["cohors@latest", "mcp"] }
{ "command": "npx", "args": ["-y", "@cohors/cli@latest", "mcp"] }
```

Or with cohors installed: `claude mcp add cohors -- "$(which cohors)" mcp`, adding `--allow-writes`, `--allow-run` or `--allow-launchpad`. Listed in the [MCP Registry](https://registry.modelcontextprotocol.io/v0/servers?search=io.github.rushirb2001/cohors) as `io.github.rushirb2001/cohors`.

</details>

## Use it from scripts

Every view is a command. At a terminal it prints a report; piped, it prints JSON.

```text
$ cohors overview
  13 repositories. 6 have work at risk, 2 are clean.

  Do next
     RISK  Push 4 commits from 3 repositories
           They exist only on this machine: analytics,
           design-system and payments.
           cohors push --select 'ahead,!diverged'
     WARN  Reconcile 1 diverged branch
           cohors scan --select diverged
     NOTE  Pull 2 repositories that are behind
           cohors pull --select 'behind,!diverged'
     …

$ cohors push --select ahead --dry-run
  Dry run
  would push 4 repos, nothing will change
    ▸ mobile-app     ~/code/mobile-app
    ▸ payments       ~/code/payments
    ▸ design-system  ~/code/design-system
    ▸ analytics      ~/code/analytics
```

`--select` picks the repos. Join terms with a comma (all must match) and put `!` in front to exclude:

| Term | Matches |
|---|---|
| `dirty` `ahead` `behind` `diverged` | Repos in that state |
| `unpublished` `stale-fetch` `attention` | No upstream, not fetched lately, anything needing a look |
| `name:pay*` `ci:failing` | By name, or by CI result |
| `all` | Every repo |

```sh
cohors fetch --select all                      # refresh every remote
cohors run --select 'name:web-*' 'npm test'    # one command, many repos
cohors standup                                 # this week's commits, as Markdown
cohors overview --json | jq -r '.actions[].title'
```

## Use it in VS Code and the browser

<table>
<tr>
<td width="50%"><img src="assets/vscode.png" alt="cohors in VS Code"></td>
<td width="50%"><img src="assets/web.png" alt="cohors in the browser"></td>
</tr>
<tr>
<td valign="top">

The cohors icon in the activity bar shows the fleet as native panes. Push, pull and fetch from any step, run a repo's commands in a terminal, or start Claude on a task.

</td>
<td valign="top">

`cohors web --root ~/code --watch` serves the dashboard on localhost, with pull requests and CI. Read-only unless you add `--allow-writes`. `cohors web --demo` shows the sample fleet.

</td>
</tr>
</table>

## The launchpad

A repo can commit one small file saying what to run there and what's in progress, so it travels with every clone:

```text
your-repo/.cohors/
├── repo.json          # commands to run, tasks in progress
└── agents/release.md  # a prompt this repo ships for agents
```

Ask Claude to *set up this repo for the launchpad* and it writes the file. Commands from npm, Cargo, uv, Makefiles and Gradle are found without one.

```text
$ cohors launchpad
  13 repositories, 3 set up, 5 open tasks

  Open tasks
    ▸ payments  2 open
      ◆ Idempotency keys on charge retries  high  doing
      ○ Back off webhook redelivery
    ▸ web-app  2 open
      ✕ Keyboard order in the checkout fo…  blocked
      ○ Cart drawer animation  high
```

Check a file with `cohors launchpad validate .`. The format is an open spec: [spec/v1/SPEC.md](spec/v1/SPEC.md).

## Safe by default

- `pull` is fast-forward only, and nothing force-pushes.
- No action runs without `--select`, and every one takes `--dry-run`.
- No single key can destroy work. Commit and stash ask first.
- The MCP server and the browser dashboard start read-only.
- One unreadable repo is reported, never fatal.
- No account, no telemetry. It talks only to GitHub. See the [privacy policy](PRIVACY.md).

## Staying up to date

Once a day cohors checks for a release. Installed with Homebrew, pip or the shell installer, it updates itself in the background and shows **Restart to update**; press <kbd>R</kbd>. `cohors update` does it now. Set `updates = "notify"` or `"off"` in the config to change that.

<details>
<summary>Updating by hand</summary>
<br>

| Installed with | Run |
|---|---|
| Homebrew | `brew update && brew upgrade cohors` |
| pip | `pip install -U cohors` |
| npm | `npm install -g @cohors/cli@latest` |
| Cargo | `cargo install cohors` |
| Shell installer | the install line again |
| `uvx` / `npx` with `@latest` | nothing, each run is the newest |

</details>

## How it compares

| | cohors | git-scope | mani / gita | lazygit / gitui |
|---|:---:|:---:|:---:|:---:|
| Every repo at once | ✓ | ✓ | text only | — |
| Bulk fetch, pull, push, run | ✓ | read-only | ✓ | — |
| Pull requests and CI | ✓ | — | — | one repo |
| Cross-repo search and standup | ✓ | — | — | — |
| MCP server for coding agents | ✓ | — | — | — |

<div align="center">
<br>

[Contributing](CONTRIBUTING.md) &ensp;·&ensp; [Changelog](CHANGELOG.md) &ensp;·&ensp; [Privacy](PRIVACY.md) &ensp;·&ensp; [MIT License](LICENSE)

<sub><i>cohors</i> is Latin for "cohort": every repository, marshalled under one command.</sub>

</div>
