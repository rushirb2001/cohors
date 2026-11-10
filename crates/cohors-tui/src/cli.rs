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
    #[arg(long, value_name = "PATH", global = true)]
    pub config: Option<String>,

    /// Override the configured roots; repeatable (e.g. `--root ~/work --root ~/oss`).
    #[arg(long, value_name = "DIR", global = true)]
    pub root: Vec<String>,

    /// Force a fresh scan, ignoring the cache.
    #[arg(long, global = true)]
    pub no_cache: bool,

    /// Keep the dashboard live: re-scan every few seconds (skips the launch screen).
    #[arg(long, global = true)]
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
    /// Discover and snapshot every repo, then report what needs you (no TUI).
    ///
    /// At a terminal this prints a grouped report: what needs you now, what is
    /// minor, what is calm, and the commands that clear it. Piped or redirected
    /// (or with `--json`) it prints the raw JSON snapshots instead.
    Scan {
        /// Only include repos matching a selector: JSON (`'{"behind":true}'`)
        /// or shorthand (`dirty`, `behind`, `ahead`, `attention`, `name:pay*`,
        /// `ci:failing`, comma-separated to AND them). Results print in
        /// dirty-first order. Omit to print the whole fleet.
        #[arg(long, value_name = "QUERY")]
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
    /// the page re-scans live. One command starts everything: it finds the
    /// `cohors-web` crate, makes sure Trunk (the WASM bundler) is installed
    /// (installing it for you the first time unless `--no-install`), then runs the
    /// server and prints the URL. Run it from inside the cohors repository (it
    /// builds the web app from source). Ctrl-C to stop.
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
        #[arg(long, value_name = "QUERY")]
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
    #[arg(long, value_name = "QUERY")]
    pub select: Option<String>,
    /// Preview the matching repos without acting.
    #[arg(long)]
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
