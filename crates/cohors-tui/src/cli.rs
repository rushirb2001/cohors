//! Command-line interface (clap).

use clap::builder::styling::{AnsiColor, Color, RgbColor, Style, Styles};
use clap::{Parser, Subcommand};

/// clap's own help/error palette, matched to the TUI: brand purple for headings,
/// bold for anything you literally type, dim for placeholders. clap turns all of
/// this off by itself when the output isn't a terminal, so piped help stays plain.
const HELP_STYLES: Styles = Styles::styled()
    .header(
        Style::new()
            .bold()
            .fg_color(Some(Color::Rgb(RgbColor(181, 92, 164)))),
    )
    .usage(
        Style::new()
            .bold()
            .fg_color(Some(Color::Rgb(RgbColor(181, 92, 164)))),
    )
    .literal(Style::new().bold())
    .placeholder(Style::new().dimmed())
    .error(
        Style::new()
            .bold()
            .fg_color(Some(Color::Ansi(AnsiColor::Red))),
    )
    .invalid(
        Style::new()
            .bold()
            .fg_color(Some(Color::Ansi(AnsiColor::Yellow))),
    )
    .valid(Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green))));

/// The top-level help layout. Same as clap's default minus `{about-with-newline}`:
/// the masthead `main` attaches already carries the tagline, so the plain about
/// line would just repeat it.
pub const HELP_TEMPLATE: &str = "\
{before-help}{usage-heading} {usage}

{all-args}{after-help}";

