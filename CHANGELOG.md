# Changelog

All notable changes to **cohors** are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Versioning policy

Each number says what a release holds (ADR-059):

- **MAJOR** (`X.0.0`) — a headline change in what cohors is, or a break in its
  public contract (command and flag names, the `scan` JSON, the config file).
  While cohors is `0.x`, a contract break bumps MINOR instead and is listed
  under **Breaking**; `1.0.0` is the deliberate freeze.
- **MINOR** (`0.x.0`) — anything you can newly do: a command, a flag, a setting,
  a view, an install channel.
- **PATCH** (`0.x.y`) — fixes and refinements, with nothing new to learn.

Every piece of work gets its own version as it lands: a big feature is a
MINOR version, and the fixes, refinements and small features after it are
PATCH versions, so the history reads 0.8.1, 0.8.2, 0.8.3 … .
`scripts/version.sh patch|minor|major` moves **[Unreleased]** under the new
version and bumps it everywhere it is written (it refuses less than a
**Breaking** entry needs); the commit message never carries the number.

A release is a separate, public step: `scripts/release.sh` tags the version
the tree is at (`vX.Y.Z`), and its notes cover every version since the
previous tag. The version in `Cargo.toml` (`[workspace.package]`) is the single
source of truth.

## [Unreleased]

## [0.8.26] — 2026-11-16

### Added

- **A note when a session starts where work was left behind.** With the
  Claude Code plugin, opening a session in a repository that has work sitting
  in a linked worktree (commits that are on no remote, or files that were
  never committed), or commits of its own unpushed for more than two days,
  gets one line: `cohors: payments has work that exists only on this machine:
  5 commits and 1 uncommitted file in 1 worktree, untouched for 9 days. Ask
  Claude for the details.` Claude is given where each piece is, as
  information, with the instruction not to commit, push, move or delete it
  unless you ask. A repository with nothing left behind gets nothing. A
  worktree touched in the last hour is left out, since someone is working in
  it. You are shown the same note at most once a day, sooner when what it
  says changes. It reads only the repository the session is in, takes about
  a tenth of a second, and connects to nothing.
- **"Session notes"** in the plugin's settings turns it off
  (`claude plugin configure cohors@cohors`). Without the plugin, add
  `cohors hook session-start` as a `SessionStart` hook in your own Claude
  Code settings; `cohors hook --help` shows how.
- **`touched`** on each linked worktree in the scan JSON: when its
  uncommitted files were last changed.

## [0.8.25] — 2026-11-16

### Fixed

- **The dashboard repairs its own screen.** It sends the terminal only what
  changed since the last frame, so when the screen was wiped or written over
  from outside (Cmd+K, a pane closed and reopened onto a dashboard that kept
  running, a multiplexer re-attaching, a line printed by another program) it
  stayed broken: blank or garbled except for the few characters that happened
  to change, most visibly the header, whose clock changes every minute. The
  whole screen is now sent again every 10 seconds, with no erase first so
  nothing blinks, and **Ctrl+L** redraws it at once, as in a shell.
- **Git and ssh can no longer ask a question across the dashboard.** A fetch,
  pull or push that needed an HTTPS password, a key's passphrase or a new
  host confirmed wrote the question into the terminal, usually over the
  header, and then took some of the keys typed at the dashboard. Started from
  the dashboard, the MCP server or the web page, git now fails at once with
  its own message instead (`terminal prompts disabled`), reported like any
  other failure; a credential helper, an ssh agent or an askpass program you
  set up keeps working. A command run at a terminal (`cohors push api`) can
  still ask.

## [0.8.24] — 2026-11-16

### Added

- **cohors sees the work left in linked worktrees.** A coding agent usually
  works in a git worktree (Claude Code keeps them under `.claude/worktrees/`),
  and what it leaves there was invisible: a repo with five unpushed commits in
  an agent's worktree read "No work is at risk". Each repo's linked worktrees
  are now read with it, wherever they are on disk: commits that are on no
  remote, and files that were never committed. Either one puts the repo under
  **Work at risk** (unpushed commits older than two days are a RISK), with a
  reason that names the worktree, `5 commits not pushed in worktree agent-a`,
  and one **Do next** step, `Back up 5 commits and 3 uncommitted files sitting
  in 2 worktrees`. Those commits count in **Not backed up**. A worktree whose
  folder was deleted is reported too.
- **`cohors show`** lists a repo's worktrees with what each holds and its
  folder; `cohors scan --select worktree-work` (and `{"worktree_work": true}`
  for an agent) picks the repos that have any; the MCP `list_repos` returns
  the worktrees that hold work, each with its path. The scan JSON gains
  `worktrees` on every repo.

### Changed

- **A worktree's folder is no longer counted as an uncommitted file of the
  repo around it.** `git status` lists a worktree checked out inside a repo as
  an untracked folder, so every repo with agent worktrees read as dirty; what
  the worktree holds is reported as its own state instead.
- **A worktree stays a part of its repo**, not a repo of its own: the count of
  repositories does not change, and fetch, pull, push, commit, stash and run
  still act on the main checkout only. A worktree kept beside its repo, under
  a folder cohors searches, is still its own row, as before, and is then not
  listed a second time under its repo.

## [0.8.23] — 2026-11-16

### Added

- **`--report` prints the report a person reads even when the output is
  piped**, as plain text with no colour codes. A pipe still gets JSON without
  it, so scripts are unchanged, and `--json` wins when both are given. It is
  for a coding agent calling cohors through a shell: on a fleet of 27
  repositories `cohors overview` piped is 39,700 characters of JSON and
  3,200 with `--report`, `cohors scan` 89,400 and 4,200. It works on every
  command that has a report (`overview`, `scan`, `next`, `launchpad`, `prs`,
  `changes`, `show`, `search`, `discover`, `watch`, `agent ls`), before or
  after the command's name.

## [0.8.22] — 2026-11-16

### Changed

- **What `cohors mcp` tells an agent is several times smaller.** Measured on
  a fleet of 27 repositories: `list_repos` went from 102,000 characters to
  about 15,000, `overview` from 40,000 to 11,000, `standup` from 66,000 to
  20,000 and `next` from 12,700 to 5,300. Every answer is compact JSON now
  (indentation was a fifth to two fifths of each one), and these four reads
  are brief unless asked otherwise: `list_repos` leaves out per-branch
  status, the weekly activity series and anything empty, keeps a repo's
  GitHub data to its checks and open pull requests, and returns the first 50
  repos (`omitted` counts the rest, `limit` raises it); `overview` lists the
  20 most urgent repos that have a reason, in words, and counts the clean
  ones; `standup` is the markdown digest without the same commits a second
  time as records; `next` lists 8 more sessions rather than 20. `changes`
  lists at most 100 files a repo (`max_files`), and says how many it left
  out.
- **`detail: "full"` on `list_repos`, `overview`, `standup` and `next`
  returns everything**, field for field what `cohors scan --json` and
  `cohors overview --json` print, and `fields` on `list_repos` now takes
  `branches`. A test keeps the brief answers inside a size budget.

## [0.8.21] — 2026-11-16

### Fixed

- **The package pages say what is new in 0.8**, not 0.7.7, on npm, PyPI,
  crates.io and the Claude Code plugin's page: the self-updating install,
  `cohors doctor` and `cohors install`, commands named like the MCP tools,
  commands that ask before they change anything, exit codes and shell
  completions.
- **Every page's MCP setup points at the copy that keeps itself up to date**
  (`~/.local/bin/cohors`, registered for every folder with `--scope user`),
  with each channel's own path for when updates are off. The npm page no
  longer sends you to a copy npm never updates.
- **Two names in this changelog read right again**: `COHORS_NO_UPDATE` and
  `COHORS_DISABLE_UPDATES` in 0.8.0 and 0.8.1 had a date pasted into them.
  `scripts/version.sh` and `scripts/release.sh` now refuse a date run into a
  word.

## [0.8.20] — 2026-11-16

### Fixed

- **The documentation describes cohors as it is now.** The README, the
  package pages (PyPI, npm, crates.io, Docker), the editor extension's and
  the plugin's pages and the launchpad spec no longer say updates come once a
  day or only to Homebrew, pip and shell-installer copies, or use the old
  `cohors agent run` forms; they cover the self-updating install and
  `cohors doctor`, the mouse and the first-run tips, and commands that ask
  before they change anything. The changelog's versioning policy says how
  versions and releases work now.

## [0.8.19] — 2026-11-16

### Changed

- **`cohors --help` groups its commands by what they are for**, each group
  with a line that says so: *See what needs you* (reports; nothing changes),
  *Change your repositories* (each lists the repositories and asks before it
  acts), *Agents and the launchpad*, *Other ways to look*, and *Set up and keep
  cohors current*. A test makes sure every command is in exactly one group.

## [0.8.18] — 2026-11-16

### Changed

- **A command that changes your repositories asks first.** At a terminal,
  `fetch`, `pull`, `push`, `commit`, `stash` and `run` list the repositories
  they are about to change (a commit with the files that would go in) and
  ask `Pull 5 repositories? [y/N]`; only `y` goes ahead. `--yes` (`-y`) skips
  the question, `--dry-run` shows the same list without asking, and a script
  with no terminal is never asked.

## [0.8.17] — 2026-11-16

### Fixed

- **`cohors prs` says what it is doing instead of sitting silent.** At a
  terminal it says it is reading your repositories, then that it is asking
  GitHub about them, read-only, and counts the answers as they arrive
  (`Asking GitHub about 27 repositories (read-only; Ctrl-C stops it)…
  12 of 27`). It now asks only about the repositories you selected, not the
  whole fleet.

## [0.8.16] — 2026-11-16

### Changed

- **Every overlay fits the screen it is on.** On a narrow terminal the
  standup and a command's results drop their repository column (the header
  then says which repository of how many it shows), a repository's detail
  becomes one column that scrolls, and the command line drops its
  descriptions. Footer keys that do not fit are left off whole, never cut in
  half, and the one that closes always stays. The help scrolls (`↑/↓`, the
  wheel) when it is taller than the screen, and a click inside it no longer
  closes it. A test draws every screen and overlay at twelve sizes, from
  48×12 to 240×70.
- **The start screen says what each name means.** Each folder's
  repositories are listed in rows labelled *at risk*, *to look at* and
  *clean*, each with its count, instead of by colour alone, and a tip about
  the dashboard's keys says what to press (`press / in the dashboard`).

## [0.8.15] — 2026-11-16

### Added

- **The web dashboard says when a newer cohors is out,** at the top of every
  page, in the same words as the terminal dashboard, with its command to
  copy.

## [0.8.14] — 2026-11-16

### Added

- **Run one command in many repositories from VS Code.** Tick repositories
  and choose Run…: the command is shown exactly as it will run before
  anything starts, and it needs a trusted workspace.
