<div align="center">

# cohors

**A governed control plane for all your git repositories — for you, and your coding agent.**

Coding agents made single-repo work cheap. The bottleneck moved to the *fleet*: an agent is blind across your 20 repos — it can't see which need attention, can't find which ones call `X`, and can't safely act across them. cohors is the fix — one fast view of every repo, a search that indexes the whole fleet, and bulk actions an agent can preview before you ever grant write access. Over MCP, in the terminal, and (soon) the browser. One Rust core.

<sub><i>cohors</i> — Latin for "cohort," a Roman legion's core unit. Every repository, marshalled into one cohort under your command.</sub>

[![CI](https://github.com/rushirb2001/cohors/actions/workflows/ci.yml/badge.svg)](https://github.com/rushirb2001/cohors/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/tag/rushirb2001/cohors?label=release&sort=semver&color=brightgreen)](https://github.com/rushirb2001/cohors/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange)](https://www.rust-lang.org/)

</div>

---

## The fleet is the new bottleneck

You — and your agent — now work across dozens of repos, and the hard questions are fleet-wide: *Which repos have uncommitted work? Which still import the old client? Run the tests in everything I touched today. Rename this header everywhere and open a PR in each.*

By hand that's `cd … && git status` across a dozen tabs and a `for d in */` loop with no guard rail. And handing that loop to an agent means trusting it with **unbounded mutation across your whole machine**. cohors makes the fleet a first-class thing you can **enumerate, search, and act on — safely.**

## For your agent: a governed MCP control plane

`cohors mcp` gives a coding agent the same fleet sense you have, behind a safety model you control. The workflow it unlocks — **find → target → preview → act:**

```text
search("X-Tenant-Id", kind=content)      →  the 3 repos that still use it
run("<codemod>", {ids:[…]}, dry_run)     →  "would run in 3 repos"  (nothing touched)
#  …you approve the plan, then:
run("<codemod>", {ids:[…]}, confirm)     →  per-repo { ok, exit_code, stdout, … }
```

Register it — **read-only by default**, opt into actions with flags:

```sh
claude mcp add cohors -- cohors mcp                              # read-only
claude mcp add cohors -- cohors mcp --allow-writes --allow-run   # + bulk actions
```

**Why an MCP and not a bash loop?** The value isn't capability — an agent *can* loop over `git`. It's the **governed boundary**, which an agent cannot give itself:

- Read-only unless you launch it armed; `pull` is fast-forward-only (it can't lose work).
- `confirm` on destructive actions; `dry_run` previews the blast radius with zero side effects (even on a read-only server).
- Scope-locked to your configured roots; an empty selector matches **nothing**, so a fumbled argument can't fan out across everything.
- A target cap, an optional `run` command allowlist, and an audit log of every action.

**Tools.** Reads (always on): `list_repos`, `get_repo`, `fleet_summary`, `search`, `repo_path`, `list_prs`, `ci_status`. Actions (flag-gated): `fetch`, `pull`, `stash`, `run`. Every read carries diagnostics (the roots searched, the config in effect), so an empty fleet explains itself instead of reading as "all clear."

## For you: a fast fleet dashboard

The same core powers a terminal dashboard. See it in five seconds — no setup, nothing written to disk:

```sh
cohors demo
```

- **Every repo on one screen** — branch, ahead/behind, dirty state, stashes, last activity, sorted dirty-first.
- **Fast** — parallel scanning with a warm cache; launches in milliseconds across dozens of repos.
- **Bulk actions** — fetch, pull (ff-only), stash (confirmed), and run a command across the selected repos with live, per-repo output.
- **Remote-aware** — open pull-request counts, CI status, and ahead/behind vs upstream, inline.
- **Weekly standup** — every commit you made this week, across every repo, in one view.

## Install

Requires [Rust](https://rustup.rs) and `git` on your `PATH`.

```sh
cargo install --git https://github.com/rushirb2001/cohors cohors-tui   # the `cohors` binary
cohors init     # auto-detects where your repos live and writes a config
cohors          # launch the dashboard
```

Or clone and `cargo install --path crates/cohors-tui`. Distribution via crates.io, Homebrew, and prebuilt release binaries is planned.

## Usage

```sh
cohors                 # scan your repos and launch the dashboard
cohors demo            # full UI on built-in sample data (nothing written to disk)
cohors init            # detect your repos → ~/.config/cohors/config.toml
cohors scan            # print repository snapshots as JSON (scriptable)
cohors scan --select dirty           # filter — e.g. dirty, behind, 'name:pay*', or raw JSON
cohors mcp             # run the MCP server (read-only; --allow-writes / --allow-run to arm)
```

Inside the dashboard: `↑`/`↓` move · `Enter` inspect · `/` fuzzy filter · `Space` mark · `f`/`p`/`S` fetch/pull/stash · `!` run a command · `Tab` weekly standup · `?` help · `q` quit. Bulk actions target the marked repos, or the current one when nothing is marked.

## How it compares

| | cohors | git-scope | mani / gita | lazygit / gitui |
|---|:---:|:---:|:---:|:---:|
| Multi-repository overview | Yes | Yes | Text only | No |
| Polished terminal UI | Yes | Yes | No | Yes |
| Bulk actions (fetch / pull / run) | Yes | Read-only | Yes | No |
| Remote / PR / CI awareness | Yes | No | No | Single repo |
| Cross-repo search + weekly standup | Yes | No | No | No |
| **Agent control plane (MCP)** | **Yes** | No | No | No |
| Language | Rust | Go | Go / Python | Go / Rust |

## How it's built

A Rust workspace around a pure, I/O-free core with thin adapters — so the *same* analysis powers the terminal, the agent, and (soon) the browser:

- `cohors-core` — pure, WASM-safe models and analysis: attention scoring, the selector language, search.
- `cohors-config` · `cohors-git` ([gitoxide](https://github.com/GitoxideLabs/gitoxide)) · `cohors-github` — config, local git, and GitHub adapters.
- `cohors-tui` ([ratatui](https://ratatui.rs)) — the `cohors` binary: dashboard, CLI, and MCP server.

Because the core is independent of its data sources and front-ends, one selector language and one analysis serve every surface.

## Contributing

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) for the development setup, quality gates, and conventions.

## License

Released under the [MIT License](LICENSE).
