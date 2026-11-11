<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/readme/cohors-dark.png">
  <img src="assets/readme/cohors-light.png" alt="cohors" width="460">
</picture>

### Every git repo you own, on one screen.

What is unpushed, what is behind, what is about to be lost, and what to do next,<br>across every repository on your machine. In your terminal, your editor, your browser and your coding agent.

[![CI](https://github.com/rushirb2001/cohors/actions/workflows/ci.yml/badge.svg)](https://github.com/rushirb2001/cohors/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/tag/rushirb2001/cohors?label=release&sort=semver&color=brightgreen)](https://github.com/rushirb2001/cohors/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange)](https://www.rust-lang.org/)

[**Install**](#install) &emsp; [**Terminal**](#in-the-terminal) &emsp; [**VS Code**](#in-vs-code) &emsp; [**Browser**](#in-the-browser) &emsp; [**Your agent**](#for-your-coding-agent) &emsp; [**Launchpad**](#the-launchpad)

<img src="assets/demo.gif" alt="The cohors dashboard. Pressing P on Do next pushes every commit that exists only on this machine, and Not backed up drops from five repositories to one." width="100%">

<sub>A real run on thirteen sample repositories with real remotes. One key pushes the four that existed only on this machine;<br>the fifth has diverged from its remote, so cohors leaves that one to you.</sub>

</div>

## Try it in a minute

```sh
brew install rushirb2001/tap/cohors
cohors
```

cohors looks in the folders where code usually lives and opens a dashboard of every repository it finds. Nothing is written until you press a key that says so. To look around on sample data first, run `cohors demo`. Other ways to install are [below](#install).

## Five questions a pile of terminals cannot answer

| | |
|---|---|
| **What only exists on this laptop?** | Commits never pushed and branches never published, counted and named. |
| **What should I do first?** | A short list of steps, most urgent first: push, publish, pull, review. |
| **Where is work about to be lost?** | Every repository at risk, with the reason and the step that fixes it. |
| **What did I ship this week?** | Your commits across every repository, ready to paste into a standup. |
| **What is waiting in each repo?** | Open tasks and the commands to run, written down in the repo itself. |

One Rust binary reads your repositories once and every place below draws from that same reading, so they always agree.

### In the terminal

Five cards on one screen, each in its own colour: what to do next, work at risk, open tasks, the week's commits, and every repository. A letter is one verb everywhere: `f` fetches, `p` pulls, `P` pushes, `c` commits, `S` stashes. On "Do next" a letter carries out that step for all its repositories; on a repository it acts on that one, or on the ones you marked.

<img src="assets/terminal.png" alt="The cohors terminal dashboard with its five cards" width="100%">

### In VS Code

The cohors icon in the activity bar opens the overview as six native panes, and the dashboard tab has the same cards, with a detail panel for each repository. Everything comes from the `cohors` binary, so the editor and the terminal never disagree.

<img src="assets/vscode.png" alt="The cohors dashboard inside VS Code" width="100%">

### In the browser

`cohors web` serves the same scan on localhost. It is read-only unless you start it with write access, and your GitHub token stays on your machine: the page never sees it.

<img src="assets/web.png" alt="The cohors browser dashboard" width="100%">

### For your coding agent

`cohors mcp` gives a coding agent the same view of your fleet that you have, behind limits you set. It works as **find, target, preview, act**:

```text
search("X-Tenant-Id", kind=content)      →  the 3 repos that still use it
run("<codemod>", {ids:[…]}, dry_run)     →  "would run in 3 repos"  (nothing touched)
#  …you approve the plan, then:
run("<codemod>", {ids:[…]}, confirm)     →  per-repo { ok, exit_code, stdout, … }
```

**In Claude Code, install the plugin.** It registers the server, finds the binary wherever your install put it, and adds skills for fleet work, setting a repo up, and planning your focus:

```text
/plugin install cohors --marketplace rushirb2001/cohors
```

**Or register the server by hand.** It is read-only until you add flags:

```sh
claude mcp add cohors -- "$(which cohors)" mcp                              # read-only
claude mcp add cohors -- "$(which cohors)" mcp --allow-writes --allow-run   # + bulk actions
claude mcp add cohors -- "$(which cohors)" mcp --allow-launchpad            # + launchpad writes
```

Use the absolute path, as above. An agent starts the server with a stripped `PATH`, so a bare `cohors mcp` often resolves to nothing and the agent quietly ends up with no tools. Any other client (Claude Desktop, Cursor, Zed, …) takes the same command and arguments in its `mcpServers` block; run `cohors mcp` by hand and it prints both, filled in with your install path.

**Why an MCP server and not a bash loop?** An agent can already loop over `git`. What it cannot give itself is a boundary:

- Read-only unless you start it armed, and `pull` is fast-forward only, so it cannot lose work.
- Destructive actions need `confirm`; `dry_run` shows what would happen without touching anything, even on a read-only server.
- Locked to your configured roots, and an empty selector matches **nothing**, so a fumbled argument cannot fan out across every repository.
- A cap on how many repositories one call can touch, an optional allowlist for `run`, and an audit log of every action.

<details>
<summary><b>Every tool the server offers</b></summary>
<br>

**Reads, always on:** `overview`, `list_repos`, `get_repo`, `fleet_summary`, `search`, `changes`, `repo_path`, `list_prs`, `ci_status`, `standup`, and the launchpad reads `launchpad`, `read_contract`, `read_board`.

**Actions, behind `--allow-writes` and `--allow-run`:** `fetch`, `pull`, `push`, `commit`, `stash`, `run`.

**Launchpad writes, behind `--allow-launchpad`:** `write_repo_contribution`, `write_board`, `record_activity`.

Every read reports the roots it searched and the config in effect, so an empty fleet explains itself instead of reading as "all clear."

</details>

## The launchpad

Status tells you what state a repo is in. The launchpad tells you what you can *do* there. A repository opts in by committing one small file, so its entry travels with the clone:

```text
your-repo/.cohors/
├── repo.json          # flows to run, agents it ships, short-lived tasks
└── agents/release.md  # a prompt this repo ships for recurring work
```

You don't write it. In that repo, ask your agent to **"set up this repo for the launchpad"**: it reads the code and writes the file, and cohors rejects anything invalid with the full list of problems. Flows from npm, cargo, uv, Makefile and gradle are found with no file at all.

```sh
cohors launchpad              # your focus board and every open task across the fleet
cohors launchpad validate .   # check this repo's .cohors/repo.json
```

Then ask your agent to **"plan my day"**. It reads every repo's tasks, the uncommitted and unpushed work nobody wrote a task for, and failing CI, and proposes a focus board across all of them.

The format is an open contract, written down in [spec/v1/SPEC.md](spec/v1/SPEC.md). Text inside a repo's launchpad files is treated as untrusted data: the agent is told so, and cohors never runs a declared flow on its own.

## Install

Every channel installs the same prebuilt binary from the [latest release](https://github.com/rushirb2001/cohors/releases/latest). You need `git` on your `PATH`, and no Rust.

**Homebrew** (macOS and Linux)

```sh
brew install rushirb2001/tap/cohors
```

> Newer Homebrew versions that enforce tap trust can skip the install without saying so. Trust the formula once, then install:
> ```sh
> brew trust --formula rushirb2001/tap/cohors
> brew install rushirb2001/tap/cohors
> ```

**pip** (macOS on Apple Silicon and Intel, Linux on x86_64 and ARM64)

```sh
pip install cohors
```

**Shell installer** (checksum-verified)

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/rushirb2001/cohors/releases/latest/download/cohors-tui-installer.sh | sh
```

**From source** (needs [Rust](https://rustup.rs))

```sh
cargo install --git https://github.com/rushirb2001/cohors cohors-tui   # installs the `cohors` binary
```

**The VS Code extension** is built from this repository until it is on the Marketplace:

```sh
python3 scripts/package-extension.py
code --install-extension target/extension/cohors-*.vsix
```

With no config, cohors looks in the folders where code usually lives. Scope it to one folder with `cohors --root ~/code`, or run `cohors init` to save a config you can edit (roots, aliases, groups, ignore rules). crates.io distribution is planned.

## Commands

```sh
cohors                     # find every repo and open the dashboard
cohors --root ~/code       # only the repos under one folder
cohors demo                # the whole dashboard on sample data (nothing written to disk)
cohors init                # save an editable config to ~/.config/cohors/config.toml
cohors overview            # what to do next across every repo, and why (JSON when piped)
cohors scan                # what needs you, grouped by urgency
cohors scan --select dirty # filter: dirty, behind, unpublished, 'name:pay*', or raw JSON
cohors standup             # your commits across every repo, ready to paste
cohors fetch --select all --dry-run   # preview any bulk action before it touches anything
cohors launchpad           # focus board and open tasks across the fleet
cohors agent run setup-repo --repo .  # start your coding agent on a repo, with a shipped prompt
cohors mcp                 # the MCP server (read-only; --allow-writes / --allow-run to arm)
cohors web                 # the same fleet in the browser (--root to scope, --watch for live)
```

Run an action without a selector and it explains itself: what it does, how safe it is, and how to pick repositories. Nothing writes without a selector, and every action takes `--dry-run`.

**Keys in the dashboard**

| Key | Does | Key | Does |
|---|---|---|---|
| `1`–`5` | focus a card | `f` `p` `P` | fetch, pull, push |
| `Tab` | next card | `c` `S` | commit, stash |
| `↑` `↓` | move | `F` | fetch everything |
| `Enter` | open a repository | `s` | this week's standup |
| `/` | filter | `t` | tasks across the fleet |
| `:` | command line (`Tab` completes, `↑` recalls) | `?` | every key |
| `Space` | mark a repository | `q` | quit |

## Safe by default

- `pull` is fast-forward only, and nothing ever force-pushes.
- Every bulk action takes `--dry-run`, and nothing writes without a selector naming which repositories.
- No action is bound to a bare keystroke that can destroy work.
- The browser dashboard and the MCP server are read-only until you start them with write access.
- One repository that cannot be read is reported as such; it never takes the dashboard down.

## How it compares

| | cohors | git-scope | mani / gita | lazygit / gitui |
|---|:---:|:---:|:---:|:---:|
| Multi-repository overview | Yes | Yes | Text only | No |
| Polished terminal UI | Yes | Yes | No | Yes |
| Bulk actions (fetch / pull / run) | Yes | Read-only | Yes | No |
| Remote / PR / CI awareness | Yes | No | No | Single repo |
| Cross-repo search and weekly standup | Yes | No | No | No |
| **Agent control plane (MCP)** | **Yes** | No | No | No |
| Per-repo tasks, flows and a focus board | Yes | No | Tasks via config | No |
| Language | Rust | Go | Go / Python | Go / Rust |

<details>
<summary><b>How it is built</b></summary>
<br>

A Rust workspace around a pure, I/O-free core with thin adapters, so the *same* analysis powers the terminal, the agent and the browser:

- `cohors-core`: pure, WASM-safe models and analysis (attention scoring, the selector language and its shorthand parser, search, the launchpad contract).
- `cohors-config`, `cohors-git` ([gitoxide](https://github.com/GitoxideLabs/gitoxide)) and `cohors-github`: config, local git (bare repos included, history-memoized rescans), and GitHub (parallel, rate-limit-aware) adapters.
- `cohors-actions`: the write layer, holding the git action primitives, the one action registry every surface wires from, and the `dispatch()` the machine surfaces share.
- `cohors-launchpad`: reads each repo's `.cohors/` contribution and makes validated, symlink-safe writes of contributions, the focus board and the activity log.
- `cohors-fleet`: the composition layer, with one `Scanner` (config, git and GitHub wired once) and the read facade every front-end goes through, enforced by tests rather than convention.
- `cohors-mcp`: the MCP server (hand-rolled JSON-RPC over stdio), started by `cohors mcp`.
- `cohors-tui` ([ratatui](https://ratatui.rs)): the `cohors` binary (dashboard and CLI).
- `cohors-web` ([Leptos](https://leptos.dev), WASM): the browser front-end and the native `cohors web` server.
- `extension/`: the VS Code extension, plain JavaScript with no dependencies, which runs the binary.

Because the core is independent of its data sources and front-ends, one selector language, one analysis and one action registry serve every surface, and structural tests fail the build if any surface drifts.

</details>

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the development setup, quality gates and conventions.

## License

Released under the [MIT License](LICENSE).

<sub><i>cohors</i> is Latin for "cohort," a Roman legion's core unit: every repository, marshalled under one command.</sub>
