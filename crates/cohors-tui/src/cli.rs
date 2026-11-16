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

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Write a starter config file and print its path.
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
    Overview {
        /// Print JSON instead of the human report.
        #[arg(long)]
        json: bool,
    },
    /// Your commits across every repo, as a paste-ready digest (no TUI).
    ///
    /// Lists the commits you authored (matched by `git config user.email`) in
    /// the last seven days, grouped by repo with the most active first, as
    /// markdown you can paste into a standup channel.
    Standup {
        /// Only today's commits instead of the last seven days.
        #[arg(long)]
        today: bool,
        /// Print the commits grouped by repo as JSON instead of markdown.
        #[arg(long)]
        json: bool,
    },
    /// Launch the dashboard on a built-in sample fleet: no config, no scanning.
    ///
    /// A zero-setup way to try cohors: every column and view is populated with
    /// privacy-safe demo data. Actions are stubbed (nothing touches your disk).
    Demo,
    /// Update cohors to the newest release, the way this copy was installed.
    ///
    /// Asks GitHub for the newest release and installs it with Homebrew, pip,
    /// or the release's shell installer, whichever installed this copy. A
    /// build from source, or a copy run through `uvx`, only says how to get it.
    /// The dashboard does this by itself once a day (the `updates` setting).
    Update {
        /// Only say whether a newer release exists; install nothing.
        #[arg(long)]
        check: bool,
    },
    /// git fetch across the selected repos (non-destructive).
    Fetch(ActionArgs),
    /// git pull --ff-only across the selected repos, never merging or losing work.
    Pull(ActionArgs),
    /// git push across the selected repos, never force-pushing.
    Push(ActionArgs),
    /// git add -A + commit across the selected repos (stages tracked + untracked).
    Commit {
        #[command(flatten)]
        action: ActionArgs,
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
    Stash(ActionArgs),
    /// Run a shell command in each selected repo (the fleet codemod/test primitive).
    Run {
        #[command(flatten)]
        action: ActionArgs,
        /// The shell command to run in each repo. Omit it to see what this
        /// command does and how to pick repos.
        #[arg(value_name = "COMMAND")]
        command: Option<String>,
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
    /// Start a coding agent on a repo, a task, or the whole fleet.
    ///
    /// `cohors agent run setup-repo` asks your agent (Claude Code, Codex,
    /// Gemini CLI or agy, whichever is installed) to read the current repo and
    /// write its launchpad file; `cohors agent run plan-day` plans your focus
    /// across every repo. `cohors agent runners` shows what is installed.
    Agent {
        #[command(subcommand)]
        action: AgentCmd,
    },
    /// Serve your fleet to a coding agent over MCP (read-only by default).
    ///
    /// Speaks Model Context Protocol as a stdio server, so a client spawns it
    /// rather than you. Run it by hand and it prints how to register it and what
    /// an agent would get. Reads are always on; writes are opt-in per flag.
    Mcp {
        /// Enable write tools (`fetch`, `pull`, `push`, `commit`, `stash`).
        #[arg(long)]
        allow_writes: bool,
        /// Enable the `run` tool (arbitrary shell across repos).
        #[arg(long)]
        allow_run: bool,
        /// Enable the local-only `open` tool (shared desktop session).
        #[arg(long)]
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
}

#[derive(Debug, Subcommand)]
pub enum AgentCmd {
    /// Which agent runners are installed, and why one is not.
    Runners {
        /// Print JSON instead of the human list.
        #[arg(long)]
        json: bool,
    },
    /// The prompts cohors ships (`setup-repo`, `plan-day`, …).
    Prompts {
        /// Print JSON instead of the human list.
        #[arg(long)]
        json: bool,
    },
    /// Start an agent in this terminal.
    Run(AgentLaunch),
    /// Print the command that would start an agent, as JSON (for editors and
    /// scripts). Writes the prompt file and logs the launch, like `run`.
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