/// A fast dashboard for all your git repos.
#[derive(Debug, Parser)]
#[command(name = "cohors", version, about, long_about = None, styles = HELP_STYLES)]
pub struct Cli {
    /// Override the config file location.
    #[arg(
        long,
        value_name = "PATH",
        global = true,
        help_heading = "Global options"
    )]
    pub config: Option<String>,

    /// Override the configured roots; repeatable (e.g. `--root ~/work --root ~/oss`).
    #[arg(
        long,
        value_name = "DIR",
        global = true,
        help_heading = "Global options"
    )]
    pub root: Vec<String>,

    /// Force a fresh scan, ignoring the cache.
    #[arg(long, global = true, help_heading = "Global options")]
    pub no_cache: bool,

    /// Keep the dashboard live: re-scan every few seconds (skips the launch screen).
    #[arg(long, global = true, help_heading = "Global options")]
    pub watch: bool,

    /// Print the report a person reads even when the output is piped, in plain
    /// text. A pipe gets JSON otherwise; a coding agent's shell wants this.
    #[arg(long, global = true, help_heading = "Global options")]
    pub report: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Write a starter config file and print its path.
    #[command(
        after_help = "Examples:\n  cohors init                       detect your repos and write a config\n  cohors init --root ~/code         seed it with one folder\n  cohors init --force               start over (the old file is kept as .bak)"
    )]
    #[command(display_order = 23)]
    Init {
        /// Overwrite an existing config file.
        #[arg(long)]
        force: bool,
    },
    /// List every repo cohors finds, without reading them (no TUI).
    ///
    /// Only walks the folders, so it is the quick way to check what the
    /// dashboard will show. At a terminal it lists the repos under each root
    /// and marks those nested inside another repo; piped it prints one path per
    /// line (for `xargs` and friends); `--json` prints them as JSON.
    #[command(
        after_help = "Examples:\n  cohors discover                   the repos cohors will show\n  cohors discover --deep            look inside every repo, at any depth\n  cohors discover | head            one path per line, for scripts"
    )]
    #[command(display_order = 10)]
    Discover {
        /// Search every folder at any depth, inside repos too. Repos only this
        /// finds are remembered, so the dashboard shows them from now on.
        #[arg(long)]
        deep: bool,
        /// Print JSON instead of the human list.
        #[arg(long)]
        json: bool,
    },
    /// Discover and snapshot every repo, then report what needs you (no TUI).
    ///
    /// At a terminal this prints a grouped report: what needs you now, what is
    /// minor, what is calm, and the commands that clear it. Piped or redirected
    /// (or with `--json`) it prints the raw JSON snapshots instead.
    #[command(
        after_help = "Examples:\n  cohors scan                       what needs you, at a glance\n  cohors scan -s dirty              only repos with uncommitted work\n  cohors scan --json | jq '.[].name'   the raw snapshots"
    )]
    #[command(display_order = 3)]
    Scan {
        /// Search every folder at any depth, inside repos too. Repos only this
        /// finds are remembered, so the dashboard shows them from now on.
        #[arg(long)]
        deep: bool,
        /// Only include repos matching a selector: JSON (`'{"behind":true}'`)
        /// or shorthand (`dirty`, `behind`, `ahead`, `attention`, `name:pay*`,
        /// `ci:failing`, comma-separated to AND them). Results print in
        /// dirty-first order. Omit to print the whole fleet.
        #[arg(short, long, value_name = "QUERY")]
        select: Option<String>,
        /// Print raw JSON instead of the human report. JSON is already the
        /// default whenever stdout is piped, so existing scripts keep working.
        #[arg(long)]
        json: bool,
    },
    /// What to do next across every repo, and why (no TUI).
    ///
    /// The briefing the dashboard opens with, as a report: the actions worth
    /// taking (most urgent first), the key figures, and the repos where local
    /// work could be lost. Piped or with `--json` it prints the full overview
    /// model as JSON — the same one the web dashboard, the editor extension
    /// and the MCP `overview` tool use.
    #[command(visible_alias = "status")]
    #[command(
        after_help = "Examples:\n  cohors status                     the briefing (same as overview)\n  cohors overview --json            the overview model the web and editor use"
    )]
    #[command(display_order = 1)]
    Overview {
        /// Print JSON instead of the human report.
        #[arg(long)]
        json: bool,
        /// Only these repos (the selector language of `scan --select`).
        #[arg(short, long, value_name = "QUERY")]
        select: Option<String>,
    },
    /// Your commits across every repo, as a paste-ready digest (no TUI).
    ///
    /// Lists the commits you authored (matched by `git config user.email`) in
    /// the last seven days, grouped by repo with the most active first, as
    /// markdown you can paste into a standup channel.
    #[command(
        after_help = "Examples:\n  cohors standup                    your commits this week, as markdown\n  cohors standup --today | pbcopy   today's, to paste"
    )]
    #[command(display_order = 9)]
    Standup {
        /// Only today's commits instead of the last seven days.
        #[arg(long, conflicts_with = "since")]
        today: bool,
        /// From when: `yesterday`, `3d` (days back) or a date (`2026-11-01`).
        #[arg(long, value_name = "WHEN")]
        since: Option<String>,
        /// Only these repos (the selector language of `scan --select`).
        #[arg(short, long, value_name = "QUERY")]
        select: Option<String>,
        /// Print the commits grouped by repo as JSON instead of markdown.
        #[arg(long)]
        json: bool,
    },
    /// Launch the dashboard on a built-in sample fleet: no config, no scanning.
    ///
    /// A zero-setup way to try cohors: every column and view is populated with
    /// privacy-safe demo data. Actions are stubbed (nothing touches your disk).
    #[command(display_order = 20)]
    Demo,
    /// Update cohors to the newest release, the way this copy was installed.
    ///
    /// Asks the place this copy came from (GitHub releases, the Homebrew tap,
    /// PyPI, npm or crates.io) for the newest release. cohors' own install
    /// (`cohors install`) downloads, verifies and switches to it; Homebrew,
    /// pip and the shell installer install it their way; npm, uv, cargo and
    /// source builds are told the command. The dashboard does this by itself
    /// on every launch and every few minutes (the `updates` setting).
    #[command(
        after_help = "Examples:\n  cohors update                     install the newest release the way this one was installed\n  cohors update --check             only say whether there is one"
    )]
    #[command(display_order = 24)]
    Update {
        /// Only say whether a newer release exists; install nothing.
        #[arg(long)]
        check: bool,
        /// With --check: print the answer as JSON.
        #[arg(long, requires = "check")]
        json: bool,
    },
    /// Set up cohors' own self-updating copy, or reinstall it.
    ///
    /// Downloads the release for this platform from GitHub, checks it against
    /// the release's SHA-256, keeps it in ~/.local/share/cohors/versions/ and
    /// links ~/.local/bin/cohors to it. From then on it updates itself, like
    /// Claude Code's native install. A copy installed by Homebrew, pip, npm or
    /// cargo sets this up by itself on its first run and hands control to it
    /// whenever it is newer. Says whether typing `cohors` runs it.
    #[command(
        after_help = "Examples:\n  cohors install                    the newest release\n  cohors install 0.7.8              a given release"
    )]
    #[command(display_order = 26)]
    Install {
        /// A release version (0.7.8), or `latest` (the default).
        version: Option<String>,
    },
    /// Say how cohors is installed and whether it keeps itself up to date.
    ///
    /// Read-only: which cohors you ran and whether it handed control to
    /// cohors' own install, every cohors on your PATH and where it came from,
    /// the managed versions, whether auto-updates are on and why, the last
    /// update attempt, and what to fix.
    #[command(display_order = 27)]
    Doctor,
    /// Remove cohors' own self-updating copy and what its updates keep.
    ///
    /// Removes ~/.local/share/cohors/versions/, the ~/.local/bin/cohors link
    /// (only when it is cohors' own), and the update state, lock and memo.
    /// Leaves your config and anything cohors did not make, and says how to
    /// remove a copy a package manager installed.
    #[command(
        after_help = "Examples:\n  cohors uninstall --dry-run        list what would be removed\n  cohors uninstall                  remove it"
    )]
    #[command(display_order = 28)]
    Uninstall {
        /// Only list what would be removed.
        #[arg(long)]
        dry_run: bool,
    },
    /// git fetch across the selected repos (non-destructive).
    #[command(
        after_help = "Examples:\n  cohors fetch -s all                 every repo\n  cohors fetch payments web           repos by name\n  cohors fetch -s stale-fetch --json  results as JSON"
    )]
    #[command(display_order = 12)]
    Fetch {
        #[command(flatten)]
        action: ActionArgs,
        #[command(flatten)]
        names: RepoNames,
    },
    /// git pull --ff-only across the selected repos, never merging or losing work.
    #[command(
        after_help = "Examples:\n  cohors pull -s 'behind,!diverged'   every repo a fast-forward can update\n  cohors pull payments                one repo, by name\n  cohors pull -s behind -n            preview the targets"
    )]
    #[command(display_order = 13)]
    Pull {
        #[command(flatten)]
        action: ActionArgs,
        #[command(flatten)]
        names: RepoNames,
    },
    /// git push across the selected repos, never force-pushing.
    #[command(
        after_help = "Examples:\n  cohors push -s 'ahead,!diverged'    every repo with commits to send\n  cohors push -s unpublished          branches never pushed\n  cohors push payments --json         one repo, result as JSON"
    )]
    #[command(display_order = 14)]
    Push {
        #[command(flatten)]
        action: ActionArgs,
        #[command(flatten)]
        names: RepoNames,
    },
    /// git add -A + commit across the selected repos (stages tracked + untracked).
    #[command(
        after_help = "Examples:\n  cohors commit -m \"wip\" -s dirty -n     preview: every file, per repo\n  cohors commit -m \"docs: notes\" notes  one repo, by name\n  cohors commit -m \"fix\" -s dirty --staged  only what you staged"
    )]
    #[command(display_order = 15)]
    Commit {
        #[command(flatten)]
        action: ActionArgs,
        #[command(flatten)]
        names: RepoNames,
        /// Commit message (applied to every selected repo). Omit it to see what
        /// this command does and how to pick repos.
        #[arg(short, long, value_name = "MSG")]
        message: Option<String>,
        /// Commit only what is already staged in each repo (no `git add -A`).
        #[arg(long)]
        staged: bool,
        /// Commit files that look like secrets (`.env`, private keys) too.
        /// Without it, a repo where one would go in is refused.
        #[arg(long)]
        allow_secrets: bool,
    },
    /// git stash push (tracked changes) across the selected repos.
    #[command(
        after_help = "Examples:\n  cohors stash -s dirty -n            preview\n  cohors stash payments web           repos by name"
    )]
    #[command(display_order = 16)]
    Stash {
        #[command(flatten)]
        action: ActionArgs,
        #[command(flatten)]
        names: RepoNames,
    },
    /// Run a shell command in each selected repo (the fleet codemod/test primitive).
    #[command(
        after_help = "Examples:\n  cohors run -s dirty -- cargo test      no quoting needed after --\n  cohors run \"npm test\" -s name:web-*     one quoted command\n  cohors run -s all --json -- git status -s   per-repo output as JSON"
    )]
    #[command(display_order = 17)]
    Run {
        #[command(flatten)]
        action: ActionArgs,
        /// The shell command to run in each repo: one quoted string, or the
        /// words after `--` (`cohors run -s dirty -- cargo test`). Omit it to
        /// see what this command does and how to pick repos.
        #[arg(value_name = "COMMAND")]
        command: Vec<String>,
        /// Per-repo timeout in seconds.
        #[arg(long, default_value_t = 120)]
        timeout: u64,
    },
    /// Build and serve the web dashboard for a folder of repos, then open it.
    ///
    /// The same local scan as the TUI, in the browser: `cohors web --root ~/code`
    /// shows that folder's repos (local status + remote CI/PRs). With `--watch`
    /// the page re-scans live. Release builds and `cargo install cohors` carry
    /// the page inside the binary, so it starts at once. A build from a
    /// checkout has no page inside it: run it in that checkout and it builds the
    /// page with Trunk (installing Trunk the first time unless `--no-install`).
    /// Ctrl-C to stop.
    #[command(
        after_help = "Examples:\n  cohors web --root ~/code          serve one folder's repos and open the browser\n  cohors web --watch --no-open      keep it live, open it yourself"
    )]
    #[command(display_order = 21)]
    Web {
        /// Port to serve on.
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// Don't open a browser window automatically.
        #[arg(long)]
        no_open: bool,
        /// Fail with instructions instead of installing Trunk when it's missing.
        #[arg(long)]
        no_install: bool,
        /// Enable write actions over `/api/action` (`fetch`/`pull`/`push`/`commit`/`stash`).
        #[arg(long)]
        allow_writes: bool,
        /// Enable the `run` action over `/api/action` (arbitrary shell across repos).
        #[arg(long)]
        allow_run: bool,
        /// Serve the built-in sample fleet instead of scanning (like `cohors demo`).
        #[arg(long)]
        demo: bool,
    },
    /// What each repo offers and what work is waiting: flows, agents, tasks.
    ///
    /// A repo opts in by committing `.cohors/repo.json` (ask your agent to
    /// "set up this repo for the launchpad"). Bare, this reports the focus
    /// board and every open task across the fleet; piped or with `--json` it
    /// prints the same data as JSON.
    #[command(
        after_help = "Examples:\n  cohors launchpad                  focus board and open tasks\n  cohors launchpad validate .       check this repo's .cohors/repo.json\n  cohors launchpad --json           flows, agents and tasks per repo"
    )]
    #[command(display_order = 19)]
    Launchpad {
        #[command(subcommand)]
        action: Option<LaunchpadCmd>,
        /// Only include repos matching a selector (same language as `scan`).
        #[arg(short, long, value_name = "QUERY")]
        select: Option<String>,
        /// Print JSON instead of the human report.
        #[arg(long)]
        json: bool,
    },
    /// Start a coding agent: a session, a shipped prompt, a task, or one of the
    /// repo's own agents.
    ///
    /// Name what to start and cohors works out what it is. Bare, it starts a
    /// session in the current repo; at a terminal it first offers the recent
    /// sessions there to resume (Enter starts a new one).
    #[command(
        args_conflicts_with_subcommands = true,
        after_help = "Examples:\n  cohors agent                                   start a session here (or pick one to resume)\n  cohors agent setup-repo                        run a shipped prompt in this repo\n  cohors agent plan-day                          plan your focus across the fleet\n  cohors agent rate-limit-backoff -r payments    work on a task in another repo\n  cohors agent --resume                          pick a recent session to resume\n  cohors agent setup-repo --print                the launch command as JSON, for editors\n  cohors agent ls                                runners, prompts, this repo's agents and tasks"
    )]
    #[command(display_order = 18)]
    Agent(AgentArgs),
    /// Print shell completions: `cohors completions zsh > ~/.zfunc/_cohors`.
    #[command(
        after_help = "Examples:\n  cohors completions bash > ~/.local/share/bash-completion/completions/cohors\n  cohors completions zsh  > ~/.zfunc/_cohors     (with fpath+=~/.zfunc)\n  cohors completions fish > ~/.config/fish/completions/cohors.fish"
    )]
    #[command(display_order = 25)]
    Completions {
        /// bash, zsh, fish, elvish or powershell.
        #[arg(value_name = "SHELL")]
        shell: clap_complete::Shell,
    },
    /// What to do now, each with its command: sessions to resume, next steps (no TUI).
    ///
    /// The fleet's "do next" steps and your focus board are in the list too.
    /// Bare, it lists the suggestions, numbered; `cohors next 2` runs the
    /// second one. The list ranks the agent conversations held in each repo
    /// (Claude Code, Codex) with what needs doing, from the repo you are in.
    /// Last come up to two tips about parts of cohors you have not used yet;
    /// each goes once done, once dismissed, or after five days of showing.
    #[command(
        visible_alias = "suggest",
        after_help = "Examples:\n  cohors next              what to do now, numbered\n  cohors next 1            do the first one\n  cohors next --json       the same list for scripts and agents\n  cohors next --dismiss web   stop suggesting the web dashboard"
    )]
    #[command(display_order = 2)]
    Next {
        /// Run this suggestion (its number in the list). Only cohors itself
        /// is run; a tip for another program is printed for you to run.
        #[arg(value_name = "N")]
        pick: Option<usize>,
        /// How many suggestions to list (tips come after them).
        #[arg(short = 'l', long, default_value_t = 6)]
        limit: usize,
        /// Print JSON instead of the human list.
        #[arg(long)]
        json: bool,
        /// Hide a tip for good: web, extension or claude.
        #[arg(long, value_name = "ID", conflicts_with = "pick")]
        dismiss: Option<String>,
    },
    /// Follow the fleet as it changes, one line per change (no TUI).
    ///
    /// Prints the fleet once it has been read, then again whenever something
    /// changes: a commit, push, pull or fetch, a dirty or clean worktree, a
    /// stash, a repo appearing or going, a launchpad task. Piped or with
    /// `--json`, each line is one JSON object: the overview `cohors overview
    /// --json` prints, the launchpad and the suggestions. Runs until stopped,
    /// or until its input closes when another program started it.
    #[command(display_order = 11)]
    Watch {
        /// Print JSON lines instead of the human summary.
        #[arg(long)]
        json: bool,
    },
    /// Serve your fleet to a coding agent over MCP (read-only by default).
    ///
    /// Speaks Model Context Protocol as a stdio server, so a client spawns it
    /// rather than you. Run it by hand and it prints how to register it and what
    /// an agent would get. Reads are always on; writes are opt-in per flag.
    #[command(
        after_help = "Examples:\n  claude mcp add cohors -- \"$(command -v cohors)\" mcp   register it with Claude Code\n  cohors mcp --allow-writes         let an agent fetch, pull, push, commit, stash"
    )]
    #[command(display_order = 22)]
    Mcp {
        /// Enable write tools (`fetch`, `pull`, `push`, `commit`, `stash`).
        #[arg(long)]
        allow_writes: bool,
        /// Enable the `run` tool (arbitrary shell across repos).
        #[arg(long)]
        allow_run: bool,
        /// Deprecated, and does nothing: it was meant for an `open` tool that
        /// was never built. Still accepted (hidden from --help) so a
        /// registration that passes it keeps starting; it prints a warning.
        #[arg(long, hide = true)]
        allow_open: bool,
        /// Enable the launchpad write tools (`write_repo_contribution`,
        /// `write_board`, `record_activity`): launchpad metadata only, never git.
        #[arg(long)]
        allow_launchpad: bool,
    },
    /// Print one repo's absolute path, and nothing else (no TUI).
    ///
    /// Made for the shell: `cd "$(cohors path payments)"`. A repo is named by
    /// its exact name, alias, id or path first (`.` is the repo you are in),
    /// then by a unique prefix in any case, then by a glob (`'pay*'`). No
    /// match exits 1 and suggests close names; several matches exit 2 and
    /// list them.
    #[command(
        after_help = "Examples:\n  cd \"$(cohors path payments)\"      jump to a repo by name\n  cohors path pay                   a unique prefix is enough"
    )]
    #[command(display_order = 5)]
    Path {
        /// The repo: a name, alias, id, path, unique prefix or glob.
        #[arg(value_name = "REPO")]
        repo: String,
    },
    /// One repo in detail: sync, changes, commits, branches, stashes (no TUI).
    ///
    /// What the dashboard's detail pane shows, as a report: why the repo
    /// needs you, its branch against its upstream, the working tree, recent
    /// commits, every local branch and stash, and (with a GitHub token) its
    /// open PRs. Names a repo like `cohors path`. Piped or with `--json` it
    /// prints what the MCP `get_repo` tool returns.
    #[command(
        after_help = "Examples:\n  cohors show payments              one repo in detail\n  cohors show . --json              the repo you are in, as JSON"
    )]
    #[command(display_order = 4)]
    Show {
        /// The repo: a name, alias, id, path, unique prefix or glob.
        #[arg(value_name = "REPO")]
        repo: String,
        /// Print JSON instead of the human report.
        #[arg(long)]
        json: bool,
    },
    /// The uncommitted files in each repo (no TUI).
    ///
    /// Lists every changed file, repo by repo, with git's two-letter status.
    /// Covers every dirty repo unless `--select` says otherwise. `--patch`
    /// adds the diff, capped per repo. A secret file (`.env`, key material,
    /// credential stores) is named, but its contents are never shown. Piped
    /// or with `--json` it prints what the MCP `changes` tool returns.
    #[command(
        after_help = "Examples:\n  cohors changes                    every uncommitted file, by repo\n  cohors changes -s name:pay* --patch   with the diff"
    )]
    #[command(display_order = 6)]
    Changes {
        /// Only include repos matching a selector (same language as `scan`).
        #[arg(short, long, value_name = "QUERY")]
        select: Option<String>,
        /// Add each repo's diff against HEAD (staged, unstaged and untracked).
        #[arg(long)]
        patch: bool,
        /// Cap each repo's diff at this many bytes.
        #[arg(long, value_name = "BYTES", default_value_t = crate::commands::reads::DEFAULT_PATCH_BYTES)]
        max_bytes: usize,
        /// Print JSON instead of the human report.
        #[arg(long)]
        json: bool,
    },
    /// Find text in every repo's files (no TUI).
    ///
    /// Matches a fixed string, case-sensitively, in the files git does not
    /// ignore, and lists the hits by repo as `path:line`. Secret files
    /// (`.env`, key material, credential stores) are never searched. Covers
    /// the whole fleet unless `--select` says otherwise. Piped or with
    /// `--json` it prints what the MCP `search` tool returns.
    #[command(
        after_help = "Examples:\n  cohors search TODO                every match across the fleet\n  cohors search retry -s name:pay*   only some repos"
    )]
    #[command(display_order = 7)]
    Search {
        /// The text to look for.
        #[arg(value_name = "TEXT")]
        query: String,
        /// Only search repos matching a selector (same language as `scan`).
        #[arg(short, long, value_name = "QUERY")]
        select: Option<String>,
        /// Stop after this many matches across the fleet.
        #[arg(long, value_name = "N", default_value_t = crate::commands::reads::DEFAULT_MAX_RESULTS)]
        max_results: usize,
        /// Print JSON instead of the human report.
        #[arg(long)]
        json: bool,
    },
    /// Open pull requests and CI for each repo on GitHub (no TUI).
    ///
    /// For each repo: the checks on its default branch (and on each branch,
    /// for repos with several), its open PRs by the branch they target, and
    /// how many wait on your review. Needs a GitHub token (`gh auth login`,
    /// or `$GITHUB_TOKEN`); without one it says so and lists nothing. Piped
    /// or with `--json` it prints what the MCP `list_prs` and `ci_status`
    /// tools return.
    #[command(
        after_help = "Examples:\n  cohors prs                        open PRs and CI per repo\n  cohors prs -s ci:failing          only red builds"
    )]
    #[command(display_order = 8)]
    Prs {
        /// Only include repos matching a selector (same language as `scan`).
        #[arg(short, long, value_name = "QUERY")]
        select: Option<String>,
        /// Print JSON instead of the human report.
        #[arg(long)]
        json: bool,
    },
}

