<div align="center">

# 🛰️ myriarch

**Mission control for all your git repos.**

A fast, beautiful terminal dashboard — and web app — that shows the live status of *every* git repository on your machine and lets you act on them in bulk. One Rust core, two front-ends.

<sub><i>myriarch</i> · Greek <b>μυριάρχης</b>, "commander of ten thousand" — the officer who marshalled a <i>myriad</i>. Now it commands your myriad of repos.</sub>

[![Status](https://img.shields.io/badge/status-pre--alpha%20(building%20in%20public)-orange)](docs/ROADMAP.md)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange)](https://www.rust-lang.org/)

</div>

> 🚧 **Status: pre-0.1, in active development.** This README describes the product we're building. Code lands milestone by milestone — see the [Roadmap](docs/ROADMAP.md).

---

## The problem

If you have more than a handful of repos — microservices, a polyrepo org, client work, side projects — you live in *multi-repo blindness*: which repos have uncommitted work? Which are behind their remote? Where did I leave that branch? Today you answer this with a graveyard of `cd ... && git status` and a dozen terminal tabs.

Existing tools each solve one slice:

- **lazygit / gitui** — gorgeous, but one repo at a time.
- **mani / gita / meta** — run commands across repos, but no visual dashboard and no insight.
- **git-scope** — a nice multi-repo status view, but *read-only*, *local-only*, with no remote/PR awareness.

**myriarch is the one that does all of it:** a single pane of glass across every repo, with the polish of lazygit, the breadth of mani, *and* the ability to act — plus remote/PR/CI health and an online dashboard you can share with your team.

## What myriarch does

- 🛰️ **One screen, every repo.** Auto-discovers git repos under your project roots and shows branch, ahead/behind, dirty state, stashes, and last activity — sorted *dirty-first* so what needs you bubbles to the top.
- ⚡ **Instant.** Parallel scanning + a warm cache. Launches in milliseconds even with 50+ repos.
- 🔍 **Fuzzy everything.** Jump to any repo by name or path; filter to just the dirty ones.
- 🎬 **Act in bulk.** Fetch/pull across selected repos, open any repo in your editor or lazygit, copy paths — without leaving the dashboard.
- 🌐 **Remote-aware** *(v0.2)*. Open-PR counts, CI status, and ahead/behind vs upstream, right in the table.
- 🗓️ **"What did I ship?"** *(v0.2)*. A cross-repo weekly standup: every commit you made this week, across every repo, in one view.
- 🖥️ **Online version** *(v0.4)*. The same core, compiled to WebAssembly: connect GitHub, see your whole fleet's health in the browser, and share a read-only team dashboard.

## Why it's built the way it is

myriarch is a Rust **workspace** with a pure, I/O-free **core** (`myriarch-core`) that holds all the domain logic — and thin **adapters** around it:

- a **local git** data source (`myriarch-git`, via [gitoxide](https://github.com/GitoxideLabs/gitoxide)),
- a **GitHub** data source (`myriarch-github`),
- a **TUI** front-end (`myriarch-tui`, via [ratatui](https://ratatui.rs)),
- and a **WASM web** front-end (`myriarch-web`, via [Leptos](https://leptos.dev)).

Because the core is data-source- and front-end-agnostic, the *exact same* analysis powers the terminal and the browser. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## How myriarch compares

| | myriarch | git-scope | mani / gita | lazygit / gitui |
|---|:---:|:---:|:---:|:---:|
| Multi-repo overview | ✅ | ✅ | ⚠️ text only | ❌ |
| Beautiful TUI | ✅ | ✅ | ❌ | ✅ |
| Bulk actions (fetch/pull/run) | ✅ | ❌ read-only | ✅ | ❌ |
| Remote / PR / CI awareness | ✅ *(v0.2)* | ❌ | ❌ | ⚠️ single repo |
| Cross-repo "weekly standup" | ✅ *(v0.2)* | ❌ | ❌ | ❌ |
| Online / shareable dashboard | ✅ *(v0.4)* | ❌ | ❌ | ❌ |
| Language | Rust | Go | Go / Python | Go / Rust |

## Install

> Coming with v0.1. Planned channels: `cargo install myriarch`, `cargo binstall myriarch`, Homebrew tap, Nix flake, and prebuilt binaries on every GitHub Release. See [docs/DISTRIBUTION.md](docs/DISTRIBUTION.md).

```sh
# (planned)
cargo install myriarch
myriarch            # launch the dashboard
myriarch init       # write a starter config
```

## Quickstart (planned)

```sh
myriarch init                      # creates ~/.config/myriarch/config.toml
# edit roots = ["~/projects", "~/work"]
myriarch                           # scan + launch the TUI
```

Keys: `j/k` move · `/` fuzzy filter · `d` dirty-only · `s` cycle sort · `Enter` open in editor · `F` fetch all · `?` help · `q` quit. Full keymap in [docs/TUI-DESIGN.md](docs/TUI-DESIGN.md).

## Documentation

| Doc | What's in it |
|---|---|
| [VISION](docs/VISION.md) | Who it's for, the thesis, success metrics |
| [COMPETITIVE-ANALYSIS](docs/COMPETITIVE-ANALYSIS.md) | Every competitor, feature matrix, our wedge |
| [ARCHITECTURE](docs/ARCHITECTURE.md) | Crate layout, the core+adapters design, dependencies |
| [ROADMAP](docs/ROADMAP.md) | v0.1 → v0.4 milestones with acceptance criteria |
| [MVP-SPEC](docs/MVP-SPEC.md) | The detailed spec for v0.1 (build this first) |
| [TUI-DESIGN](docs/TUI-DESIGN.md) | Wireframes, keymap, states, theming |
| [DISTRIBUTION](docs/DISTRIBUTION.md) | How we ship and how myriarch gets discovered |
| [DECISIONS](docs/DECISIONS.md) | Architecture decision records (ADRs) |

## Contributing

myriarch is open source (MIT) and built in public. Issues, ideas, and PRs welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE) © myriarch contributors
