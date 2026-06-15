<div align="center">

# 🛡️ cohors

**Mission control for all your git repos.**

A fast, beautiful terminal dashboard — and web app — that shows the live status of *every* git repository on your machine and lets you act on them in bulk. One Rust core, two front-ends.

<sub><i>cohors</i> · Latin for "cohort" — a Roman legion's core battle unit of ~480. Every repo, marshalled into one cohort under your command.</sub>

[![CI](https://github.com/rushirbhavsar/cohors/actions/workflows/ci.yml/badge.svg)](https://github.com/rushirbhavsar/cohors/actions/workflows/ci.yml)
[![Version](https://img.shields.io/badge/version-v0.1%20·%20local%20dashboard-brightgreen)](docs/ROADMAP.md)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange)](https://www.rust-lang.org/)

![cohors dashboard demo](docs/demo.gif)

<sub>Rendered at release time from generated <strong>sample</strong> repos — never real data (<a href="docs/DECISIONS.md">ADR-015</a>): <code>vhs docs/demo.tape</code>.</sub>

</div>

> ✅ **v0.3 is here.** The dashboard discovers every repo, shows local + GitHub PR/CI status, does a cross-repo weekly standup (v0.2), and now lets you **multi-select repos and act in bulk** — fetch/pull, stash (with confirmation), and run any command across them with live per-repo output (v0.3). The **MCP server** (v0.4) — the same powers for your coding agent — is next, then the web app (v0.5). See the [Roadmap](docs/ROADMAP.md).

---

## The problem

If you have more than a handful of repos — microservices, a polyrepo org, client work, side projects — you live in *multi-repo blindness*: which repos have uncommitted work? Which are behind their remote? Where did I leave that branch? Today you answer this with a graveyard of `cd ... && git status` and a dozen terminal tabs.

Existing tools each solve one slice:

- **lazygit / gitui** — gorgeous, but one repo at a time.
- **mani / gita / meta** — run commands across repos, but no visual dashboard and no insight.
- **git-scope** — a nice multi-repo status view, but *read-only*, *local-only*, with no remote/PR awareness.

**cohors is the one that does all of it:** a single pane of glass across every repo, with the polish of lazygit, the breadth of mani, *and* the ability to act — plus remote/PR/CI health and an online dashboard you can share with your team.

## What cohors does

- 🛰️ **One screen, every repo.** Auto-discovers git repos under your project roots and shows branch, ahead/behind, dirty state, stashes, and last activity — sorted *dirty-first* so what needs you bubbles to the top.
- ⚡ **Instant.** Parallel scanning + a warm cache. Launches in milliseconds even with 50+ repos.
- 🔍 **Fuzzy everything.** Jump to any repo by name or path; filter to just the dirty ones.
- 🎬 **Act in bulk.** Fetch/pull across selected repos, open any repo in your editor or lazygit, copy paths — without leaving the dashboard.
- 🌐 **Remote-aware** *(v0.2)*. Open-PR counts, CI status, and ahead/behind vs upstream, right in the table.
- 🗓️ **"What did I ship?"** *(v0.2)*. A cross-repo weekly standup: every commit you made this week, across every repo, in one view.
- 🤖 **Agent-native (MCP)** *(v0.4)*. Run a `cohors` MCP server so Claude Code (and other agents) get the same fleet sense + bulk actions you have — "find every repo calling `X` and open a PR in each." See [docs/MCP-DESIGN.md](docs/MCP-DESIGN.md).
- 🖥️ **Online version** *(v0.5)*. The same core, compiled to WebAssembly: connect GitHub, see your whole fleet's health in the browser, and share a read-only team dashboard.

## Why it's built the way it is

cohors is a Rust **workspace** with a pure, I/O-free **core** (`cohors-core`) that holds all the domain logic — and thin **adapters** around it:

- a **local git** data source (`cohors-git`, via [gitoxide](https://github.com/GitoxideLabs/gitoxide)),
- a **GitHub** data source (`cohors-github`),
- a **TUI** front-end (`cohors-tui`, via [ratatui](https://ratatui.rs)),
- and a **WASM web** front-end (`cohors-web`, via [Leptos](https://leptos.dev)).

Because the core is data-source- and front-end-agnostic, the *exact same* analysis powers the terminal and the browser. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## How cohors compares

| | cohors | git-scope | mani / gita | lazygit / gitui |
|---|:---:|:---:|:---:|:---:|
| Multi-repo overview | ✅ | ✅ | ⚠️ text only | ❌ |
| Beautiful TUI | ✅ | ✅ | ❌ | ✅ |
| Bulk actions (fetch/pull/run) | ✅ | ❌ read-only | ✅ | ❌ |
| Remote / PR / CI awareness | ✅ *(v0.2)* | ❌ | ❌ | ⚠️ single repo |
| Cross-repo "weekly standup" | ✅ *(v0.2)* | ❌ | ❌ | ❌ |
| Agent control (MCP) | ✅ *(v0.4)* | ❌ | ❌ | ❌ |
| Online / shareable dashboard | ✅ *(v0.5)* | ❌ | ❌ | ❌ |
| Language | Rust | Go | Go / Python | Go / Rust |

## Install

**v0.1 — from source.** Needs [Rust](https://rustup.rs) (the version is pinned in `rust-toolchain.toml`) and `git` on your `PATH`:

```sh
git clone https://github.com/rushirbhavsar/cohors && cd cohors
cargo install --path crates/cohors-tui   # installs the `cohors` binary
```

Or straight from git, without cloning:

```sh
cargo install --git https://github.com/rushirbhavsar/cohors cohors-tui
```

> Crates.io (`cargo install cohors`), `cargo binstall`, a Homebrew tap, and prebuilt binaries on every GitHub Release are planned — see [docs/DISTRIBUTION.md](docs/DISTRIBUTION.md).

## Try it in 5 seconds

No config, no scanning, nothing touched on disk — just the full dashboard on a
built-in sample fleet:

```sh
cohors demo
```

Every column and view is populated with privacy-safe demo data (ahead/behind,
dirty, stashed, CI/PRs, a detached HEAD, an unreadable repo). Press `?` for the
legend, `Tab` for the standup, `!` to see the command runner. It's the fastest
way to see what cohors is before you point it at your own repos.

## Quickstart

```sh
cohors init                      # writes ~/.config/cohors/config.toml
# edit it: roots = ["~/projects", "~/work"]
cohors                           # scan + launch the dashboard
cohors scan                      # or: print snapshots as JSON (scriptable)
```

Keys: `↑`/`↓` move · `Space` mark · `a` mark all · `Esc` clear · `/` fuzzy filter · `d` dirty-only · `s` cycle sort · `Tab` weekly standup · `Enter` inspect repo (detail pane: commits, changes, branches, stashes) · `o` open with… (auto-detected editors, reveal, lazygit — `d` sets a default) · `f`/`F` fetch selection/all · `p` pull (ff-only) · `!` run a command across the selection · `S` stash (confirms) · `L` lazygit · `y` copy path · `h` hide hints · `r` refresh · `?` help · `q` quit. Bulk actions target the marked repos, or the current one when nothing is marked. Full keymap in [docs/TUI-DESIGN.md](docs/TUI-DESIGN.md).

## Documentation

| Doc | What's in it |
|---|---|
| [VISION](docs/VISION.md) | Who it's for, the thesis, success metrics |
| [COMPETITIVE-ANALYSIS](docs/COMPETITIVE-ANALYSIS.md) | Every competitor, feature matrix, our wedge |
| [ARCHITECTURE](docs/ARCHITECTURE.md) | Crate layout, the core+adapters design, dependencies |
| [ROADMAP](docs/ROADMAP.md) | v0.1 → v0.4 milestones with acceptance criteria |
| [MVP-SPEC](docs/MVP-SPEC.md) | The detailed spec for v0.1 (build this first) |
| [TUI-DESIGN](docs/TUI-DESIGN.md) | Wireframes, keymap, states, theming |
| [DISTRIBUTION](docs/DISTRIBUTION.md) | How we ship and how cohors gets discovered |
| [USE-CASES](docs/USE-CASES.md) | Personas + jobs-to-be-done across TUI / MCP / web |
| [MCP-DESIGN](docs/MCP-DESIGN.md) | The `cohors mcp` agent surface: tools, selectors, safety |
| [DECISIONS](docs/DECISIONS.md) | Architecture decision records (ADRs) |

## Contributing

cohors is open source (MIT) and built in public. Issues, ideas, and PRs welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE) © cohors contributors
