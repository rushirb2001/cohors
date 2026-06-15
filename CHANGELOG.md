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

## [0.3.57] — 2026-06-14

### Changed

- **New spider mark** — a two-tone design with a solid eyed body and shaded
  (`▒`) outer legs, giving it depth and a bit of personality.
- **The header packs to the left.** The brand block, divider, and info column are
  now sized to their content and grouped together instead of the info column
  floating against the right edge, removing the empty gap mid-line.
- **Compact header for narrow terminals.** When the window is too narrow to hold
  the taglines and info column, the header collapses to the spider, the `cohors`
  lede, and the watched directory — nothing else.

### Changed

- **The spider's legs are now half-block diagonals.** Four legs reach out from a
  small `▟█▙` body as solid `▀▄`/`▄▀` diagonals, for a cleaner, leggier spider
  than the corner-quadrant version.

## [0.3.55] — 2026-06-14

### Changed

- **The spider mark is leggier and less "droid".** The body shrank from a
  three-row block with two eyes to a single compact row, and eight quadrant-block
  legs now splay to the corners — reading as a spider rather than a robot head.

## [0.3.54] — 2026-06-14

### Changed

- **The header info column now shows session orientation instead of fleet counts**
  (which already live in the Attention panel right below). The three rows are the
  watched directory, the active config path, and the fleet's most recent commit
  (`active 2h ago`) — context that isn't shown anywhere else.
- Dropped the trailing em-dash from the header tagline.

## [0.3.53] — 2026-06-14

### Changed

- **The spider mark is now a true purple** (`#A855F7`) instead of the terminal's
  pinkish ANSI magenta, and the `cohors` wordmark matches it.
- **The header gained a right-hand info column.** A full-height divider splits the
  brand block from a one-glance summary: the watched directory (home-abbreviated),
  the repo / dirty counts, and a needs-attention / all-clear status line.

## [0.3.52] — 2026-06-14

### Changed

- **The header mark is now a pixel-art spider** instead of the shield — a chunky
  block-glyph body with two eyes and splayed legs, evoking a spider at the centre
  of its web (every repo a thread it holds). It renders in the app's purple
  (`Color::Magenta`) and stays `NO_COLOR`-safe: the eyes are gaps and the body is
  solid blocks, so the silhouette reads even without colour.

## [0.3.51] — 2026-06-14

### Changed

- **The header is now a logo lockup** — a shield mark built from block glyphs
  (`▟█▙ / ▜█▛ / ▀`) beside the `cohors` wordmark, version, and tagline. The box
  grew to five rows to hold it; the mark renders in the accent colour and is
  `NO_COLOR`-safe (no emoji, all monospace-stable glyphs).

## [0.3.50] — 2026-06-14

### Changed

