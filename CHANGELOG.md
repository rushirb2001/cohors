# Changelog

All notable changes to **cohors** are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Versioning policy

cohors is pre-1.0, so it follows the `0.MINOR.PATCH` convention:

- **MINOR** (`0.x.0`) — new features _and_ any breaking change (the public surface
  is still unstable before 1.0).
- **PATCH** (`0.x.y`) — backwards-compatible bug fixes and polish only.

Every release gets an entry below and an annotated git tag (`vX.Y.Z`) on the
release commit. The version in `Cargo.toml` (`[workspace.package]`) is the single
source of truth and is bumped in a dedicated `chore(release)` commit.

## [Unreleased]

_Nothing yet._

## [0.3.5] — 2026-06-14

### Changed

- The **standup** view now uses the command-runner's two-pane layout: a repo
  list with per-repo commit counts on the left, and the focused repo's commits
  scrollable on the right (`↑/↓` switch repo, `PgUp/PgDn` scroll). `y` still
  copies the full Markdown digest. Repo grouping/ordering is shared with the
  digest via a new `cohors_core::group_commits`.

## [0.3.4] — 2026-06-14

### Added

- The background now **dims behind a modal overlay** (help, standup, command
  runner, confirm), so the open view stands out and the rest recedes — a
  terminal-friendly stand-in for a blur.

## [0.3.3] — 2026-06-14

### Changed

- The footer now groups the key hints into labelled rows — **select** / **act** /
  **view** — with the key in an accent colour and a plain-word description, so
  it reads like a legend (it's clear that, e.g., the "act" keys act on the marked
  repos). Each row wraps independently on a narrow terminal.

## [0.3.2] — 2026-06-14

### Changed

- The key-hint **footer** is now a box whose commands **wrap onto more lines on a
  narrow ("compact") terminal**, instead of being truncated on the right.
- **Reverted** the command-run view to the two-pane list + scrollable detail (the
  per-repo boxed column from 0.3.1 wasn't wanted). The mouse-scroll reversal from
  0.3.1 stays.

## [0.3.1] — 2026-06-13

### Changed

- The command-run view now shows **one boxed section per repo** (a `╭─ name · ✓`
  header rule + its output) in a single scrollable column, and the output
  **wraps** so it stays readable in a narrow/compact terminal (was a fixed
  two-pane list + detail that clipped long lines).

### Fixed

- **Reversed the scroll direction.** cohors now captures the mouse and handles
  wheel/trackpad scroll itself (the terminal was translating it to arrow keys),
  so a scroll-up gesture moves the list/content up instead of down.

## [0.3.0] — 2026-06-13

Bulk actions across the fleet — select repos and act on all of them.

### Added

- **Multi-select**: `Space` marks/unmarks a repo (`a` marks all, `Esc` clears),
  with a `· N selected` count in the header and a `●` gutter on marked rows.
  Marks survive sort/filter/refresh. Actions target the marked set, or the
  current repo when nothing is marked.
- **Command runner** (`!`): run an arbitrary shell command across the selected
  repos concurrently (bounded pool), with a live per-repo status list
  (`✓`/`✗ exit N`), a scrollable per-repo output pane, a combined
  `N ✓ · M ✗` summary, and copy-to-clipboard (`y`).
- **Bulk stash** (`S`) behind a confirmation modal (default No), and **fetch**
  (`f`) / **pull** (`p`) now act on the whole selection.

### Notes

- Bulk *checkout* is served by the command runner (`! git checkout <branch>`)
  rather than a dedicated built-in. Config `groups`/tags are deferred (optional).

## [0.2.2] — 2026-06-13

### Fixed

- The **Remote** indicator was invisible in terminals whose font lacks a text
  glyph for the cloud character (`☁`, U+2601). Replaced it with a colored status
  dot (`●`) — a basic glyph present in every monospace font, colored via ANSI
  like the rest of the UI (green passing, red failing, yellow pending). Follows
  the same monochrome-glyph-plus-color approach Claude Code's TUI uses; emoji are
  avoided because they're double-width and can't be themed or `NO_COLOR`'d.

## [0.2.1] — 2026-06-13

### Changed

- The **Remote** column now shows a single cloud (`☁`) colored by CI health —
  green passing, red failing, yellow pending, dim when there's no signal — with
  the open-PR count beside it, instead of distinct `✓`/`✗`/`●`/`·` glyphs. One
  recognizable icon reads more simply than four.

### Fixed

- Remote (PR/CI) state now **persists**: it's carried across a re-scan instead of
  blanking to "—" until re-enrichment, and it's written to the warm-start cache
  so a relaunch shows it immediately. Previously the column flickered on every
  refresh and never survived a restart.

## [0.2.0] — 2026-06-13

Remote-aware fleet, a weekly standup, and a full dashboard redesign.

### Added

- **GitHub enrichment** (`cohors-github`): per-repo open-PR count, CI/check
  status, and default branch via the REST API. Token is discovered from
  `gh auth token` or `$GITHUB_TOKEN`; results are cached (5-minute TTL) and
  rate-limit-aware. The local scan paints first and enrichment fills in on a
  background thread, so the network never blocks the dashboard.
- **Remote** column showing CI state and open-PR count (or `—` off-GitHub).
- **Weekly standup** view (`Tab`): a scrollable digest of every commit you
  authored across all repos in a window (today / this week), grouped by repo and
  ordered most-active-first, with per-repo commit counts. Copy it to the
  clipboard as Markdown with `y`; scroll with `↑/↓` · `PgUp/PgDn` · `g/G`.

### Changed

- **Redesigned TUI** for readability: a branded header box (name, version,
  description), titled **Attention** and **Repositories** panels with
  plain-word labels instead of terse glyphs, rounded borders throughout, and
  tightened, header-labelled columns.

## [0.1.0] — 2026-06-13

Initial release: a local git dashboard that beats `git-scope`.

### Added

- Fast ratatui TUI listing every local git repo, with parallel discovery
  (`gix`, `git2` fallback) behind a pure, WASM-safe core + adapter design.
- Per-repo status: branch, ahead/behind upstream, dirty counts
  (staged/modified/untracked), stash count, and last commit.
- Sort (dirty-first), fuzzy filter, dirty-only toggle, and a help overlay;
  arrow-key navigation with `Home`/`End`.
- Actions on a synchronous loop: fetch, pull (fast-forward only), open in
  `$EDITOR`, open in `lazygit`, copy path, reveal in file manager.
- Attention/health scoring in the core — urgency sort plus aging-unpushed and
  stale-stash detection — surfaced as a fleet triage summary.
- JSON snapshot cache for instant warm start; TOML config with XDG paths;
  `cohors init` and `cohors scan` (JSON) commands.
- Privacy-safe sample-data generator for demos; CI (fmt/clippy/test on
  macOS + Linux, plus a wasm-core build) and a release-on-tag binary workflow.

<!-- Compare/release links intentionally omitted until a remote is configured. -->