/// Shared arguments for the bulk-action subcommands. A selector is still required
/// in practice so an action never silently hits the whole fleet (pass
/// `--select all` to opt into it), but it is `Option` here so that omitting it
/// prints the command's own guide instead of a bare clap error.
#[derive(Debug, clap::Args)]
pub struct ActionArgs {
    /// Which repos to act on, in the same selector language as `scan --select`
    /// (`dirty`, `behind`, `name:pay*`, `'{"ahead":true}'`, `all`, …).
    /// Omit it to see what this command does and how to pick repos.
    #[arg(short, long, value_name = "QUERY")]
    pub select: Option<String>,
    /// Preview the matching repos without acting.
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    /// Print the per-repo results (or, with --dry-run, the targets) as JSON.
    #[arg(long)]
    pub json: bool,
    /// Act without asking first. At a terminal cohors lists the repositories
    /// it is about to change and asks `[y/N]`; a script (no terminal) is never
    /// asked.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

/// Repos named directly, as an alternative to `--select`.
#[derive(Debug, clap::Args)]
pub struct RepoNames {
    /// Repos to act on, by name, unique prefix, glob or path (instead of
    /// --select): `cohors pull payments web`.
    #[arg(value_name = "REPO", conflicts_with = "select")]
    pub repos: Vec<String>,
}

/// `cohors agent [WHAT]`: what to start, and how.
#[derive(Debug, clap::Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub cmd: Option<AgentCmd>,
    /// What to start: a shipped prompt (`setup-repo`, `plan-day`, …), a task
    /// id, or an agent the repo declares. Omit it to start a session.
    #[arg(value_name = "WHAT")]
    pub what: Option<String>,
    /// The repository: a path, or a repo name from your fleet (default: the
    /// repo containing the current directory).
    #[arg(short = 'r', long, value_name = "PATH|NAME")]
    pub repo: Option<String>,
    /// Which runner: `claude-code`, `codex`, `gemini-cli` or `agy` (default:
    /// the first one installed; for --resume, the one that owns the session).
    #[arg(
        short = 'w',
        long = "with",
        visible_alias = "runner",
        value_name = "RUNNER"
    )]
    pub runner: Option<String>,
    /// Take WHAT as a task id (when a name could mean several things).
    #[arg(long, conflicts_with_all = ["agent", "prompt"])]
    pub task: bool,
    /// Take WHAT as an agent the repo declares.
    #[arg(long, conflicts_with = "prompt")]
    pub agent: bool,
    /// Take WHAT as a shipped prompt.
    #[arg(long)]
    pub prompt: bool,
    /// Resume a session: its id, its number in the list, or nothing to pick
    /// from the recent sessions in this repo.
    #[arg(long, value_name = "ID|N", num_args = 0..=1, default_missing_value = "", conflicts_with = "what")]
    pub resume: Option<String>,
    /// Print the launch as JSON (for editors and scripts) instead of starting.
    #[arg(long)]
    pub print: bool,
}

