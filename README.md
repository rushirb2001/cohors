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

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/answers-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/answers-light.svg">
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/answers.svg" alt="What cohors answers: what to do first (Do next), what is about to be lost (Work at risk), what is waiting in each repo (Open tasks), what you shipped this week (Commits), and what exists only on this laptop (Repositories)." width="100%">
</picture>

<br>

**Where it looks.** With no config, the folders where code usually lives. Point it at yours with `cohors --root ~/code` (repeat for more), or save them with `cohors init`, which writes `~/.config/cohors/config.toml` for roots, aliases, groups and ignore rules.

## Use it in the terminal

```sh
cohors
```

<img src="assets/terminal.png" alt="The cohors terminal dashboard" width="100%">

**Do next** lists what to do, most urgent first. Press a step's letter to act on every repo in it, or mark repos with <kbd>Space</kbd> and press the letter yourself.

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/keys-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/keys-light.svg">
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/keys.svg" alt="Dashboard keys: P p f push, pull, fetch; c S commit, stash (asks first); F fetch everything; 1 to 5 jump to a card; colon for the command line; Enter opens a repo; slash filters; s and Y show and copy the standup; t shows tasks; question mark lists every key." width="100%">
</picture>

<br>

From the <kbd>:</kbd> line: `:commit wip: cart drawer` commits the marked repos, `:!npm test` runs a command in them, `:jump payments` goes to one. `cohors --watch` keeps it all live.

<a name="use-it-with-claude-code"></a>

## <img src="https://cdn.simpleicons.org/claude/D97757" alt="" height="26" align="top"> Use it with Claude Code

```text
/plugin marketplace add rushirb2001/cohors
```
```text
/plugin install cohors@cohors
```

Ask *"what needs my attention across my repos?"* and Claude gets the whole fleet in one call:

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/claude-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/claude-light.svg">
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/claude.svg" alt="Claude asks cohors what needs attention; overview returns: 13 repositories, 6 have work at risk, 2 are clean, then the actions: push 4 commits from 3 repositories (risk), reconcile 1 diverged branch, publish 1 branch with no upstream (warn)." width="100%">
</picture>

<br>

More to ask:

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/ask-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/ask-light.svg">
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/ask.svg" alt="More to ask Claude: what have I changed (changes), where do we handle retries (search), write my standup (standup), back up everything only on this laptop (push, as a dry run first), which PRs are failing CI (ci_status and list_prs), plan my day (plan-focus)." width="100%">
</picture>

<br>

It starts **read-only**. Turn on what Claude may do with `claude plugin configure cohors@cohors`, then start a new session:

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/permissions-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/permissions-light.svg">
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/permissions.svg" alt="What Claude may do, each off by default: Git actions (fetch, pull, push, commit, stash; no force push, fast-forward-only pull, confirm to commit), Run commands (confirm and a per-call cap), Launchpad writes (validated, never touches git)." width="100%">
</picture>

<br>

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

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/scripts-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/scripts-light.svg">
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/scripts.svg" alt="cohors overview prints the fleet headline and Do next: RISK push 4 commits from 3 repositories, with the command cohors push --select ahead,!diverged; WARN reconcile 1 diverged branch. Then cohors push --select ahead --dry-run would push 4 repos, nothing will change: mobile-app, payments, design-system, analytics." width="100%">
</picture>

<br>

`--select` picks the repos. Join terms with a comma (all must match) and put `!` in front to exclude:

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/select-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/select-light.svg">
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/select.svg" alt="--select terms: dirty, ahead, behind, diverged; unpublished, stale-fetch, attention; name:pay* and ci:failing; all. A comma means all must match, ! excludes, as in behind,!diverged." width="100%">
</picture>

<br>

```sh
cohors fetch --select all                      # refresh every remote
cohors run --select 'name:web-*' 'npm test'    # one command, many repos
cohors standup                                 # this week's commits, as Markdown
cohors overview --json | jq -r '.actions[].title'
```

## Use it in VS Code and the browser

<br>

<table>
<tr>
<td width="50%"><img src="assets/vscode.png" alt="cohors in VS Code"></td>
<td width="50%"><img src="assets/web.png" alt="cohors in the browser"></td>
</tr>
<tr>
<td valign="top">

The cohors icon in the activity bar shows the fleet as native panes. Push, pull and fetch from any step, run a repo's commands in a terminal, or start Claude on a task.

[![VS Code Marketplace](https://img.shields.io/visual-studio-marketplace/v/rushirb2001.cohors?label=VS%20Code&color=007ACC)](https://marketplace.visualstudio.com/items?itemName=rushirb2001.cohors)
[![Open VSX](https://img.shields.io/open-vsx/v/rushirb2001/cohors?label=Open%20VSX&color=A60EE5)](https://open-vsx.org/extension/rushirb2001/cohors)

Search **cohors** in the Extensions view. Cursor, Windsurf and VSCodium install it from Open VSX.

</td>
<td valign="top">

`cohors web --root ~/code --watch` serves the dashboard on localhost, with pull requests and CI. Read-only unless you add `--allow-writes`. `cohors web --demo` shows the sample fleet.

</td>
</tr>
</table>

<br>

## The launchpad

A repo can commit one small file saying what to run there and what's in progress, so it travels with every clone:

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/tree-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/tree-light.svg">
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/tree.svg" alt="A repo .cohors folder: repo.json holds commands to run and tasks in progress; agents/release.md is a prompt the repo ships for agents. It travels with every clone." width="100%">
</picture>

<br>

Ask Claude to *set up this repo for the launchpad* and it writes the file. Commands from npm, Cargo, uv, Makefiles and Gradle are found without one.

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/launchpad-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/launchpad-light.svg">
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/launchpad.svg" alt="cohors launchpad: 13 repositories, 3 set up, 5 open tasks. Your focus board, then open tasks per repo: data-pipeline, payments (idempotency keys on charge retries, doing), web-app (keyboard order in the checkout form, blocked)." width="100%">
</picture>

<br>

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

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/update-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/update-light.svg">
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/update.svg" alt="Updating by hand: brew update && brew upgrade cohors; pip install -U cohors; npm install -g @cohors/cli@latest; cargo install cohors; rerun the shell installer; uvx and npx with @latest need nothing." width="100%">
</picture>

<br>

</details>

## How it compares

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/compare-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/compare-light.svg">
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/compare.svg" alt="How cohors compares with git-scope, mani or gita, and lazygit or gitui: only cohors covers every repo at once, bulk fetch, pull, push and run, pull requests and CI, cross-repo search and standup, and an MCP server for coding agents." width="100%">
</picture>

<br>

<div align="center">
<br>

[Contributing](CONTRIBUTING.md) &ensp;·&ensp; [Changelog](CHANGELOG.md) &ensp;·&ensp; [Privacy](PRIVACY.md) &ensp;·&ensp; [MIT License](LICENSE)

<sub><i>cohors</i> is Latin for "cohort": every repository, marshalled under one command.</sub>

</div>