- **A repository's panel in VS Code shows it in full:** changed files, every
  branch against its upstream, stashes, recent commits, and with a GitHub
  token its open pull requests and CI.
- **VS Code says when a newer cohors is out,** in the dashboard and the
  sidebar, with the command to copy. `cohors watch --json` lines now carry
  `update`, the shape the MCP server's `meta.update` has.

## [0.8.13] — 2026-11-16

### Changed

- **`cohors mcp --allow-open` is deprecated.** It never did anything: the
  `open` tool it was meant for was never built. A server registered with it
  still starts, and says on stderr that the flag can go.

## [0.8.12] — 2026-11-16

### Fixed

- **The plugin's agents can use cohors.** The `cohors-plan` and
  `cohors-setup` agents were not allowed the cohors tools; now they are.
- **The plugin runs the cohors that keeps itself up to date.** When cohors
  installed itself (`~/.local/bin/cohors`), the plugin starts that copy
  before a Homebrew or Cargo one that may be older.

## [0.8.11] — 2026-11-16

### Added

- **Claude can find repositories buried inside others.** The MCP server has
  a `discover` tool: every repository under your roots, with `deep: true`
  also those only a deep search reaches and which of them the dashboard
  does not show yet. It changes nothing; it names the command for you to
  run (`cohors discover --deep`) to add them.

## [0.8.10] — 2026-11-16

### Added

- **Claude can brief you on part of your fleet.** The MCP `overview` tool
  takes a selector (`{"group": "payments"}`, `{"name": "api*"}`), as
  `cohors overview --select` does: next steps, figures and repositories then
  cover only those.

## [0.8.9] — 2026-11-16

### Added

- **Claude finds a repository the way you name it.** The MCP tools accept a
  unique prefix (`paym` for payments), as `cohors path` does. A name two
  repositories share is no longer quietly the first one: the answer lists
  each with its path. A typo gets "did you mean: payments?". Writing a
  repository's launchpad file still needs its exact name.

## [0.8.8] — 2026-11-16

### Added

- **Claude can ask "where did I leave off?" across your repositories.** The
  MCP server has a `next` tool: the agent conversations to resume (each with
  its `cohors agent --resume` command), the fleet's next steps, your focus
  board and up to two setup tips, the same list `cohors next --json` prints.
  Session titles and commands are passed on as information, never followed.

## [0.8.7] — 2026-11-16

### Added

- **`tools.json` lists cohors' MCP tools for directories that do not run
  servers.** The repository's root now carries the tool catalog exactly as
  `cohors mcp` answers `tools/list`: 22 tools, each with its description,
  input schema and read or write hints, so a directory whose scanner cannot
  read tools out of Rust source (mcprush's, for one) can list them. A test
  fails whenever the catalog changes and the file was not regenerated
  (`scripts/tools-json.sh`).

## [0.8.6] — 2026-11-16

### Added

- **The mouse works inside every overlay too.** A click outside one closes
  it (a confirmation is cancelled, never accepted); a click picks a row in the
  "Open with" and "push which branch" pickers (and a second click takes it),
  a repository or a commit in the standup, and a repository in a command's
  results; every key in an overlay's footer (`y copy`, `w window`,
  `y confirm`, `Esc close`…) is a button; and the wheel scrolls a
  repository's detail and the task board.

## [0.8.5] — 2026-11-16

### Added

- **Thirty first-run tips teach cohors one thing a day.** After the setup
  tips, the dashboard's tip line and the start screen show one tip for
  someone new to cohors: the dashboard's keys first (jump to a card, filter
  with `/`, the standup, marking repositories and acting on them all, commit
  and stash, open, run, start an agent…), then commands worth knowing
  (`cohors next`, `cohors search`, `cd "$(cohors path NAME)"`,
  `cohors pull --select behind --dry-run`, `cohors discover --deep`…). Each
  stays up for a day, then the next takes its place; `x` moves on at once,
  and `i` copies a command tip. `COHORS_NO_TIPS=1` turns them off.

## [0.8.4] — 2026-11-16

### Added

- **The dashboard answers the mouse.** Click one of the cards 1–5 to give it
  the focus, click a row to put the cursor on it, and click that row again to
  open it, as Enter would. Every key shown on a card's bottom edge, on the key
  bar and on the tip line is a button, and the wheel scrolls the card under
  the pointer. In the detail view a click picks a repository or one of its
  three cards; with an overlay open, clicks go to the overlay (a click closes
  the help). A click never does more than a key would: anything that asks
  before it acts still asks.

## [0.8.3] — 2026-11-16

### Fixed

- **`cohors doctor` never runs a `cohors` from the current folder.** It asks
  each copy on your PATH for its version; an empty or relative PATH entry
  (`.`), or a copy in a folder other users can write to, is now skipped
  instead of run, so a repository with its own `cohors` file cannot run code
  when you ask doctor what is installed.

## [0.8.2] — 2026-11-16

### Added

- **Setup tips point at the parts of cohors you have not used yet.** When the
  web dashboard was never opened, VS Code, Cursor or Windsurf is installed
  without the cohors extension, or Claude Code is installed without cohors
  in it, cohors says so below the real work: one `Tip` line under the
  dashboard's header (`i` copies the command, `x` hides it), one line on the
  start screen, the end of `cohors next` (and its `--json`), and the web
  page's "Pick up where you left off". A tip goes once it is done, once you
  hide it (`cohors next --dismiss web|extension|claude`), or after it has
  shown on five separate days; `COHORS_NO_TIPS=1` turns them off. Finding
  out only reads files on your machine; nothing is sent anywhere.

## [0.8.1] — 2026-11-16

### Added

- **Every copy hands over to cohors' own install.** The first run of a copy
  from Homebrew, pip, pipx, uv, npm or `cargo install` sets up cohors' own
  install in `~/.local/share/cohors` (a copy of itself, so no download) with
  `~/.local/bin/cohors` linked to it, and says so once. From then on every
  run hands over to the newest version there, whichever `cohors` your PATH
  finds. Not in CI, for builds from source, or with `updates = "off"`,
  `COHORS_NO_UPDATE` or `COHORS_DISABLE_UPDATES`; `"notify"` sets nothing up.
- **`cohors uninstall`** (try `--dry-run` first) removes cohors' own copy and
  its update files, and names the command that removes any copy a package
  manager installed (`brew uninstall cohors`, `pip uninstall cohors`,
  `npm rm -g @cohors/cli`).

## [0.8.0] — 2026-11-16

### Added

- **cohors updates itself, the way Claude Code does.** cohors' own install
  downloads the release for your platform, checks its SHA-256, tries the new
  binary, then switches to it, keeping the two newest versions. A binary that
  fails its check is never run.
- **`cohors install [version]`** sets that install up (or reinstalls it) by
  hand and says whether typing `cohors` runs it.
- **`cohors doctor`** says which cohors you ran and whether it handed over,
  every cohors on your PATH and where it came from, the versions kept,
  whether updates are on and why, the last update, and what to fix. It
  changes nothing.
- **`COHORS_DISABLE_UPDATES=1`** blocks every update, `cohors update` and
  `cohors install` included; `COHORS_NO_UPDATE` still stops only the
  background checks.

## [0.7.16] — 2026-11-16

### Fixed

- **`uv tool install` copies are recognised** (they were taken for `uvx`) and
  told `uv tool upgrade cohors`.

## [0.7.15] — 2026-11-16

### Changed

- **Each copy asks the place it installs from:** Homebrew's tap, PyPI, npm,
  crates.io or GitHub releases, so cohors never offers what that channel
  cannot install yet (Homebrew and pip updates used to start before the tap
  and PyPI had the release, fail, and stay failed for a day). A copy that
  only tells names the exact command (`npm i -g @cohors/cli@latest`,
  `cargo install cohors`); a failed update says "Auto-update failed · run
  cohors doctor" and is not tried again for a day.

## [0.7.14] — 2026-11-16

### Changed

- **A command you run at a terminal looks for a newer cohors while it runs**
  and says so at the end, never waiting on the network. It used to repeat
  only what the dashboard had last found.

## [0.7.13] — 2026-11-16

### Changed

- **The dashboard looks for a new release when it opens and every five
  minutes while it runs**, and says so in its header the moment it knows:
  "Update available: 0.7.8 — updating…", then "Updated to 0.7.8 — restart to
  use it (R)". An open dashboard used to look once. Each look is a
  conditional request, so it usually costs an empty "not modified".

## [0.7.12] — 2026-11-16

### Fixed

- **A new release is noticed right away.** It could stay hidden for up to a
  day behind the once-a-day check.

## [0.7.11] — 2026-11-16

### Changed

- **The dashboard's commit chart is one solid block.** The last twelve weeks
  sit two rows tall at the header's top right, each week's bar filled with
  block glyphs and the space above it with braille dots, right-aligned with
  the line under it (`12 weeks · scanned 2m ago`), instead of thin bars
  floating in blank space over a caption out of line with them. The
  plain-text look keeps its one-row ASCII chart.

## [0.7.10] — 2026-11-16

### Fixed

- **The headline counts every repository**, on every surface (dashboard, web,
  editor, MCP): `29 repositories. 2 have work at risk, 6 need a look, 21 are
  clean.` Before, the repositories that needed a look were left out, so the
  numbers did not add up.

## [0.7.9] — 2026-11-16

### Fixed

- **The start screen says what it found and where.** Each folder is listed
  with how many repositories it holds, how many are at risk, need a look or
  sit inside another repo, and the repositories by name (at risk first and
  marked); each "do next" step names the repositories it is about.

## [0.7.8] — 2026-11-16

### Fixed

- **`cohors watch` keeps escape codes in repository names off the terminal.**
  Its human output now filters control characters from repo names and the
  headline, like every other view (the JSON stream was never affected).

## [0.7.7] — 2026-11-16

### Added

- **Repos nested inside a repo are found.** When a folder you point cohors at
  is itself a git repo (say `~/code`, tracked for your own notes), it is now
  searched too, so `~/code/projects/api-dashboard` and the rest show up beside
  it. Inside a repo, build output and dependency folders (`target`,
  `node_modules`, `vendor`, …) are skipped, and a repo's own submodules and
  inner worktrees are not listed a second time.
- **`cohors discover`** lists every repo a search finds without reading them:
  by folder at a terminal (nested ones marked), one path per line in a pipe,
  and `--json` for scripts.
- **`--deep` on `cohors discover` and `cohors scan`** searches every folder at
  any depth, inside repos too. What only it finds is remembered, and a running
  dashboard, the web page and the MCP server pick it up without a restart.