#[derive(Debug, Subcommand)]
pub enum AgentCmd {
    /// What you can start: runners (the default marked), prompts, and the
    /// current repo's agents, tasks and recent sessions.
    #[command(visible_alias = "list")]
    Ls {
        /// Print JSON instead of the human list.
        #[arg(long)]
        json: bool,
    },
    /// Which agent runners are installed, and why one is not.
    #[command(hide = true)]
    Runners {
        /// Print JSON instead of the human list.
        #[arg(long)]
        json: bool,
    },
    /// The prompts cohors ships (`setup-repo`, `plan-day`, …).
    #[command(hide = true)]
    Prompts {
        /// Print JSON instead of the human list.
        #[arg(long)]
        json: bool,
    },
    /// Start an agent in this terminal (the 0.7 form of `cohors agent WHAT`).
    #[command(hide = true)]
    Run(AgentLaunch),
    /// Print the command that would start an agent, as JSON (the 0.7 form of
    /// `cohors agent WHAT --print`; the editor extension uses it).
    #[command(hide = true)]
    Command(AgentLaunch),
    /// Start a runner on a prompt file cohors already wrote. This is what the
    /// printed command calls on shells that cannot pass a file's contents to
    /// a program safely; it is not meant to be typed by hand.
    #[command(hide = true)]
    Exec {
        /// The runner id.
        #[arg(long)]
        runner: String,
        /// A prompt file under the launchpad home's `prompts/` directory.
        #[arg(long, value_name = "PATH")]
        prompt_file: String,
        /// Start the conversation under this session id.
        #[arg(long)]
        session: Option<String>,
    },
}

