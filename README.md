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

<p>
<a href="#install"><img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/nav-install.svg" alt="Install" height="33"></a>&nbsp;
<a href="#use-it-in-the-terminal"><img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/nav-terminal.svg" alt="Terminal" height="33"></a>&nbsp;
<a href="#use-it-with-claude-code"><img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/nav-claude-code.svg" alt="Claude Code" height="33"></a>&nbsp;
<a href="#use-it-from-scripts"><img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/nav-scripts.svg" alt="Scripts" height="33"></a>&nbsp;
<a href="#use-it-in-vs-code-and-the-browser"><img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/nav-vs-code-browser.svg" alt="VS Code &amp; browser" height="33"></a>&nbsp;
<a href="#the-launchpad"><img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/nav-launchpad.svg" alt="Launchpad" height="33"></a>
</p>

<img src="assets/demo.gif" alt="It starts with one agent: Claude Code finishes a task in payment-service and commits it. Now ten Claude Code sessions in ten repositories each say done. But who checks all ten? The cohors mark and name answer. cohors opens on all 24 repositories and outlines what they left: 9 not backed up. P pushes 8 of them, never forcing, and Not backed up drops from 9 to 1. Or ask your agent: Claude calls cohors and answers for all 24 repositories. The end card shows how to install it." width="100%">

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

The mouse works too: click a card to focus it, a row to select it and the same row again to open it, and any key shown on screen is a button. New to it? The line under the header teaches one thing a day; <kbd>x</kbd> shows the next.

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/keys-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/keys-light.svg">
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/keys.svg" alt="Dashboard keys: P p f push, pull, fetch; c S commit, stash (asks first); F fetch everything; 1 to 5 jump to a card; colon for the command line; Enter opens a repo; slash filters; s and Y show and copy the standup; t shows tasks; question mark lists every key." width="100%">
</picture>

<br>

From the <kbd>:</kbd> line: `:commit wip: cart drawer` commits the marked repos, `:!npm test` runs a command in them, `:jump payment-service` goes to one. `cohors --watch` keeps it all live.

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
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/claude.svg" alt="Claude asks cohors what needs attention; overview returns: 24 repositories, 4 have work at risk, 3 need a look, 17 are clean, then the actions, each a warning: push 1 commit from 1 repository, reconcile 1 diverged branch, publish 1 branch that has no upstream." width="100%">
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

**Agent worktrees are covered.** When a coding agent works in a git worktree (Claude Code keeps them under `.claude/worktrees/`), cohors reads it as part of its repo: commits that were never pushed and files that were never committed show up as work at risk, with the folder to go to. `cohors scan --select worktree-work` lists the repos that have any.

It starts **read-only**. Turn on what Claude may do with `claude plugin configure cohors@cohors`, then start a new session:

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/permissions-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/permissions-light.svg">
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/permissions.svg" alt="What Claude may do, each off by default: Git actions (fetch, pull, push, commit, stash; no force push, fast-forward-only pull, confirm to commit), Run commands (confirm and a per-call cap), Launchpad writes (validated, never touches git)." width="100%">
</picture>

<br>

Or let cohors brief Claude for you: `cohors agent plan-day`, or `cohors agent idempotency-keys --repo payment-service` to start on one task. `cohors agent --resume` picks a conversation up again, and `cohors next` lists the ones worth resuming.

<details>
<summary>Other MCP clients</summary>
<br>

```json
{ "command": "uvx", "args": ["cohors@latest", "mcp"] }
{ "command": "npx", "args": ["-y", "@cohors/cli@latest", "mcp"] }
{ "command": "docker", "args": ["run", "-i", "--rm", "-v", "/path/to/your/code:/repos", "ghcr.io/rushirb2001/cohors"] }
```

The Docker image serves every repository in the folder mounted at `/repos`. It's built for linux/amd64 and linux/arm64.

Or with cohors installed: `claude mcp add --scope user cohors -- "$HOME/.local/bin/cohors" mcp`, the self-updating copy cohors links on its first run (`cohors doctor` prints where it is), adding `--allow-writes`, `--allow-run` or `--allow-launchpad`. Listed in the [MCP Registry](https://registry.modelcontextprotocol.io/v0/servers?search=io.github.rushirb2001/cohors) as `io.github.rushirb2001/cohors`.

</details>

## Use it from scripts

Every view is a command. At a terminal it prints a report; piped, it prints JSON. Add `--report` to get the report through a pipe too, as plain text: a fraction of the JSON's size, and the form to hand a coding agent's shell.

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/scripts-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/scripts-light.svg">
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/scripts.svg" alt="cohors overview prints the fleet headline and Do next, each step with the command that starts it: WARN push 1 commit from 1 repository (cohors push --select ahead,!diverged); WARN reconcile 1 diverged branch (cohors scan --select diverged). Then cohors push --select ahead --dry-run would push 2 repos, nothing will change: mobile-app, payment-service." width="100%">
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

At a terminal, a command that changes repositories lists them and asks `[y/N]` before it acts. Piped, or with `--yes`, it goes straight on.

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
  <img src="https://raw.githubusercontent.com/rushirb2001/cohors/main/assets/readme/launchpad.svg" alt="cohors launchpad: 24 repositories, 4 set up, 6 open tasks. Your focus board, then open tasks per repo: api-docs (search box in the top navigation), event-ingest, payment-service (back off on processor rate limits, doing), web-storefront (keyboard order in the checkout form, blocked)." width="100%">
</picture>

<br>

Check a file with `cohors launchpad validate .`. The format is an open spec: [spec/v1/SPEC.md](spec/v1/SPEC.md).

## Safe by default

- `pull` is fast-forward only, and nothing force-pushes.
- No action runs without `--select` or repository names, and every one takes `--dry-run`.
- Nothing changes unseen. At a terminal every command that changes repositories lists them and asks first; in the dashboard no single key can destroy work, and commit and stash ask first.
- The MCP server and the browser dashboard start read-only.
- One unreadable repo is reported, never fatal.
- No account, no telemetry. It talks only to GitHub. See the [privacy policy](PRIVACY.md).

## Staying up to date

cohors keeps itself up to date however you installed it. Its first run sets up cohors' own copy in `~/.local/share/cohors` (with `~/.local/bin/cohors` linked to it) and from then on hands over to the newest version there, the way Claude Code does: it looks for a release when it starts and every few minutes, downloads it, checks its SHA-256, tries it, and switches to it. The dashboard then says **Updated to … — restart to use it**; press <kbd>R</kbd>. `cohors update` does it now and `cohors doctor` says how your copy is set up. Set `updates = "notify"` or `"off"` in the config, or `COHORS_DISABLE_UPDATES=1`, to change that. Never in CI, and never for a build from source.

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
