<div align="center">

# cohors

**Mission control for all your git repositories.**

A fast terminal dashboard that shows the live status of every git repository on your machine and lets you act on them in bulk — fetch, pull, stash, and run commands across many repos at once. One Rust core, built to grow into a web app and an agent interface.

<sub><i>cohors</i> — Latin for "cohort," a Roman legion's core unit. Every repository, marshalled into one cohort under your command.</sub>

[![CI](https://github.com/rushirb2001/cohors/actions/workflows/ci.yml/badge.svg)](https://github.com/rushirb2001/cohors/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/tag/rushirb2001/cohors?label=release&sort=semver&color=brightgreen)](https://github.com/rushirb2001/cohors/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange)](https://www.rust-lang.org/)

</div>

---

## Overview

If you work across more than a handful of repositories — microservices, a polyrepo organization, client projects, or a sprawling `~/projects` — you lose track of which ones have uncommitted work, which are behind their remote, and where you left off. Answering that today means a long sequence of `cd … && git status` and a wall of terminal tabs.

cohors replaces that with a single, fast dashboard across every repository, and the ability to act on them in bulk without leaving the terminal.

Try it in five seconds, with no setup and nothing written to disk:

```sh
cargo run -p cohors-tui -- demo
```

This launches the full interface on a built-in, privacy-safe sample fleet — every column and view populated, so you can see exactly what cohors does before pointing it at your own repositories.

## Features

- **Every repository on one screen.** Auto-discovers git repositories under your configured roots and shows branch, ahead/behind, dirty state, stashes, and last activity — sorted dirty-first, so what needs attention rises to the top.
- **Fast.** Parallel scanning with a warm cache; the dashboard launches in milliseconds across dozens of repositories.
- **Fuzzy navigation.** Jump to any repository by name or path, or filter instantly to just the ones with changes.
- **Bulk actions.** Fetch, pull (fast-forward only), stash (with confirmation), and run any command across the selected repositories with live, per-repo output.
- **Remote-aware.** Open pull-request counts, CI status, and ahead/behind against upstream, shown inline.
- **Weekly standup.** Every commit you made this week, across every repository, gathered into one view.

A coding-agent (MCP) server and a WebAssembly web dashboard are on the roadmap, both built on the same core.

## How it compares

| | cohors | git-scope | mani / gita | lazygit / gitui |
|---|:---:|:---:|:---:|:---:|
| Multi-repository overview | Yes | Yes | Text only | No |
| Polished terminal UI | Yes | Yes | No | Yes |
| Bulk actions (fetch / pull / run) | Yes | Read-only | Yes | No |
| Remote / PR / CI awareness | Yes | No | No | Single repo |
| Cross-repo weekly standup | Yes | No | No | No |
| Language | Rust | Go | Go / Python | Go / Rust |

## Architecture

cohors is a Rust workspace built around a pure, I/O-free core that holds all the domain logic, with thin adapters around it:

- `cohors-core` — pure analysis and models, with no I/O (kept WebAssembly-safe).
- `cohors-config` — configuration and repository discovery.
- `cohors-git` — the local git provider, built on [gitoxide](https://github.com/GitoxideLabs/gitoxide).
- `cohors-github` — the GitHub provider for remote, PR, and CI data.
- `cohors-tui` — the terminal front-end, built on [ratatui](https://ratatui.rs); ships the `cohors` binary.

Because the core is independent of both its data sources and its front-ends, the same analysis will power the terminal, an agent interface, and the browser.

## Installation

cohors requires [Rust](https://rustup.rs) (the toolchain version is pinned in `rust-toolchain.toml`) and `git` on your `PATH`.

Install directly from the repository:

```sh
cargo install --git https://github.com/rushirb2001/cohors cohors-tui
```

Or clone and install from source:

```sh
git clone https://github.com/rushirb2001/cohors && cd cohors
cargo install --path crates/cohors-tui
```

Both install the `cohors` binary. Distribution through crates.io, Homebrew, and prebuilt release binaries is planned.

## Usage

```sh
cohors demo      # full UI on built-in sample data — no config, nothing written to disk
cohors init      # write ~/.config/cohors/config.toml, then set: roots = ["~/projects", "~/work"]
cohors           # scan your repositories and launch the dashboard
cohors scan      # print repository snapshots as JSON (scriptable)
```

Inside the dashboard, the essentials are:

- `↑` / `↓` move, `Enter` inspect a repository, `/` fuzzy filter, `d` dirty-only, `s` cycle sort.
- `Space` mark, `a` mark all, `Esc` clear selection.
- `f` / `F` fetch selection / all, `p` pull (fast-forward only), `S` stash, `!` run a command across the selection.
- `Tab` weekly standup, `o` open in your editor, `y` copy path, `r` refresh, `q` quit.

Bulk actions target the marked repositories, or the current one when nothing is marked. Press `?` for the full keymap.

## Contributing

Contributions are welcome. Please see [CONTRIBUTING.md](CONTRIBUTING.md) for the development setup, quality gates, and conventions.

## License

Released under the [MIT License](LICENSE).