/// What to start and where. Shared by `agent run` and `agent command`.
#[derive(Debug, clap::Args)]
pub struct AgentLaunch {
    /// A shipped prompt (`setup-repo`, `declare-flows`, `add-agent`,
    /// `track-tasks`, `plan-day`), or `agent` (with --agent), `task` (with
    /// --task), `start` (just open a session) or `resume` (with --session).
    #[arg(value_name = "WHAT")]
    pub what: String,
    /// Which runner to use: `claude-code`, `codex`, `gemini-cli` or `agy`.
    /// Omit it to use the first one installed.
    #[arg(long, value_name = "ID")]
    pub runner: Option<String>,
    /// The repository: a path, or a repo name from your fleet (default: the
    /// repo containing the current directory).
    #[arg(long, value_name = "PATH|NAME")]
    pub repo: Option<String>,
    /// The id of an agent the repo declares (for `agent`).
    #[arg(long, value_name = "ID")]
    pub agent: Option<String>,
    /// The id of a task in the repo (for `task`).
    #[arg(long, value_name = "ID")]
    pub task: Option<String>,
    /// The session to re-enter (for `resume`).
    #[arg(long, value_name = "ID")]
    pub session: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum LaunchpadCmd {
    /// Check a repo's `.cohors/repo.json` against the contract.
    Validate {
        /// The repository to check (default: the current directory).
        #[arg(value_name = "PATH")]
        path: Option<String>,
        /// Print the result as JSON (`valid`, `errors`, `warnings`).
        #[arg(long)]
        json: bool,
    },
    /// Print the contract a `.cohors/repo.json` must satisfy, as JSON.
    Contract,
    /// Append an agent-session event to the activity log (used by hooks).
    Record {
        /// `start`, `end`, `launch`, or `note`.
        #[arg(long)]
        phase: String,
        /// Which agent runner this is.
        #[arg(long, default_value = "claude-code")]
        runner: String,
        /// The repo-declared agent, if one is running.
        #[arg(long)]
        agent: Option<String>,
        /// The repository (default: the one containing the current directory).
        #[arg(long, value_name = "PATH")]
        repo: Option<String>,
    },
}