- **Every dashboard follows changes live.** The terminal dashboard (now by
  default), the web page and the VS Code extension show a commit, push, pull,
  fetch, new repo or a repo found by `--deep` within about half a second, and
  an edited file within 3 seconds, all from one change source; the web page
  gets them pushed over server-sent events. `cohors watch --json` streams the
  same changes as one JSON line each, for scripts and editors. The web page
  and the VS Code sidebar also show where to pick up: the agent sessions to
  resume and the next steps.
- **`cohors next`: what to do now, each with its command.** It ranks the
  conversations you had with coding agents in each repo (Claude Code and
  Codex, found on this machine, read-only) with the fleet's next steps and
  your focus board; `cohors next 2` runs the second. `cohors agent --resume`
  with no id lists the recent sessions in the repo you are in to pick from,
  and a bare `cohors agent` at a terminal offers them before starting a new one.
- **`cohors agent [WHAT]`**: name a shipped prompt, a task or one of the
  repo's agents and cohors works out which (`cohors agent setup-repo`,
  `cohors agent rate-limit-backoff -r payments`), with `-w/--with` for the
  runner, `--print` for the launch as JSON and `cohors agent ls` for what you
  can start. The 0.7 forms (`agent run`, `agent command`, `agent runners`,
  `agent prompts`) still work.
- **Read commands named like the MCP tools**: `cohors show <repo>`,
  `cohors path <repo>` (`cd "$(cohors path pay)"`), `cohors changes`,
  `cohors search <text>`, `cohors prs`, and `cohors status` for `overview`.
- **Writes take repo names** (`cohors pull payments web`), `run` takes the
  command after `--` (`cohors run -s dirty -- cargo test`), and `fetch`,
  `pull`, `push`, `commit`, `stash` and `run` print their results as JSON
  with `--json`.
- **`cohors completions <shell>`** for bash, zsh, fish, elvish and PowerShell;
  every command's `--help` ends with examples, and `cohors --help` lists the
  exit codes (0 done, 1 failed or not found, 2 usage, 130 interrupted).
- **Each install channel has its own listing**: PyPI, npm, crates.io,
  Docker, the VS Code Marketplace, the Claude Code plugin and the release
  notes each say how to install, update and set cohors up on that channel.
- **`commit` shows what it will commit and won't commit a secret.**
  `commit --dry-run` lists each repo's files; a repo where a file that looks
  like a secret (`.env`, a private key) would go in is refused, on every
  surface (CLI, dashboard, MCP, web), unless you pass `--allow-secrets`.
  `--staged` commits only what you already staged.
- **A finished look for the start screen and every plain-terminal view**: a
  header band like the dashboard's, headings as full-width rules, filled
  severity chips and keycaps, with ASCII forms for `NO_COLOR`, `icons =
  "ascii"` and non-UTF-8 terminals.
- **A Docker image, `ghcr.io/rushirb2001/cohors`**, for linux/amd64 and
  linux/arm64. It runs the MCP server over the repositories mounted at `/repos`
  (`docker run -i --rm -v "$HOME/code:/repos" ghcr.io/rushirb2001/cohors`), and
  any other cohors command works too. It installs the release's own Linux
  binary after checking its checksum, and is published with every release.

### Changed

- **The dashboard refreshes on real changes without `--watch`**, and idles
  at a third of the CPU `--watch` used to (`--watch` still skips the launch
  screen).
- **`cohors init` run again says where your config is** and what it holds
  instead of failing; `--force` keeps the old file as `config.toml.bak`, and
  `--root` seeds the roots you name.
- **`fetch`, `pull`, `push`, `commit` and `stash` work on every selected repo
  at once** (the bounded pool the MCP server and the web already use), each
  line printed as its repo finishes: `fetch --select all` on 24 repos went
  from 1.4 s to 0.45 s.

### Fixed

- **Write commands report failure in their exit status.** `fetch`, `pull`,
  `push`, `commit`, `stash` and `run` exit 1 when a repo fails, so a script or
  CI step no longer reads a failed push as success. A selector with no `-m`
  (commit) or no command (run) exits 2 and says which is missing.
- **A rejected push says why**: `rejected (pull first)` instead of the
  remote's URL. Fetch, pull, commit and stash also report git's reason line.
- **`run --timeout` stops the whole job**, including what the command started
  (`a; b`, `npm test`, a backgrounded process), and Ctrl-C stops every running
  job instead of leaving them behind. A failed command shows its exit code.
- **Config mistakes are no longer silent.** A `--config` that names no file, or
  a `--root` that names no folder, is an error saying so; a broken config file
  shows the line and column; an unknown key is reported with a suggestion
  (`max_dept`: did you mean `max_depth`?).
- **Selectors**: a typo lists the whole vocabulary, and `branch:` takes globs
  (`branch:feat/*`).
- **`scan --select`** says how many repos were scanned and how many matched,
  and lists clean matches by name.
- **`cohors agent command`** no longer edits the repo's tracked `.gitignore`;
  cohors' ignore lines go to `.git/info/exclude`.
- `overview` figures read in the singular for one; `cohors mcp`'s registration
  lines carry the flags it was run with; `-s` is short for `--select` and `-n`
  for `--dry-run`; `--help` lists the global flags under their own heading.
- Discovery never lists a bare repository found inside another repo's working
  tree (a committed one is attacker-controlled), and `cohors discover` keeps
  control characters in folder names off the terminal.

## [0.7.6] — 2026-11-12

A distribution and documentation release: cohors now installs from every
common place, including your editor's extension store. The `cohors` binary
itself is unchanged from 0.7.5, so updating is optional.

### Added

- **The VS Code extension is in the VS Code Marketplace and on Open VSX.**
  Search "cohors" in the Extensions view of VS Code, Cursor, Windsurf or
  VSCodium, or run `code --install-extension rushirb2001.cohors`. It still
  needs the `cohors` tool installed.
- **A listing page for the extension**: what it shows, how to install the
  `cohors` tool first, and its commands and settings. The extension also has
  better categories, keywords and a gallery banner, so it's easier to find.
- **The `.vsix` is attached to every GitHub release**, for installing offline
  or in editors without a store.
- **Every release publishes the extension automatically**: to the Marketplace
  through Trusted Publishing (GitHub proves the build's identity, so no
  Marketplace token is stored) and to Open VSX with a publish-only token.

### Changed

- **A rewritten README.** Every install command (Homebrew, pip, npm, Cargo, the
  shell installer, and `uvx`/`npx` to run once) sits at the top, each in its
  own block to copy. Below it is a shorter tour that shows real output from
  the `cohors demo` sample fleet rather than describing it.
- **The README's tables and terminal output are drawn images** in the cohors
  palette, each with a light and a dark version that follows your GitHub
  theme. They're generated from real captures by `scripts/readme-art.py`, so
  they can be rebuilt when the output changes, and every image has a text
  description for screen readers.
- **The launch film's closing card lists all four installs** (Homebrew, pip,
  npm and Cargo). The studio build now renders the full film in 4K and adds a
  4:5 cut for social feeds.

### Fixed

- **A failed crates.io publish now fails the release job.** Before, piping the
  output through `tee` hid the failure, and a release could look published
  when it wasn't.
- **crates.io publishing waits out the limit on new crates.** When crates.io
  says "try again after", it waits ten minutes and tries again instead of
  giving up. It also always uses the newest publish script from main, so a
  fix to publishing also reaches a release that's already tagged.
- **The README's "Use it with Claude Code" link** still works now that the
  heading carries Claude's icon.

## [0.7.5] — 2026-11-12

### Fixed

- **cohors is on npm, as `@cohors/cli`.** npm refused the bare name `cohors`
  (too close to `cors` and `colors`), so 0.7.2 and 0.7.4 never reached npm. Run
  `npx @cohors/cli@latest`, install with `npm install -g @cohors/cli`, or give
  MCP clients `npx -y @cohors/cli@latest mcp`; the command is still `cohors`.

## [0.7.4] — 2026-11-12

0.7.3 was tagged but never published (its release step failed), so its
changes ship here.

### Added

- **cohors is on crates.io: `cargo install cohors`.** Every crate is published,
  and the browser dashboard ships prebuilt inside the `cohors` crate, so
  `cohors web` works with no Trunk and no checkout. A crates.io install is told
  about new releases and the command to get them (`cargo install cohors`), never
  rebuilt in place.
- **Homebrew and crates.io version badges** in the README, and a Cargo row in
  its install table.

### Changed

- **The binary's crate is now named `cohors`** (it was `cohors-tui`), so every
  channel uses one name. Release files follow: `cohors-installer.sh` and
  `cohors-<target>.tar.xz`. Each release still carries a
  `cohors-tui-installer.sh` that runs the new installer, so copies of cohors up
  to 0.7.2, and older instructions, keep working. The shell installer's
  settings are now `COHORS_INSTALL_DIR` and `COHORS_NO_MODIFY_PATH`; the old
  `COHORS_TUI_*` names still work through that script.

### Fixed

- **Updating through Homebrew refreshes the tap first.** Homebrew refreshes
  taps only about once a day, so `brew upgrade` could find the old formula,
  change nothing and still succeed, and cohors then asked for a restart that
  came back as the old version. It now runs `brew update` first and checks that
  Homebrew really has the new version before saying "Restart to update".

## [0.7.2] — 2026-11-12

### Added

- **cohors is on npm.** `npm install -g cohors`, or `npx cohors@latest` with
  nothing to install; MCP clients can run `npx -y cohors@latest mcp`. The
  package fetches the same binary from the GitHub release. An npm or
  npx copy is told about new releases and the command to get them, never
  updated in place.
- **The README's install section lists every channel in one table**, with
  the install command and the run-without-installing command for each, and PyPI
  and npm version badges.

## [0.7.1] — 2026-11-12

### Added

- **cohors keeps itself up to date.** Once a day the dashboard looks for a
  newer release and installs it in the background the way cohors was
  installed (Homebrew, pip or the shell installer), then says "New version
  available → 0.7.1 → Restart to update"; `R` restarts into it. A failed
  install is reported and not retried for a day. `cohors update` updates on the
  spot (`--check` only looks), one-off commands end with a one-line notice, and
  the MCP server tells your agent in every read's `meta`. A source build or
  `uvx` only gets the notice. The `updates` setting (`auto`, `notify`, `off`)
  and `COHORS_NO_UPDATE=1` control it; it never runs in CI.