- **The command-run view now matches the standup view's structure.** Its repo
  list and output are each wrapped in a titled box (`Repos` and the focused
  repo's name), with the same padding and a clean (cap-less) scrollbar — the two
  "list + detail" overlays were previously inconsistent (one boxed, one bare).

## [0.3.49] — 2026-06-14

### Changed

- **Shell mode (opening the palette with `!`) is now visually distinct.** The box
  title becomes " Shell command ", the border, prompt, and row switch to the
  warning accent, and the cheat sheet **collapses to just the highlighted `:!<cmd>`
  row** (with a `▌` marker) — so it's clear you're about to run a shell command,
  not a built-in verb.

## [0.3.48] — 2026-06-14

### Changed

- **Unified the two command surfaces into one.** The separate `!` "Run command"
  modal is gone; pressing `!` now just opens the single `:` command palette
  pre-seeded with `!` (the shell shortcut). One command line drives built-in
  verbs *and* shell.
- **Redesigned the palette as a proper two-column table** — a `:` prompt, a
  horizontal divider, then `command │ description` rows separated by a vertical
  divider (verbs in accent, descriptions dim). Much clearer than the previous
  flat hint rows.

### Removed

- The `CommandInput` mode and its modal (folded into the `:` palette).

## [0.3.47] — 2026-06-14

### Changed

- Both command modals now carry a **structured, colour-coded cheat sheet inside
  the box** instead of dumping hints on the footer. The `:` palette groups verbs
  by `act` / `shell` / `view` / `go` (verbs in accent, descriptions dim,
  placeholders shown), and the `!` runner explains what it does with example
  commands. Their footers are slimmed to just `⏎ run · Esc cancel`.

## [0.3.46] — 2026-06-14

### Changed

- The `:` command palette now renders as a **centered boxed overlay** (a
  " Command " box with a `:` prompt over the dimmed dashboard), matching the `!`
  command-runner's design, instead of a bare line in the top strip. The two
  command surfaces now look consistent.

## [0.3.45] — 2026-06-14

### Added

- **`:!<cmd>` runs a shell command across the target repos from the command line**
  — folding the `!` runner into the `:` palette, so one command line drives both
  cohors's built-in verbs and arbitrary shell. (`!` stays as the quick shortcut.)
  Groundwork toward a single selector-targeted command surface shared with the
  planned CLI `--select` and MCP.

## [0.3.44] — 2026-06-14

Tier 4: watch mode.

### Added

- **`cohors --watch`** keeps the dashboard live: it re-scans automatically every
  ~5 seconds while idle (not during a scan, an in-flight action, a command run,
  or while you're in an overlay/command line), so the board stays current
  hands-free.

## [0.3.43] — 2026-06-14

Tier 3: command mode.

### Added

- **A `:` command mode** (vim/k9s-style). Press `:` to get a command line, then:
  `:fetch` / `:pull` / `:push`, `:refresh`, `:standup`, `:sort name|dirty|recent`,
  `:dirty`, `:filter <text>` (or `/<text>`), `:help`, `:quit`, or a bare repo name
  to jump the cursor to it. Reuses the same handlers as the keybindings, so every
  verb behaves identically to its key. Unknown input shows an "unknown command"
  toast.

## [0.3.42] — 2026-06-14

Tier 2: a repo detail pane — inspect before you act.

### Added

- **`Enter` now opens a per-repo detail pane** (read-only, scrollable) showing
  the repo's recent commits (with colour-coded types), working-tree changes (with
  porcelain status), local branches (current marked), and stashes. Data is read
  off-thread, so the pane shows a brief "Reading repo…" state and never blocks the
  UI; `cohors demo` seeds it with sample data. New pure `cohors_core::RepoDetail`
  model + `cohors_git::repo_detail()` adapter. See ADR-027.
- **Groundwork for command mode (`:`)**: a pure, unit-tested command parser
  (`crate::command`) mapping `:fetch`/`:sort name`/`/wip`/`<repo>` etc. to typed
  actions — wired into a `:` input in a later release.

### Changed

- **`Enter` no longer opens the editor** (it opens the detail pane); the editor
  is reached via the **`o` "Open with…" picker**. Supersedes the `Enter` binding
  from 0.3.21.

## [0.3.41] — 2026-06-14

### Changed

- Made the two repo-list overflow hints consistent: both now read `{arrow} N
  more` (`↑ N more` / `↓ N more`), instead of the bottom one using a different
  `… N more ↓` form.

## [0.3.40] — 2026-06-14

### Changed

- The repo list now shows **both overflow hints at once** when repos are hidden
  above *and* below the visible window — `↑ N more` below the header and
  `… N more ↓` at the bottom — instead of only one. At the very top or bottom it
  still shows just the relevant one.

## [0.3.39] — 2026-06-14

### Changed

- Removed the **blank line between the repository column headers and the rows**
  (the header no longer carries a bottom margin).
- The `↑ N more` overflow hint (shown when scrolled to the end) now sits **just
  below the column headers** rather than above them, so the headers always stay
  at the top. The header is rendered as its own row to make room for it.

## [0.3.38] — 2026-06-14

### Fixed

- The repo list's overflow hint now sits on the correct side: `… N more ↓` at the
  **bottom** while repos remain below, and `↑ N more` at the **top** once you've
  scrolled to the end (the hidden repos are then above). Previously the `↑ N more`
  was wrongly shown at the bottom.

## [0.3.37] — 2026-06-14

### Changed

- Added a blank gap row between the standup's description sentence and the panes,
  so the text has breathing room above the Repos/commits boxes.

## [0.3.36] — 2026-06-14

### Changed

- The standup description is now a **flowing sentence** instead of a chip list:
  "You authored 130 commits this week across 5 repos, shipping 68 features,
  fixing 14 bugs, polishing 16 design changes, clearing 15 chores, and writing 8
  doc updates." Each commit type maps to a natural verb/noun clause, the nouns
  stay colour-coded by kind, and the sentence wraps.

## [0.3.35] — 2026-06-14

### Fixed

- The standup description no longer overflows. It's now a proper two-line block —
  a sentence (`You authored 129 commits this week across 5 repos`) above the
  colour-coded type breakdown (`68 feat · 16 design · 15 chore · …`) — and both
  **wrap** instead of clipping, with the breakdown capped at the top 6 types plus
  a `+N more`.

## [0.3.34] — 2026-06-14

### Added

- **Colour-coded commit types in the standup** — `feat` (green), `fix` (red),
  `design`/`style` (magenta), `chore`/`build`/`ci` (blue), `docs`/`content`
  (cyan), `refactor`/`perf` (yellow), unknown (dim). The colours appear both in
  the glance summary (`24 feat · 5 fix`) and on each commit's type prefix in the
  list, so the kind of work reads at a glance. Honours `NO_COLOR`.

## [0.3.33] — 2026-06-14

### Changed

- **The standup commits pane now has a highlighted cursor.** Focusing it (`→`/
  `⏎`) highlights a commit and `↑/↓` move that highlight, with the list scrolling
  to keep it in view — so scrolling is contextual instead of a free offset.
- The commits pane title shows the **commit count** (`payments · 24 commits`).
- The description above the panes is now an **at-a-glance summary of what you
  did** — `You authored 29 commits this week — 24 feat · 5 fix` (top commit
  types) — rather than UI instructions.

### Fixed

- The standup **scrollbar** now tracks the cursor correctly (proportional thumb,
  accurate position), replacing the previous free-offset scroll that could drift
  out of sync.

## [0.3.32] — 2026-06-14

### Changed

- The **standup view is shorter** — it's now sized to its content (the busiest
  repo's commit count, capped) and scrolls past that, instead of always taking
  86% of the screen and leaving a lot of empty space.
- Added a **dynamic description line above the two panes** — `Your commits <this
  week> · pick a repo (left) to read its commits (right)` — so the view explains
  what each column is and which window it covers.

## [0.3.31] — 2026-06-14

### Changed

- Gave the footer hints **breathing room**: 1-column padding inside each group
  box and spacing around the vertical column divider, so commands no longer
  stick to the borders.

## [0.3.30] — 2026-06-14

### Changed

- Added a **horizontal divider** inside each footer group box, separating the
  two-column hint grid from the full-width multi-word commands below it (edge to
  edge, touching the box borders).

## [0.3.29] — 2026-06-14

### Changed

- **Footer group boxes now use a two-column grid with a `│` divider, with the
  multi-word commands stacked full-width below it.** Short hints (`open`,
  `fetch`, `pull`, `push`, `stash`; `move`, `sort`, `help`, `quit`; `filter`,
  `clear`, `mark all`) pack two-up; longer ones (`run command`, `mark repo`,
  `dirty-only`, `standup`) get their own full-width rows. Because the columns
  only ever hold short hints, they **stay two-up even on a compact terminal**
  (no more collapsing to one column), and there's no inner padding eating the
  width.

## [0.3.28] — 2026-06-14

### Changed

- The action/scan toast now rides the **top-right of the Repositories box
  border** (e.g. `Repositories (5) · sort: dirty-first ──── ✓ pushed 3 repos`)
  instead of floating as a separate box — cleaner, no overlap with the footer.
  Still colored (green `✓` / red `✗` / dim spinner) and self-dismissing.

## [0.3.27] — 2026-06-14

### Changed

- **Moved transient feedback out of the header into a self-dismissing toast.**
  Scan progress and action results (fetch/pull/push/stash/copy/…) now appear in a
  small floating box at the bottom-right, above the footer — green `✓` for a
  confirmation, red `✗` for a failure, a spinner while scanning. In-progress
  messages stay until the work finishes; results clear themselves after a few
  seconds. The header box is now purely the cohors name, version, and tagline.

## [0.3.26] — 2026-06-14

Tier 1 of the pro-grade push: closing the unpushed loop.

### Added

- **Push (`P`), single and bulk.** Pushes the current branch to its upstream
  across the marked repos (or the current one), with the same live per-repo
  status and aggregate summary as fetch/pull. It never passes `--force`, so a
  non-fast-forward push is rejected by git (reported as "rejected (pull first)")
  rather than overwriting remote history — resolving the #1 attention reason
  (unpushed) without leaving the dashboard.

## [0.3.25] — 2026-06-14

### Changed

- The repo list's scroll hint now renders **on its own centered row inside the
  box** (bold accent) instead of on the bottom border, and shows `↑ N more` once
  you've scrolled to the bottom — so the affordance reads as content, not chrome.

## [0.3.24] — 2026-06-14

### Changed

- The repo list's `… N more ↓` overflow hint is now **centered on the bottom
  border and rendered in bold accent** (was dim and right-aligned), so it's
  clearly visible instead of tucked into the corner.

## [0.3.23] — 2026-06-14

### Added

- **A scroll affordance on the repository list.** When the fleet is taller than
  the window, the list's bottom border shows a dim `… N more ↓` so it's obvious
  there are repos below the fold; it disappears once everything fits. The table's
  scroll offset is tracked so the count is accurate as you move.

### Notes

- Audited the TUI's glyphs against what renders reliably in terminals — `↑`/`↓`
  (Arrows block), `●` (Geometric Shapes), `▌` (Block Elements), the rounded box
  borders (Box Drawing), and `…` (U+2026) are all in the well-supported ranges,
  so the structure is stable across modern monospace fonts. No glyph changes
  needed; this was a deliberate check, not an assumption.

## [0.3.22] — 2026-06-14

### Added

- When the "Open with…" picker finds **no editor CLI on your `PATH`**, it now
  shows a short note ("No editor CLI found on your PATH. Install its shell
  command to list it here.") above the still-available Reveal / lazygit options,
  instead of silently listing only those.

## [0.3.21] — 2026-06-14

Opening a repo, done properly.

### Added

- **An "Open with…" picker (`o`)** that **auto-detects the editors installed on
  your `PATH`** — VS Code, Cursor, Zed, Sublime, JetBrains, Windsurf, then
  nvim/vim/helix/emacs/… — alongside "Reveal in file manager" and lazygit. Pick
  one with `↑/↓ · ⏎`, or press `d` to **set it as your default** (remembered
  across runs in a small prefs file, so your `config.toml` is left untouched).

### Changed

- **`Enter` now opens the default editor** (resolved from your saved pick →
  `editor`/`$EDITOR`/`$VISUAL` → the first detected editor). The **first time**
  you press it with no default set, it opens the picker so you choose once.
- **`o` is now "Open with…"** (reveal-in-folder moved inside the picker). The
  old dead-end "no editor configured" message is gone.

### Notes

- A modifier+Enter trigger (e.g. `Cmd`/`Ctrl`+`Enter`) was considered but isn't
  reliably deliverable to a terminal app, so the picker uses the dedicated `o`
  key instead. See ADR-026.

## [0.3.20] — 2026-06-14

### Changed

- Moved the **sort mode into the Repositories box title** (`Repositories (18) ·
  sort: dirty-first`, plus `· dirty-only` when that filter is on) and **dropped
  the redundant repo count from the header** — the box title already carries it.
  The header's right side now shows only the live selection count and the
  transient status toast.

## [0.3.19] — 2026-06-14

### Changed

- The Attention count moved into the **box title** — `Attention (13 of 18
  repositories)` — so the body is just the category pills. Removes the separate
  summary sentence entirely.

## [0.3.18] — 2026-06-14

### Changed

- The **Attention** panel now reads on a single line — the category pills
  (`1 unpushed · 11 dirty · 2 stashed`) sit right after the summary's colon
  instead of wrapping to their own row, so the box is one row shorter.

## [0.3.17] — 2026-06-14

### Added

- **A hide/show toggle for the hint bar.** A divider line above the footer reads
  `── press h to hide hints ──`; pressing **`h`** collapses the three group boxes
  to just that line (now `── press h to unhide hints ──`), handing all the
  reclaimed rows to the repository list. Press `h` again to bring them back.

## [0.3.16] — 2026-06-14

### Changed

- **Dropped the footer's outer box** — the three `select`/`act`/`view` boxes now
  sit directly in the footer area, reclaiming two rows and a little width.
- **The boxed footer is now the only Normal-mode layout** (the stacked-rows
  fallback is gone) and it adapts to the width itself: `act`/`view` use two
  internal columns when there's room and collapse to one on a compact terminal,
  so the keys are never clipped however narrow the window gets.

## [0.3.15] — 2026-06-14

### Changed

- **The footer is now a box-of-boxes.** Instead of three stacked, wrapping rows,
  Normal mode shows one outer box holding three titled group boxes —
  **`select`**, **`act`**, **`view`** — side by side. The busier `act` and `view`
  groups lay their keys out in two internal columns (so no group runs more than a
  few rows), and the live action target rides the `act` box's bottom edge
  (`→ <repo>` / `→ N selected`). On a narrow terminal it falls back to the
  previous stacked-rows footer so the keys never get squeezed.

## [0.3.14] — 2026-06-14

### Changed

- **Redesigned the `?` help into a single aligned grid.** The legend was a
  run-on paragraph that wrapped mid-phrase; it's now one short, non-wrapping
  `glyph → meaning` row per symbol, and the keymap shares the exact same
  two-column layout, so every description lines up in one column. The overlay is
  a touch wider with proper inner padding so it reads as a structured reference.

## [0.3.13] — 2026-06-14

Try cohors in five seconds.

### Added

- **`cohors demo`** — launches the dashboard on a built-in, privacy-safe sample
  fleet with **no config, no scanning, and no disk/network access**. Every column
  and view is populated (ahead/behind, dirty, stashed, CI pass/fail/pending, open
  PRs, a detached HEAD, an off-remote repo, an unreadable repo); the standup is
  seeded from demo commits and the command runner is simulated. Real actions are
  friendly no-ops. The generator lives in the pure core (`cohors_core::demo`) so
  the future web playground reuses the exact same data. See ADR-022.

## [0.3.12] — 2026-06-14

Making the dashboard explain itself.

### Added

- **A legend in the `?` help** that decodes the columns with their real colored
  glyphs: what `↑2`/`↓5`/`·`/`—` mean in Sync, the CI dot colors and `Npr`, the
  staged-vs-unstaged colors and `s1` in Changes, and what a dim vs red row (and
  the `●` marker) signify. New users no longer have to guess what they're seeing.
- **A live action-target hint** on the footer's `act` row: it now says exactly
  what an action will hit — `→ acts on 3 selected`, or `→ acts on <repo>` when
  nothing is marked — so the "marked set, else cursor" rule is visible, not
  inferred.

### Changed

- The footer's `view` row now surfaces **`s` sort** and **`d` dirty-only**, which
  were previously only discoverable inside the help overlay.
- Action feedback is now a **brighter, auto-clearing toast**: confirmations show
  green with a `✓`, failures show red, and the message clears itself a few
  seconds after the work finishes instead of lingering in the header.

## [0.3.11] — 2026-06-14

### Changed

- The **Sync** and **Changes** columns are now sized to their actual content
  instead of a fixed width, so they stay tight. A fleet with no open PRs gets a
  narrow Sync column (just the dot/arrows) rather than always reserving room for
  a `● Npr`; the column grows only when some repo needs it (clamped to a sane
  max). Sync floors at its header width, Changes at `"Changes"` (7).

## [0.3.10] — 2026-06-14

### Fixed

- Tightened the fused **Sync** column: it was too wide (left a large empty gap
  before Changes) and showed a redundant `·` next to the remote dot for repos
  that are even with upstream (two tiny dots with a gap that read as wasted
  space). The `·` is now dropped whenever the remote dot is present, so an
  even, tracked repo shows just `●`, and the column is narrower.

## [0.3.9] — 2026-06-14

### Changed

- **Sync and Remote columns fused into one `Sync` column.** It now shows the
  upstream delta and the remote health side-by-side — e.g. `↑2 ● 2pr` (2 ahead,
  CI passing, 2 open PRs), `↓5 ●` (5 behind), `· ●` (even), or `—` for a purely
  local repo. Five data columns become four, leaving more room for the commit
  subject while keeping all the signal.

### Added

- **Responsive two-pane overlays.** The standup and command-runner views now
  adapt to the terminal width: side-by-side (list + detail) when there's room,
  and **stacked** (list on top, detail below) on a narrow terminal, so neither
  pane gets squeezed. The command-runner also gains a small gap between its repo
  list and output for clarity.

## [0.3.7] — 2026-06-14

### Changed

- **Repository table columns reworked for a more compact, aligned read.** The
  separate **Stash** column is gone — stash count is now folded into **Changes**
  as a dim `s{n}` suffix (e.g. `4 s1` = 4 changed files, 1 stash), freeing width
  for the commit subject.

### Fixed

- The **Repo** header now lines up with the repository names: it accounts for the
  two-column selection gutter (`●`/blank) that prefixes each name, instead of
  sitting two columns to its left.

## [0.3.6] — 2026-06-14

### Changed

- The standup view now wraps each pane in its own padded, titled box ("Repos"
  and the focused repo name), with the active pane's title bold — more breathing
  room and clearer structure.

### Fixed

- The standup scrollbar now renders cleanly inside the commits box (proportional
  thumb, no stray arrows) instead of cramped against the outer border.
- You can read a repo's full commit history with arrow keys alone: `→`/`⏎` (or
  `Tab`) focuses the commits pane so `↑/↓` scroll them, `←` goes back — no
  PgUp/PgDn required.

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