- **The Claude Code plugin passes Anthropic's plugin directory checks.** It
  ships no hooks (the MCP server logs each session's start and end itself),
  starts its server through `sh` with every program named, and reads its
  settings under the directory's standard names.
- **One-click install for VS Code and Cursor**, through `uvx cohors@latest`.
- **A "Staying up to date" guide** in the README for every install channel, and
  a privacy policy (`PRIVACY.md`).

### Fixed

- **A failed Homebrew tap update no longer keeps a release off PyPI and the MCP
  Registry.** Packaging now waits for the GitHub release itself, not for every
  step of the release run.

## [0.7.0] — 2026-11-11

The launchpad release. Each repository can now say what you run in it and what
work is waiting, your coding agent can plan across all of them, and cohors
arrives as a Claude Code plugin whose settings decide what Claude may do.
Release builds also carry the browser dashboard inside the binary.

### Added

- **The launchpad.** cohors now reads what each repository says about itself:
  a committed `.cohors/repo.json` listing its flows (what to run), the agent
  prompts it ships, and its short-lived tasks. The format is an open
  contract, written down in `spec/v1/SPEC.md`. Flows are also detected with zero config from
  npm/pnpm/yarn/bun scripts, cargo, uv, Makefile targets, and gradle.
- **`cohors launchpad`** reports the focus board and every open task across the
  fleet (JSON when piped). `cohors launchpad validate [PATH]` checks one repo's
  contribution, `contract` prints the rules, and `record` logs an agent session.
- **Six launchpad MCP tools.** `launchpad`, `read_contract`, and `read_board`
  are always on. `write_repo_contribution`, `write_board`, and `record_activity`
  need the new `--allow-launchpad` flag; each accepts `dry_run` without it. A
  rejected write lists every problem at once and writes nothing.
- **A Claude Code plugin**, installable from this repository as a marketplace:
  the MCP server (found on your machine by a launcher, so the stripped-`PATH`
  problem goes away), skills for fleet work, setting a repo up, and planning
  focus, and hooks that log each session. The server starts read-only; three
  plugin settings, off by default, let Claude act: **Git actions**, **Run
  commands** and **Launchpad writes** (`/plugin configure cohors@cohors`). A
  refusal from the plugin's server names the setting to turn on.
- **`cohors web` works wherever cohors is installed.** Release builds carry
  the browser dashboard inside the binary (about 2 MB), so it starts at once,
  with no checkout of this repository and no Trunk or Rust toolchain. A build
  from source without the page still builds it with Trunk inside a checkout.
- **One overview, the same everywhere.** cohors now works out a single
  briefing for the fleet: what to do next and why (most urgent first, each step
  naming the repos it applies to), six key figures (work not backed up, stashes,
  uncommitted files, stale fetches, open tasks, commits this week), every repo
  with its reasons in plain words, open tasks per repo, and the last seven days
  of commits. The terminal, the browser, the editor extension and the MCP
  server all render that one model, so their numbers cannot disagree.
- **`cohors overview`** prints the briefing at a terminal, with the exact
  command that carries out each step, and the whole model as JSON when piped or
  given `--json`. An MCP `overview` tool returns the same thing to an agent.
- **`cohors standup`** prints your commits across every repo for the last seven
  days (or `--today`) as a paste-ready digest. Standup used to exist only inside
  the dashboard; it is now also an MCP `standup` tool and a page in the browser
  and the editor extension.
- **`cohors agent`** starts a coding agent on a repo, a task, or the whole
  fleet, with whichever of Claude Code, Codex, Gemini CLI or agy is installed.
  `cohors agent run setup-repo` asks it to read the current repo and write its
  launchpad file; `plan-day` plans your focus across every repo; `agent
  runners` shows what is installed and why a runner is unavailable. A task
  started this way is linked to its conversation and resumes there next time.
  The five prompts are also served to MCP clients as prompts, and in the
  dashboard `a` on a repo does the same after asking.
- **A VS Code extension** (`extension/`): the same overview, repositories, tasks and standup in an editor
  tab that follows your theme, reading everything from the `cohors` binary.
- **The VS Code sidebar is the overview.** The cohors container in the activity
  bar holds six panes drawn by the editor itself, so they open, close, reorder
  and hide like Explorer's: Overview (the figures), Do next, Work at risk, Open
  tasks, Commits and Repositories. Each carries its count on its header, the
  activity-bar icon shows how many repositories have work at risk, and a row
  opens the dashboard at that place.
- **The dashboard has the look the terminal has.** The mark, large, leads the
  page with the headline and twelve weeks of commits; each section is a card in
  its own colour; a count of zero is a dot. `cohors.openOnStartup` opens the
  sidebar, the dashboard or both when a window starts.
- **The extension says what it is doing.** The Output panel has a "cohors"
  channel with the binary in use and each command it ran, how long it took and
  whether it worked. `vscode://rushirb2001.cohors/show` opens the dashboard from
  a terminal or a link (it only ever opens the view).
- **A studio for the project's pictures and film.** `scripts/demo-fleet.sh`
  builds a made-up fleet of real repositories in a home folder of its own;
  `scripts/studio/build.sh` captures cohors on it and renders the README's
  pictures, post images for each place they are shown, and a 37-second film in
  three framings. Nothing in them is mocked up or taken from a real machine.
- **`scripts/package-extension.py`** builds the extension's `.vsix` with nothing
  but Python, so a release needs no Node tooling.
- **The launchpad contract is in this repository**, at `spec/v1/SPEC.md`, with
  its JSON Schemas.
- **`push` publishes a branch that was never pushed** (`git push -u origin
  HEAD`) instead of skipping it. Nothing is forced.
- **ARM Linux** release binaries and pip wheels.
- **`server.json`**, the manifest for the official MCP Registry. Each release
  now lists itself there, and uploads its pip wheels, as soon as the release
  build finishes, with no step by hand.
- **Every MCP tool carries annotations**: a readable title and whether it only
  reads, may destroy work, is safe to repeat, or reaches the network. Clients
  and MCP directories show these before a call. The server also answers
  `resources/list` with an empty list instead of an error.

### Changed

- **Every surface was rebuilt around what to do next.** A quiet neutral
  palette with purple kept for the brand mark and focus. Severity is a block
  with the word in it: a filled `RISK` (brick red) or `WARN` (ochre) leads the
  rows of Do next, Work at risk and the repository tables, a quiet outlined
  `NOTE` or `INFO` marks the rest, and a clean repo has none. The key figures
  that need attention are filled tiles in the same two colours. Because the
  word carries the meaning, it reads without colour and under `NO_COLOR`
  (where the block is reverse video).
  - **Terminal:** the dashboard opens on cards (Do next, Work at risk, Open
    tasks, Commits, Repositories). `1`–`5` focus a card and each card's border
    lists the keys that act on it. Enter on the repositories card opens a
    detail view: every repo in a two-line list, with state, signals and tasks
    for the one selected. Narrow terminals degrade in a fixed order instead of
    garbling.
  - **Browser:** a sidebar and one column, with four pages (Overview,
    Repositories, Tasks, Standup), saved views, and a drawer with a repo's full
    detail. Every action previews its targets first. On a read-only server the
    buttons stay and say how to enable them.
- **The browser no longer shows sample data unless asked.** A failed load is an
  error with a Retry button, an empty fleet says so, and the demo fleet appears
  only with `--demo`, labelled as demo data. The guided tour is gone.
- **`cohors scan`** calls its last group "clean" (repos with nothing to report),
  the same definition every other surface uses.
- **The terminal dashboard is made of panels.** On a colour terminal every card
  and every overlay is a filled surface instead of a box of lines, with its title
  on a band in its section's colour: purple for Do next, brick for Work at risk,
  sage for Open tasks, slate blue for Commits, teal for Repositories. The focused
  card has the one bright band. Overlays take the colour of the section they
  belong to, and the dashboard behind an open overlay fades, fills included.
  When the terminal is tall enough a blank row separates the header, the figures
  and each band of cards; smaller terminals keep every row for content.
  A panel is a shade off the terminal's real background, so it sits right on any
  colour scheme. cohors asks the terminal for that colour; a terminal that does
  not answer gets line frames in the same section colours instead, because a
  guessed shade is wrong on half of all terminals. The new `theme` setting
  (`"auto"`, `"dark"` or `"light"`, also `COHORS_THEME`) says it outright for a
  terminal that cannot be asked. Without colour (`NO_COLOR`), or with
  `COHORS_CARDS=lines`, cards keep their line frames.
- **Quieter rows.** A count of zero is a small dot rather than a `0`, and a branch
  level with its upstream says nothing rather than `level`, so the only numbers
  on a row are the ones that mean something. On a wide terminal the repository
  name and branch columns grow to show long names whole.
- **Keys: a letter is one verb on every card.** `f` fetches, `p` pulls, `P`
  pushes, `c` commits and `S` stashes wherever a repository is under the cursor:
  on Work at risk, in the repository table and in the detail view. On Do next the
  same letters carry out the fetch, pull or push step; a step that only shows you
  repositories (stashes to review, files to commit, a diverged branch) has no
  letter and is opened with Enter, so `S` and `c` no longer mean "review" there.
  A repository key pressed on Do next says where it works instead of doing
  nothing. `s` (standup), `t` (launchpad), `F` (fetch everything) and `Y` (copy
  the standup as markdown, formerly `y` on the Commits card) work from anywhere.
  A card lists its keys only while it has the focus.
- **Command line.** Tab completes commands, sort modes and repository names, and
  cycles through them; Up and Down recall earlier lines; the cursor moves with
  the arrow keys, Home and End, and Ctrl-A, Ctrl-E, Ctrl-U and Ctrl-W work as in
  a shell. A line that is not a command says what was wrong with it
  (`commit needs a message: :commit <message>`) instead of `unknown command`, and
  is marked as a failure rather than with a tick.
- **The detail view's list is one line per repository**, like the overview's
  table, with the last commit beside it when the terminal is wide enough. The
  path and commit id were repeated under every name; they are in the card about
  the selected repository.

### Fixed

- The browser dashboard listed its roots as full paths; it shows them as
  `~/code`, the way the terminal does.
- With no roots configured, cohors could treat `/` or the home directory as a
  root when started from there (an editor extension starts it in `/`), and list
  every tool's checkout on the disk as if it were your work. Neither is ever an
  implied root now, and the VS Code extension runs cohors from the open folder
  (only in a trusted workspace, so opening an untrusted folder never adds it to
  the fleet).
- The VS Code extension read each Claude Code transcript whole on every change,
  which froze the editor for seconds on a long session. It reads the two ends of
  a large transcript instead.
- The VS Code extension used the first `cohors` on the PATH, which on a machine
  with two installs (an older Homebrew or pip one ahead of a newer cargo one,
  both reporting the same version) was the one without `cohors overview`, so the
  dashboard could not read anything. It now uses the first one that can do the
  job, and when none can, the message names the file that is too old.
- A terminal that did not say what its background was used to be treated as
  light, which could put light fills on a dark terminal. Not knowing is now its
  own case, and nothing is painted that depends on the answer.
- On a terminal that did not answer the background question, the first key
  pressed after the dashboard opened was lost. The question is now asked without
  a second reader on the keyboard, and a late answer is still used.
- The Linux pip wheels were tagged `manylinux_2_31` but built against glibc
  2.35. They are now tagged `manylinux_2_35`, so pip on an older system declines
  the wheel instead of installing a binary that may not start.

### Security

- Text read from a repo's `.cohors/` files is labelled as untrusted data to
  the agent, and control characters are stripped before it is printed.
- Launchpad writes refuse symbolic links at every path they touch.
- A flow command a repo declares must be one readable line: printable ASCII
  plus letters and digits of left-to-right scripts, at most 400 characters.
  Control characters, invisible characters, line breaks and right-to-left text
  are refused, because they let a command display as one thing and run as
  another. Such a flow is rejected by the validator and never offered.
- A prompt handed to an agent runner can never be read as a command-line flag,
  and on PowerShell prompt text is never placed on a command line at all.
  Prompt files are private to the user.
- The editor extension runs nothing a repository supplies (a flow, a declared
  agent, a task) without Workspace Trust and a confirmation that shows exactly
  what will run, resolved first and re-checked just before it starts.
- Copyable commands in the browser quote repo paths, so a hostile folder name
  cannot inject a command when pasted.

## [0.6.0] — 2026-07-23

A readability and onboarding release. Every repo row now carries its per-branch
truth, every overlay shares one visual language, and every command surface
introduces itself instead of erroring at you.

### Added

- **Per-branch CI and sync status.** Multi-branch repos expand into a row per
  branch, each with its own checks, ahead/behind, and status. CI is keyed by ref
  rather than the checked-out worktree, so this is a real per-branch fetch and not
  an echo of the default branch. Capped at 10 branches per repo to stay inside the
  GitHub rate budget; single-branch repos are unaffected.
- **Open PR counts land on their base branch**, so a PR shows against the branch
  it actually targets.
- **`P` on a multi-branch repo asks which branch to push**, via a picker, instead
  of assuming the current one. Bulk push now targets only repos that actually have
  unpushed commits.
- **A first-run launch screen**, in the plain terminal before the dashboard: the
  super-fleet view lists every folder repos were detected under, and `--root` gets
  its own scoped view naming the folder it will walk. Enter launches, `q` quits.
  `--watch` skips both, so a live loop needs no keypress.
- **`cohors scan` prints a human report** at a terminal: repos grouped by urgency,
  the commands that clear each group, and how long the sweep took, with progress
  announced on stderr while the disk walk runs. Piped, redirected, or with
  `--json`, it emits the same raw JSON it always did.
- **`cohors mcp` run by hand explains itself** rather than sitting silent waiting
  for a client that will never speak: what the server is, the copy-pasteable
  registration line for Claude Code *and* the JSON `mcpServers` block for every
  other client (both with the absolute binary path, since agents spawn with a
  stripped `PATH`), and which tools each `--allow-*` flag unlocks.

### Changed

- **Every overlay shares one framed-table language.** The standup, detail pane,
  command palette, shell prompt, confirm dialog, open-with picker, and run output
  were rebuilt as two divider-joined columns with no floating inner boxes. The run
  output auto-collapses to its content when space is tight.
- **The action commands teach instead of erroring.** A missing `--select`, `-m`,
  or `<COMMAND>` now prints the verb's own guide: what it does, its safety
  posture, the selector vocabulary, and a `--dry-run` first step, and exits 0
  having touched nothing.
- **`cohors --help` carries the masthead**, a brand-matched clap palette, and a
  "start here" footer with the four shortest paths to seeing your own fleet.
  `cohors web` gained the same masthead and section grammar.
- **Readability pass on the dashboard:** every repo name is bold, branch rows are
  italic and marked with a hollow triangle, and the selection is a soft wash
  rather than a reverse-video block that swallowed dim cells.

### Fixed

- **GitHub enrichment applies refreshed data** even when a remote was already set,
  so CI and PR counts stop going stale behind a warm cache. Stale cache entries are
  now evicted on access as well as insert.
- **Open PRs are counted via the REST `pulls` endpoint** (5000/hr) instead of the
  search API (30/min), which was rate-limiting real fleets.
- **The MCP server handles a client disconnect gracefully** instead of erroring on
  a broken pipe, and `--allow-open` is actually wired to the `open` tool.
- **Content search runs in parallel** across repos.

## [0.5.14] — 2026-07-05

### Added

- **`cohors web --demo`** serves the built-in sample fleet in the browser (the web
  counterpart to `cohors demo`): zero setup, nothing read from disk, a safe way to
  preview the dashboard.

### Changed

- **Docs refresh:** the README is rewritten around the super-fleet model (plain
  `cohors` shows every detected repo; `--root` scopes), stripped of em-dashes, and
  gains crisp light-mode demo GIFs for both the terminal and browser dashboards.
- **CI runners:** the release jobs are pinned to available images (`ubuntu-latest`
  for housekeeping, `ubuntu-22.04` for the Linux build) since GitHub retired
  `ubuntu-20.04`.

### Changed

- **First-run `cohors` greets you and lists where repos were found before
  launching.** When run unconfigured, it prints the branded welcome, the folders
  it detected repos under, and a prompt — Enter launches the whole super fleet,
  `q` quits (or `cohors --root <dir>` to scope to one). Replaces the straight-to-
  dashboard jump from 0.5.12 so you can see what the super fleet comprises first.

### Changed

- **Plain `cohors` (and `cohors web`) now shows the whole super fleet by default.**
  With no config and no `--root`, the default roots are empty, so the Scanner
  auto-detects *every* folder that holds git repos and shows them all — instead of
  defaulting to a single `~/projects` and prompting to adopt. `cohors --root <dir>`
  still scopes to that one folder. Onboarding only greets + guides when nothing is
  found anywhere (a cheap `detect_roots` probe, so the common path adds no scan).
  Removed the `[u]`-adopt prompt — the super fleet is the default, nothing to adopt.

### Changed

- **First-run onboarding happens in the terminal, not inside the dashboard.** When
  bare `cohors` starts with no config and an empty default root, it now prints the
  branded welcome in the plain terminal (shared renderer with `cohors web`) and
  waits for a key: `u` adopts the detected roots (writes the config) and opens the
  dashboard; `q` quits. No more spinning up the full-screen TUI just to show an
  empty state. Non-interactive/piped invocations print the welcome as guidance and
  exit. The in-TUI empty screen remains only as a mid-session fallback.

### Fixed

- **Snapshot layouts no longer shift when the version string changes length**
  (e.g. `0.5.9` → `0.5.10`): the TUI renders a fixed version under `#[cfg(test)]`
  so header/help snapshots assert layout, not the build number.

### Changed

- **`cohors web`'s first-run setup hint is now branded and styled to match the
  dashboard.** It leads with the purple spider mark + "Welcome to cohors", shows
  the two ways in (`cohors web --root <detected>` to launch now, or `cohors` then
  `cohors web` to save a config), and closes with the default-root / super-fleet
  note. ANSI is gated on a real terminal — piped output and `NO_COLOR` stay plain.
  The spider mark is factored into a shared `SPIDER_MARK` const the TUI header and
  the CLI both draw from.

## [0.5.9] — 2026-07-05

### Changed

- **The first-run empty state shows both ways in as one "or", with a default-root
  note.** The welcome now presents `[u] use these  or  cohors --root <dir>`
  together, then explains the model: `~/projects` is the default root — pass
  `--root <dir>` to scope cohors to a single folder, or use the default to fold
  every folder it detects into one **super fleet**.

## [0.5.8] — 2026-07-05

### Changed

- **The first-run empty state opens with a welcome, not a negative.** Instead of
  leading with "No git repos under: …", the dashboard now greets a new user with
  a brand-purple **"Welcome to cohors"** and then either offers the detected roots
  (`Found repos under: … · [u] use these`) or shows a copy-pasteable sample
  command (`cohors --root ~/code`) to point cohors at their projects.

## [0.5.7] — 2026-07-05

### Changed

- **`cohors web` gates on setup instead of launching into an empty demo fleet.**
  When no repos are found under the configured/`--root` roots, it now prints the
  same two-step rescue the TUI offers — run `cohors` first (which detects and
  saves your roots), then `cohors web` again — and names the auto-detected roots
  when there are any. The check runs *before* the Trunk install + WASM build, so
  a first-time run isn't spent waiting only to be told to start over.

## [0.5.6] — 2026-07-05

### Changed

- **`cohors-web` now inherits the workspace version** (`version.workspace = true`)
  instead of pinning its own, so every crate moves in lockstep on each release.

## [0.5.5] — 2026-07-05

TUI design polish (no behaviour changes):

### Changed

- **Fleet-activity sparkline moved into the header's right side**, filling the
  space the info column left; the attention strip became a full-width severity
  bar over the four bucket counts.
- **The docked context pane is a grey two-column card** (local/remote facts |
  release + last commit) under a solid separator that joins the box borders.
- **The key hints degrade gracefully** — the consolidated one-box bar when it
  fits, otherwise a stacked layout that shows every hint at any width, using
  short labels and `␣`/`⇥` symbols so nothing truncates.
- **The repos table degrades across three width tiers** (Wide → Medium → Narrow)
  instead of one cliff, keeping a colour-coded **CI square** (green pass / red
  fail / amber pending / grey none) in its own column when there's no room for
  the word.
- **Marked repos show a brand-purple filled circle** in a checkbox-style gutter
  (an empty `○` when unmarked).
- **Each modal carries its own key hints inside its box** — below a solid
  separator joined into the borders (Command, Shell, Confirm, Open-with, Detail,
  Standup, Help) — so the root footer keeps the `select`/`act`/`view` keys
  instead of being taken over by the overlay's.
- **Every modal's title moved off the top border to a header row inside the box**,
  under the same solid `├─┤` separator as the hints — so titles and dividers read
  as full-strength structural lines matching the box borders, not dim text baked
  into the frame.

## [0.5.0] — 2026-07-05

The first release with prebuilt binaries (Homebrew, pip, and a checksummed shell
installer, all from one cargo-dist pipeline). Highlights of everything since
0.4.21, detailed in the sections below: the web dashboard grew a first-run guided
tour, always-live polling, a severity-bucketed attention breakdown, row tints,
and a detail panel whose empty state mirrors the filled one; the TUI reached
design parity with the web — a fleet-activity + attention overview, consolidated
table columns ending in Last + Status with per-repo activity sparklines, a
one-box hint bar, and a two-column dock card; repo groups, the MCP `changes`
tool, and per-repo commit-activity data landed in the core.

### Added

- **Config `[groups]` + a `group` selector — repo clusters as a fact, not a guess.**
  Define named groups of repo-name globs (e.g. `payments = ["pay-*"]`);
  cohors stamps each repo with every group it matches (by directory name) during
  the scan. The new `groups` field rides on every snapshot (so `list_repos` /
  `get_repo` show it), and `{ "group": "payments" }` is a first-class selector
  across the TUI target-set, CLI `--select group:payments`, and MCP — so "act on
  the whole payments cluster" is one argument. Backward-compatible
  (`#[serde(default)]`); ungrouped repos carry `[]`.

- **MCP `changes` tool — see *what's* uncommitted, not just that something is.**
  Returns each selected repo's changed-file list (path + git porcelain status)
  and, with `include_patch: true`, a size-capped unified diff of the working tree
  (`max_bytes`, default 20 KB, flagged `truncated` when cut). Clean repos are
  omitted and an empty selector defaults to every dirty repo, so an agent can
  summarize or review uncommitted work ("what's the work sitting in payments?")
  without cd-ing in and shelling out to `git status` / `git diff`. Backed by new
  `cohors_core::RepoChanges` (WASM-safe) and `cohors_git::repo_changes`.

- **Per-repo commit-activity data, walked in parallel (Phase 2, data layer).**
  Each snapshot now carries `activity`: weekly commit counts for the last 12 weeks
  (oldest first), the input for an activity sparkline. The walk runs inside the
  already-parallel scan (`rayon`), so the fleet-wide cost stays negligible — it
  walks newest-first by commit time and stops as soon as it leaves the window
  (an inactive repo costs ~one lookup), with a hard cap for huge histories.
  Measured: a full scan of an 18-repo fleet stayed at ~0.09s. The field is
  `#[serde(default)]`, so old caches and the web deserialize fine. (The sparkline
  *rendering* lands next.)

### Changed

- **Attention: flag never-pushed branches, and expose the *why* over MCP.** A named
  branch on a repo that has a remote but no upstream is now an `Unpublished` reason
  ("branch never pushed") at **Warn** severity — above mere local dirtiness
  (Notice) — and `rank` sorts it as unbacked work instead of "clean", so a feature
  branch that was never pushed (real data-loss risk) stops hiding at the bottom of
  the list. Every MCP repo `assessment` now also carries `reasons` (the concrete
  labels, most urgent first), so an agent reads "branch never pushed" directly
  instead of re-deriving it from the raw fields.

- **MCP: the remote half (CI / PRs) now works when an agent launches the server,**
  and its absence is announced instead of silent. `gh` is resolved against `$PATH`
  *and* common install locations (Homebrew on Apple Silicon/Intel, Linuxbrew,
  `~/.local/bin`), so a server spawned by Claude Code with a stripped `PATH` still
  finds the user's `gh auth token` — previously CI/PR data came back empty even for
  fully-authenticated users because `gh` wasn't on the spawn `PATH`. Every tool's
  `meta` now carries `github_token` (bool) plus an actionable `github_note` when a
  token is absent, so an agent can tell the user to run `gh auth login` rather than
  silently return fewer rows.

- **MCP: rewrote every tool description and added a server `instructions` field**
  so a connected agent reliably reaches for cohors instead of shelling out to
  `git`/`find`. Each of the 13 tool descriptions now leads with *when* to use it;
  the new `instructions` string (surfaced on `initialize`) tells the agent that any
  "my repos / fleet / what needs attention / find across repos" question maps to
  these tools. Also corrected the `list_repos` `fields` guidance (ahead/behind live
  under `upstream`, staged/modified/untracked under `worktree`) and spelled out the
  full selector vocabulary in the schema. Descriptions only — no tool behaviour or
  wire shape changed.

- **TUI: Repo and Branch fused into one `name @branch` column (Phase 1),** matching
  the web and reclaiming a whole column for the Status reason. The branch is a dim
  suffix (`@main`, magenta `@a1b2c3d` for detached HEAD, `unborn` for a fresh
  repo), ellipsized so a long branch can't blow out the column; the column
  auto-sizes to its widest content within a cap. Error rows now show the red name +
  the reason in Status (the redundant "error" marker is gone). Both the wide (dock)
  and compact layouts updated.

### Added

- **All status glyphs are now mode-aware (finishing Phase 0).** The remaining
  colour-only glyphs route through the icon resolver with ASCII fallbacks: the
  marked-selection gutter `●` becomes `*`, and the compact Sync cell's remote CI
  dot (whose *colour* meant pass/fail/pending) becomes a short letter `ok` / `x` /
  `~` under ASCII/`NO_COLOR` (omitted when there's no CI). The `?`-help legend
  reflects whichever tier is active. Default (Unicode) rendering is unchanged.
- **`icons` config + a glyph foundation built for *any* terminal (Phase 0).** A
  new `icons = "auto" | "ascii" | "unicode" | "nerd"` setting (default `auto`)
  routes every status glyph through one resolver: Unicode glyphs by default, an
  **ASCII fallback** used automatically under `NO_COLOR` (a colourless `●` can't
  say what it means, so a plain word reads better) or when forced, and a
  **Nerd-Font tier** that's only ever used when you explicitly opt in (never
  assumed). First migration: the *in-sync* indicator now renders a steady **"ok"**
  in the ASCII/`NO_COLOR` path instead of a colour-only, blinking `●` — so the
  state never depends solely on colour or on an animation. Groundwork for the
  broader compact-glyph plan; more cells migrate in later phases.

### Changed

- **TUI Sync "synced" state is now a blinking green `●`** instead of the word
  "synced". Since terminals ignore the ANSI blink attribute, it's animated in
  software off the render tick (~400ms on/off), so it actually blinks. The
  ahead/behind arrows, `local`, and the web's green cloud are unchanged.

- **The `act` box's live action-target (`→ <repo>`) moved beside the title.** The
  whole `act  → <repo>` unit is centered on the title row, instead of the target
  sitting on the box's bottom border (now a clean edge).
- **Fast local reinstall: `scripts/dev-install.sh`.** The release profile's LTO +
  `codegen-units = 1` make `cargo install` take ~1-2 min (great for the shipped
  binary, overkill for iterating). The script builds the **dev** profile
  (incremental, no LTO — ~2s) and drops it into `~/.cargo/bin`, so a
  rebuild+reinstall is near-instant (~28× faster). Use `cargo install` (release)
  only for the optimized/shipped artifact.

- **TUI footer group titles (`select` / `act` / `view`) moved inside their boxes.**
  They render as a **centered header row with a divider rule beneath it** at the
  top of each box instead of sitting on the border; the box top is now a clean
  rounded edge. The `act` box's live action-target (`→ <repo>`) still sits on its
  bottom edge.

- **The TUI's empty states now read as real values, not bare `·`/`—`.** Each
  column shows a meaningful state in its natural form: count columns are numeric
  (Changes / Stash / PRs show a dim **0** when empty), and relationship columns
  are text (**synced** when even with upstream, **local** when there's no
  upstream/remote, **no CI** where a remote has no checks, **up to date** in the
  Status column when nothing needs you). Error rows leave the data cells blank
  (the reason carries the row). Real signals (counts, `passing`/`failing`, ↑/↓
  arrows) still pop; the clear states recede.

- **Leaner builds and smaller artifacts (optimization pass).** Tuned the build
  profiles: release now uses `lto = true`, `codegen-units = 1`, and `strip = true`
  (keeping unwinding panics so the TUI's terminal-restore hook still works), which
  cut the installed binary from **9.7 MB to 6.6 MB (-32%)**. The web crate's
  release profile (`opt-level = "s"` + LTO) plus a `trunk --release` (wasm-opt)
  build takes the shipped wasm bundle from **4.0 MB to 0.79 MB (-80%)**. Dev
  profiles drop full debuginfo (`line-tables-only` for the workspace, off for the
  WASM crate, which can't be gdb'd anyway) — this is the main source of
  `target/`-dir bloat, so day-to-day rebuilds stay much smaller. Also dropped
  three unused dependencies (`serde_json`/`thiserror` from cohors-github, the
  leftover `ureq` from cohors-tui after the GitHub proxy was removed).

### Fixed

- **Web: fixed a tall empty gap in the detail header (and any cell with a
  word-state).** The faint default states (`no CI`, `local`, `none`, …) reused the
  CSS class `state`, which also styles the full-page async-loading message with
  ~2.5rem padding — so e.g. a "no CI" badge inflated the header. Renamed the
  default-state class to `muted` to remove the collision.
- **CI status now comes only from the GitHub Checks (Actions) API.** The legacy
  commit Status API is no longer consulted — it reported a phantom "pending" for
  repos that post no commit statuses (and stale/broken signals generally), and
  added a second request + failure mode per repo. `combine_ci` folds just the
  check-runs (failing > running > passing; none ⇒ "no CI"). The TUI/CLI/MCP and
  the web all enrich through this path, so all of them get the corrected signal
  (ADR-043, narrowing ADR-040). PR counts already use the modern search/`pulls`
  APIs — no legacy surface involved.

### Added

- **The web detail aside surfaces sync + CI inline in its header.** Next to
  `name · branch`, the drill-in header now shows the at-a-glance sync (cloud) and
  CI status as badges — a running build animates the braille dot-spinner, same as
  the table — so you see health without scanning the facts list below. The facts
  list drops the now-redundant Sync/CI rows and gives the remaining facts
  (**Changes · Stash · PRs · Last**) a leading icon each (pencil / box /
  pull-request / clock).
- **`cohors web` is the local fleet in the browser — the same tool as the TUI
  (v0.5 slice 2).** `cohors web --root ~/code` scans **that folder's repos** and
  shows the exact same view the TUI does: local worktree status, ahead/behind,
  stash, and *why each repo needs you* (the `assess` attention reasons), enriched
  with remote CI/PRs. The native server does the scan and serves `cohors-core`
  snapshots as JSON (`/api/repos`, `/api/detail`, `/api/meta`); the browser
  deserializes the same models and renders them through the same
  `compute_view`/`assess`/`SortMode` logic — so the web and the TUI agree by
  construction (ADR-042). The GitHub token stays on the server (it enriches
  there) and never reaches the page. With nothing to scan, it falls back to the
  **demo fleet**.
- **The web fleet table mirrors the TUI's columns** — Repo · Branch · Sync
  (↑ahead ↓behind) · Changes (staged/modified/untracked) · Stash · PRs · CI ·
  Last · **Status** (the primary attention reason, severity-coloured). Columns
  are kept **compact** so the whole row (Status included) fits without sideways
  scroll: the data columns are narrow, long branch names truncate (full name on
  hover), and **Last shows just the commit age** (its subject is in the detail).
  While the remote pass is still running, the PRs/CI cells of GitHub repos show a
  **braille dot-spinner** instead of a dash. The attention summary strip
  (`fleet_summary`), the filter, the **needs-attention** toggle, and the
  **attention / recent / name / sync** sorts all run `cohors-core` verbatim.
- **The web drill-in mirrors the TUI's `Enter` pane.** Selecting a repo fetches
  its detail on demand: every reason it wants attention, the local facts (Sync ·
  Changes · Stash · CI · PRs · Last), **recent commits**, the **working-tree**
  changes, and — when it's a GitHub remote — **open PRs**, **top contributors**,
  open issues, and the latest release, plus the remote source link. Each section
  is best-effort; a dots spinner shows while it loads.
- **`cohors web --watch` keeps the page live.** With `--watch`, the dashboard
  polls for a fresh scan so edits, commits, and pulls show up without a manual
  rescan (re-scanning is cheap; remote stays server-cached). The header shows
  which folder is being scanned.

### Changed

- **The web fleet table is now icon-forward and tighter.** The terminal's cryptic
  `·`/`—` are gone, replaced by a small, color-coded SVG icon set (GitHub-style,
  not emoji): a green **cloud** for in-sync / **cloud-slash** for a local-only
  branch (or `↑↓` arrows when diverged); a **pencil + count** for uncommitted
  changes; a **box + count** for stashes; a **pull-request icon + count** for open
  PRs; and a per-state **Status** icon (pencil / arrows / box / branch / alert).
  CI stays as **text** ("passing" / "failing", a braille **dot-spinner** +
  "pending" for a running build). The **Repo and Branch columns are fused** into
  one `name @branch` cell, and long names/branches truncate with the full text on
  hover — so every row fits on one line without sideways scroll. Every glyph has a
  tooltip. Columns hug their content (a trailing spacer soaks up the leftover
  width), so there's no pooled whitespace in the Status column.
- **The web app has a typeface now** — **Google Sans** (UI) and **Google Sans
  Code** (the data table / monospace), with **IBM Plex Sans** / **IBM Plex Mono**
  as the next fallback. They're loaded from the **Google Fonts CDN** (a `<link>`
  in `index.html`), so distribution stays a single static bundle with no font
  files to ship; the family stacks fall back to system fonts when offline.
- **`cohors web` now takes `--root` (and the other global flags).** It builds its
  view from the same `Scanner` the TUI/CLI/MCP use, so `--root`, `--config`, and
  `--watch` all apply. It still builds the web app from source (run it from the
  cohors checkout); a hosted, GitHub-account version is a later slice.
- **`cohors-core`'s `RepoDetail`/`ChangedFile` are now (de)serializable**, so the
  local drill-in can ship from the server to the browser unchanged.

## [0.4.21] — 2026-06-16

### Changed

- **`cohors web` is distribution-aware about Trunk.** Trunk is a *developer*
  dependency (it builds the WASM and runs the dev server), not something every
  install must ship: `cohors web` auto-installs it via Cargo when missing, and if
  Cargo isn't present (a prebuilt-binary, non-Rust install) it points at
  `brew install trunk` instead of failing obscurely. End users won't need Trunk
  at all — once the dashboard is deployed (v0.5 slice 4), an installed
  `cohors web` outside a checkout will open the hosted URL (no local build).

## [0.4.20] — 2026-06-16

### Added

- **`cohors-web` — the web front-end begins (v0.5, slice 1).** A new WASM crate
  renders the fleet in the browser with [Leptos](https://leptos.dev), reusing
  `cohors-core` *verbatim* — the same `resolve`/sort, `assess`/attention, and
  relative-time logic the TUI runs (ADR-002 / ADR-041). Slice 1 shows the
  built-in demo fleet (the browser analog of `cohors demo`); GitHub data, OAuth,
  and deployment follow in later slices. The crate is excluded from the native
  workspace and built only for `wasm32-unknown-unknown`.
- **`cohors web` — one command to launch the dashboard.** It finds the
  `cohors-web` crate, makes sure Trunk (the WASM bundler) is installed (installing
  it for you the first time, unless `--no-install`), starts the dev server, waits
  until it's actually listening, then opens a clean **branded local URL** —
  `http://cohors.localhost:8080` (the reserved-loopback `*.localhost` TLD, so no
  `/etc/hosts` edit and no privileges). `--port`, `--no-open`, `--no-install`.
  Run it from inside the repository (Trunk builds the app from source).

## [0.4.19] — 2026-06-16

### Changed

- **Help overlay collapses to its content** (capped at 90% of the screen) instead
  of a fixed tall box with trailing whitespace.

### Fixed

- **Help legend matches the actual row colours.** It claimed a *red* name meant
  "needs attention"; red is now reserved for unreadable (`error`) repos. The
  legend now reads: dim name = clean, bold name = needs attention, red name =
  unreadable.

## [0.4.18] — 2026-06-16

### Changed

- **The full detail view (`Enter`) collapses to its content and the commits pane
  gets a scrollbar.** It was a fixed 84%-height modal, so a repo with little to
  show left acres of whitespace, and the commits pane truncated with a "↓ N more"
  instead of scrolling. The modal now sizes to the taller pane's content (capped
  at 84% of the screen), and both panes scroll — each with a scrollbar that
  appears only when it overflows — sharing one scroll position, each clamped to
  its own length.

## [0.4.17] — 2026-06-16

### Changed

- **The context pane now lives inside the Repositories box** instead of being a
  separate box below it. The panel reserves its bottom rows for a titled divider
  rule (repo name·branch on the left, `Enter: full detail` on the right) and the
  selected repo's facts, with the table laid out above — one border around both.
  On short terminals it doesn't appear and the table uses the whole box.

## [0.4.16] — 2026-06-16

### Fixed

- **CI status is now accurate for GitHub Actions repos (ADR-040).** It was read
  only from the legacy commit Status API, which returns "pending" for repos that
  post no commit statuses — i.e. every Actions-only repo, forever (and the wrong
  value was cached across restarts). CI is now derived from the **Checks API**
  (where Actions report) combined with commit statuses: failing wins, then
  pending, then passing; with no checks *and* no statuses it's "no CI" — a real,
  distinct state, not a false pending. It self-corrects on the next scan/`--watch`
  refresh.

### Changed

- **Context pane: collapses to content, purple labels, top padding (ADR-039).**
  The pane no longer reserves a fixed height with trailing blank rows — it sizes
  to its content. Field labels (`Changes`, `Stash`, …) are now brand-purple, and
  there's a blank padding row above the first fact.

## [0.4.15] — 2026-06-16

### Changed

- **The context pane now word-wraps the last commit message** across the lines
  left in the pane (continuation lines indented under the value column) instead
  of truncating it with an ellipsis. If it's still longer than the pane, the last
  visible line ends with `…`. The pane is one row taller to give the message room.

## [0.4.14] — 2026-06-16

### Fixed

- **The context pane now stays on your repo across a `--watch` re-scan.** The
  cursor was anchored to a row *index*, so when a re-scan re-sorted the fleet
  (e.g. a repo you pushed went from "ahead" to clean and dropped down the
  dirty-first list) the cursor — and the detail dock — ended up pointing at a
  different repo, making the pane look like it hadn't updated. The cursor is now
  keyed to the selected repo's id and restored to its new position after every
  re-scan, so the pane keeps following the repo you were on (matching how the
  marked selection and busy set already survive re-sorts).

## [0.4.13] — 2026-06-16

### Changed

- **Redesigned the context pane as a labeled form (ADR-039 follow-up).** The dock
  was an unreadable mix of a bulleted reason list and a cryptic `changes 3 s1
  sync ●` line that restated it. It's now a small, scannable form — one dim,
  fixed-width label per fact, everything spelled out (no `s1`/`●`), color for
  urgency:

  ```
  Changes      3 modified · 1 untracked
  Stash        1 · stale
  Upstream     2 ahead  (origin/main)
  Remote       CI passing  ·  2 open PRs
  Last commit  2h ago — fix: retry on 5xx
  ```

  Changes now shows the staged/modified/untracked breakdown (which the table's
  count hides), and the full commit message lives here (the row no longer shows
  it). No more duplication between the pane and the row.

## [0.4.12] — 2026-06-16

### Changed

- **Expanded, edge-to-edge table columns when the dock is up (ADR-039
  follow-up).** The reclaimed width now goes to legible, separate columns instead
  of compact fused glyphs: the old `Sync` (`↑2 ● 2pr`) splits into **Sync**
  (ahead/behind), **PRs** (open count), and **CI** (spelled out:
  `passing`/`failing`/`pending`); the old `Changes` (`4 s1`) splits into
  **Changes** and **Stash**. The reason column is renamed **Status**, and the
  table fills the full width. Short terminals keep the compact layout.

## [0.4.11] — 2026-06-16

### Changed

- **The repository table stops repeating the commit message when the dock shows
  it (ADR-039 follow-up).** With the dock up, the trailing column splits into
  **Last** (commit age) and **Why** — the repo's primary attention reason,
  colored by severity (`↑2 unpushed`, `↓5 behind — pull`, …) — so the
  dirty-first ordering is now self-explaining on every row, not just the selected
  one. On short terminals (no dock) the full "Last commit" column is kept, so no
  information is lost.

## [0.4.10] — 2026-06-15

### Added

- **Docked context pane — the dashboard is now a cockpit (ADR-039).** On a tall
  terminal, a pane appears below the list and follows the cursor: idle, it shows
  the selected repo at a glance — why it wants attention (the reasons that drive
  the sort), its changes/sync, and the last commit; during a bulk action, it
  shows the in-flight repos with spinners. It reads only the data already
  scanned (no extra I/O); the full PRs/commits/contributors view still lives
  behind `Enter`. On short terminals it simply doesn't appear and the list takes
  the whole area.

### Changed

- **Commit messages ellipsize** (`…`) to the column width instead of being
  hard-cut at the frame edge.
- **A repo's name is no longer coloured red just for wanting attention** — red
  is reserved for genuinely broken (`error`) repos. This also removes the red
  block that appeared when an aging-unpushed repo was the selected (highlighted)
  row.

## [0.4.9] — 2026-06-15

### Added

- **`commit` and `push` actions across every surface (ADR-038).** A new
  `commit` action stages tracked + untracked changes (`git add -A`) and commits
  them with a message ("nothing to commit" is a no-op; never amends). It joins
  `push` so the full **edit → commit → push** loop is available to your agent
  and your scripts, not just the TUI.
  - **MCP:** new `push` tool (needs `--allow-writes`) and `commit` tool (needs
    `--allow-writes` + `confirm:true` + a `message`), both with `dry_run`
    previews. The agent can now finish a cross-repo change end-to-end.
  - **CLI:** actions are now first-class subcommands —
    `cohors fetch|pull|push|commit|stash|run` — each taking the same `--select`
    selector language as `scan --select`, plus `--dry-run`. `commit` takes
    `--message`; `run` takes a command and `--timeout`. A selector is required,
    so an action never silently hits the whole fleet (pass `--select all`).

### Safety

- The CLI has no capability flags by design — the human running the command is
  the consent, like `git push` itself. The agent-facing MCP server keeps the
  full ADR-025 tiering. On every surface the shared action layer holds the same
  guarantees: `pull` is fast-forward-only, `push` never force-pushes, and
  `commit`/`stash` cannot lose work.

## [0.4.8] — 2026-06-15

### Added

- **Open-issue count + latest release in detail (ADR-037).** The detail pane's
  left column now leads its GitHub section with a one-line summary —
  `N open issues  ·  latest vX` — fetched alongside the PRs and contributors.

### Changed

- **MCP `get_repo` returns the same remote detail as the TUI (ADR-037).**
  Inspecting one repo is now consistent across surfaces: `get_repo` carries a
  `remote_detail` field (open PRs, contributors, open-issue count, latest
  release) via the same `cohors_github::fetch_repo_detail` the `Enter` pane uses,
  when the repo has a remote and a token. The fleet-wide `list_prs` rollup is
  unchanged — it answers a different (cross-repo triage) question. The control
  surfaces remain CLI, TUI, and MCP, all sharing one `cohors-core` model and one
  fetch path.

## [0.4.7] — 2026-06-15

### Added

- **Richer two-pane detail pane (ADR-036).** Pressing `Enter` on a repo now shows
  a standup-style split: the left column has changes, branches, stashes, **open
  pull requests, and top contributors**; the right column has recent commits. PRs
  and contributors are fetched from GitHub on demand (when the repo has a remote
  and a token), shown with a `loading…` / `needs a GitHub remote + token` state
  so an empty section never reads as broken. New `cohors-core` models
  (`PullRequest`, `Contributor`, `RemoteDetail`) + `cohors_github::fetch_repo_detail`.

## [0.4.6] — 2026-06-15

### Added

- **Interactive first-run rescue (ADR-035).** When the dashboard finds no repos
  under your configured roots, the empty state now detects repos elsewhere and
  offers `[u] use these · [q] quit` — pressing `u` writes those roots to your
  config and rescans live, no restart. It's a confirmed `init`: detect → show →
  write only on a keypress, never silently. (`cohors scan` keeps its clean
  stdout + stderr hint; the picker lives only in the interactive launch.)

## [0.4.5] — 2026-06-15

### Changed

- **`cohors scan` no longer goes silent on an empty fleet.** stdout stays a clean
  `[]` (the scriptable JSON contract is untouched), but a one-line hint now goes
  to **stderr** — "no git repositories found under …; run `cohors init` or pass
  --root" — so a human isn't left guessing while pipelines ignore it.
- The TUI empty state notes that `cohors init` auto-detects your repos.

## [0.4.4] — 2026-06-15

### Changed

- **`run` fans out concurrently (ADR-034).** The MCP `run` tool now executes
  across its target repos over a bounded thread pool (8), instead of one at a
  time — a fleet-wide command finishes in about the slowest repo's time, not the
  sum. Results stay in target order; per-repo timeouts are unchanged. This closes
  the v0.4 MCP hardening backlog.

## [0.4.3] — 2026-06-15

### Added

- **MCP action guardrails (ADR-033), via a new `[mcp]` config table.**
  - `max_action_targets` (default 50): an action whose selector resolves to more
    repos than the cap is refused unless the selector is an explicit
    `{all: true}` — a fumbled broad selector can't fan out. `0` disables it.
  - `run_allowlist` (default empty = any): restricts `run` to commands matching
    `*`-glob patterns (e.g. `"cargo *"`, `"git *"`).
  - **Audit log**: every executed action writes `{tool, selector, resolved_ids,
    ok/failed}` to `cohors.log`.

### Fixed

- CI: bumped `actions/checkout` to v5 (v4's Node 20 runtime is deprecated).

## [0.4.2] — 2026-06-15

### Changed

- **Auto-detected discovery, consistent across CLI/TUI/MCP (ADR-032).** With no
  configured roots, cohors probes common code directories (`~/code`,
  `~/projects`, `~/dev`, `~/src`, `~/work`, `~/Developer`, …) and the working
  directory (using its parent when you're inside a repo) to find where your repos
  live — so a zero-config run shows a populated fleet everywhere, not an empty
  one. Resolution: `--root` → config `roots` → auto-detected → current directory.
- **`cohors init` seeds real roots.** It now writes the detected roots into the
  config (and reports them) instead of the blind `~/projects` placeholder.

## [0.4.1] — 2026-06-15

### Changed

- **Remote tools now report what they dropped.** `list_prs` / `ci_status` add
  `meta.excluded` (and a note) for selected repos that have no GitHub remote, so
  an agent can reconcile "18 repos but 14 CI rows" instead of guessing.
- **Fixed stale tool descriptions.** `fetch`/`pull`/`stash`/`run` now correctly
  state that `dry_run` previews need neither the tier flag nor `confirm`, and
  `run` documents the enforced `timeout_secs` (no longer claims it's unenforced).

## [0.4.0] — 2026-06-15

**Milestone: the agent surface.** `cohors mcp` exposes the fleet to a coding
agent over the Model Context Protocol — the same core the dashboard uses. This
release marks the v0.4 milestone, delivered incrementally across 0.3.58–0.3.63:

- **Selectors** — a serializable fleet predicate with a pure `resolve()` shared
  by the CLI (`scan --select`), the TUI, and MCP (ADR-024).
- **`cohors mcp` stdio server** — hand-rolled sync JSON-RPC, no new dependency
  (ADR-028). Read tools `list_repos`, `get_repo`, `fleet_summary`, `repo_path`,
  `search`, `list_prs`, `ci_status`; results in the `cohors scan` shape plus a
  per-repo `assessment` and a top-level `fleet`.
- **Cross-fleet `search`** — content (ripgrep → git grep → in-process fallback)
  and metadata (path/name/branch), selector-scoped.
- **Action tools** `fetch`, `pull` (ff-only), `stash`, `run` behind the ADR-025
  safety tiers — read-only by default, `--allow-writes`/`--allow-run`, per-call
  `confirm`, side-effect-free `dry_run`, and a `run` timeout.
- **Fail-loud reads** — every read carries `roots`/`config_path`/`total`/
  `errored` and a `note`, and discovery falls back to the current directory, so
  an empty fleet explains itself instead of looking "all clear" (ADR-029).

No code changes since 0.3.63 — this is the version marker for the completed
milestone.

## [0.3.63] — 2026-06-15

### Added

- **Remote MCP read tools `list_prs` and `ci_status`** (ADR-031), GitHub-enriched
  on demand. With no token they return an empty set plus a `meta.note` saying so.
  The read surface is now complete: `list_repos`, `get_repo`, `fleet_summary`,
  `search`, `repo_path`, `list_prs`, `ci_status`.

### Changed

- **`dry_run` is now side-effect-free in the truest sense** — it previews the
  target set *before* any tier/`confirm` gate, so an agent can preview on a
  read-only server and a human can approve before enabling writes/run. (Fixes the
  `dry_run`-required-`confirm` contradiction.)
- **`run` enforces a per-repo timeout** (`timeout_secs`, default 120s): a hung
  command is killed and reported as `timed_out` instead of stalling the fan-out.

## [0.3.62] — 2026-06-15

### Added

- **MCP action tools (ADR-030): `fetch`, `pull` (ff-only), `stash`, `run`.** The
  agent surface is now a control plane, not just a reader. Safety per ADR-025:
  read-only by default; `fetch`/`pull` need `--allow-writes`; `stash` adds
  `confirm: true`; `run` (arbitrary shell, per-repo `{exit_code, stdout, stderr,
  truncated}` + a monotonic `run_id`) needs `--allow-run` + `confirm: true`.
  Every action takes a required selector (empty matches nothing — never the whole
  fleet), excludes error/path-less repos, and supports `dry_run` to preview the
  exact target set with no side effects. Permission errors state the fix (e.g.
  "relaunch with `cohors mcp --allow-writes`").

### Deferred

- For `run`: bounded-pool parallelism and `timeout_secs` enforcement (currently
  sequential; the arg is accepted but not yet enforced), plus the config-driven
  `max_action_targets` cap, `run_allowlist`, and audit log.

## [0.3.61] — 2026-06-15

### Changed

- **Zero-config discovery (ADR-029).** With no `roots` configured, cohors now
  scans the current directory (like ripgrep/lazygit) instead of showing an empty
  fleet. Resolution order: `--root` → config `roots` → current directory.

### Added

- **Fail-loud MCP reads.** `list_repos`, `fleet_summary`, and `search` now attach
  a `meta` block — `roots`, `config_path`, `total`, `errored`, and a plain
  `note` when the result is empty or partial — so an agent never mistakes a
  misconfigured root (`total: 0`) for "all clear."

## [0.3.60] — 2026-06-15

### Added

- **Cross-fleet `search`, exposed as an MCP tool.** `kind=content` greps file
  contents across the selected repos (ripgrep → `git grep` → an in-process
  `ignore`-walk fallback, fixed-string match, so there's no hard external
  dependency); `kind=path`/`name`/`branch` match snapshot metadata. Results are
  selector-scoped and capped (`max_results`), with a `truncated` flag. This is
  the agent's entry point for cross-repo refactors.
- Metadata search (`search_metadata`, path/name/branch) lives in the pure,
  WASM-safe `cohors-core`; content search (`search_content`) lives in the
  `cohors-git` adapter where the file I/O belongs.

## [0.3.59] — 2026-06-15

### Added

- **`cohors mcp` — a Model Context Protocol server over stdio** (ADR-028), so a
  coding agent can see your fleet with the same logic the dashboard uses.
  Read-only in this slice, with the core read tools: `list_repos` (selector /
  sort / fields / limit), `get_repo`, `fleet_summary`, and `repo_path`. Each repo
  is returned in the `cohors scan` shape plus a per-repo `assessment` (severity +
  needs-attention) and a top-level `fleet` summary. Register it with
  `claude mcp add cohors -- cohors mcp`. The `--allow-writes` / `--allow-run` /
  `--allow-open` flags are accepted for the forthcoming action tools.

### Note

- The MCP transport is a hand-rolled synchronous JSON-RPC loop (no `tokio`,
  no new dependency), keeping the binary on its sync architecture; `rmcp`
  remains an isolated future swap (ADR-028 revises ADR-023).

## [0.3.58] — 2026-06-15

### Added

- **`Selector` + pure `resolve()` in `cohors-core` (ADR-024).** A serializable
  predicate over the fleet — identity/scope, local state, remote, and `any_of`/
  `not` combinators — that resolves to an ordered `Vec<RepoId>`. The empty
  selector resolves to nothing (never "all"), so an action can't target the whole
  fleet by omission. Pure and WASM-safe.
- **`cohors scan --select <query>`.** Filter the JSON to a subset using the same
  resolver, via JSON (`'{"behind":true}'`) or shorthand (`dirty`, `behind`,
  `ahead`, `attention`, `clean`, `name:pay*`, `ci:failing`, `prs:1`, comma-separated
  to AND them). `cohors scan --select behind` and a future MCP `list_repos` share
  one resolver, so they return identical sets.

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

