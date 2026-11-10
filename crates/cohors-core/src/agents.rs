//! Agent runners and the prompts cohors hands them.
//!
//! cohors is meant to be used *with* coding agents, not only looked at. This
//! module is the pure half of that: which agent command-line tools exist
//! ("runners"), how each one is started so that a prompt is submitted **and**
//! the user is left in an interactive session, and the prompts themselves
//! ("set up this repo", "plan my day", …).
//!
//! No runner is privileged: each is one row in [`RUNNERS`] and every runner
//! receives the same prompt text.
//!
//! Like the rest of `cohors-core` nothing here touches the disk or spawns a
//! process. Finding which runners are installed, writing the prompt file and
//! starting the terminal live in the adapter crates and the front-ends.
//!
//! Two ways to start a runner come out of this module:
//!
//! - [`Runner::argv`] — the argument list for spawning the runner directly,
//!   with the prompt *text* as one argument. No shell is involved, so there is
//!   nothing to quote. Used by `cohors agent run` in the current terminal.
//! - [`Runner::shell_command`] — one line to type into a terminal (an editor's
//!   integrated terminal, or copied from the web page). The prompt is read from
//!   a file at run time, so the line stays short and prompt text can never be
//!   interpreted by the shell.

use serde::Serialize;

use crate::launchpad::Task;

/// One piece of a runner's command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arg {
    /// A fixed flag or subcommand, e.g. `--resume`.
    Lit(&'static str),
    /// The prompt goes here.
    Prompt,
    /// The session id goes here.
    Session,
}

/// How to start a session: `start` submits a prompt, `linked` does the same
/// under a session id cohors chooses (so the conversation can be found again),
/// `resume` re-enters a session by id. A runner that cannot do one of the last
/// two has `None` there; cohors never pretends a capability a tool lacks.
#[derive(Debug, Clone, Copy)]
pub struct Runner {
    /// Stable id used in `.principia/repo.json` (`supports`) and on the CLI.
    pub id: &'static str,
    /// Name shown to people.
    pub label: &'static str,
    /// The executable to look for on the PATH.
    pub bin: &'static str,
    start: &'static [Arg],
    linked: Option<&'static [Arg]>,
    resume: Option<&'static [Arg]>,
}

/// Every runner cohors knows how to start.
///
/// Each mapping must do both things: submit the prompt as the first user turn
/// *and* stay interactive. A "print" or "headless" flag would run once and
/// exit; a "system prompt" flag would open an idle session with nothing asked.
pub const RUNNERS: [Runner; 4] = [
    Runner {
        id: "claude-code",
        label: "Claude Code",
        bin: "claude",
        // `claude [prompt]`: a positional prompt starts an interactive session
        // with that prompt already submitted (`-p` would be headless).
        start: &[Arg::Prompt],
        linked: Some(&[Arg::Lit("--session-id"), Arg::Session, Arg::Prompt]),
        resume: Some(&[Arg::Lit("--resume"), Arg::Session]),
    },
    Runner {
        id: "codex",
        label: "Codex",
        bin: "codex",
        // `codex [PROMPT]`: positional. Codex has no flag to choose a new
        // session's id, so a launch cannot be linked; it can still resume one.
        start: &[Arg::Prompt],
        linked: None,
        resume: Some(&[Arg::Lit("resume"), Arg::Session]),
    },
    Runner {
        id: "gemini-cli",
        label: "Gemini CLI",
        bin: "gemini",
        // `-i/--prompt-interactive` runs the prompt and stays interactive
        // (`-p` is the headless mode). `--resume` takes "latest" or an index,
        // not an id, so a recorded session cannot be reopened directly.
        start: &[Arg::Lit("-i"), Arg::Prompt],
        linked: Some(&[
            Arg::Lit("--session-id"),
            Arg::Session,
            Arg::Lit("-i"),
            Arg::Prompt,
        ]),
        resume: None,
    },
    Runner {
        id: "agy",
        label: "agy",
        bin: "agy",
        // `--prompt` is an alias for the headless `--print`;
        // `--prompt-interactive` is the one that keeps the session.
        start: &[Arg::Lit("--prompt-interactive"), Arg::Prompt],
        linked: None,
        resume: Some(&[Arg::Lit("--conversation"), Arg::Session]),
    },
];

/// Look a runner up by id.
pub fn runner(id: &str) -> Option<&'static Runner> {
    RUNNERS.iter().find(|r| r.id == id)
}

/// Which shell a command line is written for. The two differ in how a quoted
/// string and "the contents of this file" are spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    /// sh, bash, zsh, fish: `'…'` and `"$(cat '…')"`.
    Posix,
    /// PowerShell: `'…'` and `(Get-Content -Raw '…')`.
    PowerShell,
}

impl Shell {
    /// The shell a terminal on this platform most likely runs.
    pub fn native() -> Self {
        if cfg!(windows) {
            Shell::PowerShell
        } else {
            Shell::Posix
        }
    }

    /// Quote `text` so the shell passes it through as one literal argument.
    /// Inside single quotes nothing expands in either shell; only the quote
    /// character itself needs escaping.
    pub fn quote(self, text: &str) -> String {
        match self {
            Shell::Posix => format!("'{}'", text.replace('\'', "'\\''")),
            Shell::PowerShell => format!("'{}'", text.replace('\'', "''")),
        }
    }

    /// An expression that expands to the contents of `file` as one argument.
    fn file_contents(self, file: &str) -> String {
        match self {
            // The command substitution is inside double quotes, so its output is
            // one argument and is never re-expanded: prompt text cannot break out.
            Shell::Posix => format!("\"$(cat {})\"", self.quote(file)),
            Shell::PowerShell => format!("(Get-Content -Raw {})", self.quote(file)),
        }
    }
}

/// Make sure a prompt cannot be mistaken for a command-line flag.
///
/// The prompt reaches the runner as one argument. Its text can come from a
/// file inside a repository (a repo-declared agent), which on a cloned repo
/// is written by someone else. If that text began with `-`, the runner's
/// argument parser could read it as an option (for instance one that turns
/// off permission prompts) instead of as the prompt. An argument is only
/// treated as an option when its first character is `-`, so putting a fixed
/// line in front removes the possibility for every runner, whatever its
/// parser. Prompts that do not start with `-` are returned unchanged.
pub fn guard_prompt(text: &str) -> String {
    if text.starts_with('-') {
        format!("Prompt:\n\n{text}")
    } else {
        text.to_string()
    }
}

/// What to start: a new session with a prompt, the same under a chosen session
/// id, or an existing session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch<'a> {
    Start,
    Linked { session: &'a str },
    Resume { session: &'a str },
}

impl Runner {
    /// Can a launch be started under a session id cohors chooses?
    pub fn can_link(&self) -> bool {
        self.linked.is_some()
    }

    /// Can a session be re-entered by id?
    pub fn can_resume(&self) -> bool {
        self.resume.is_some()
    }

    fn args(&self, launch: Launch<'_>) -> Option<(&'static [Arg], Option<String>)> {
        match launch {
            Launch::Start => Some((self.start, None)),
            Launch::Linked { session } => self.linked.map(|a| (a, Some(session.to_string()))),
            Launch::Resume { session } => self.resume.map(|a| (a, Some(session.to_string()))),
        }
    }

    /// Arguments for spawning the runner directly, with `prompt` as the prompt
    /// text. `None` when this runner cannot do that kind of launch.
    pub fn argv(&self, launch: Launch<'_>, prompt: &str) -> Option<Vec<String>> {
        let (args, session) = self.args(launch)?;
        Some(
            args.iter()
                .map(|arg| match arg {
                    Arg::Lit(text) => (*text).to_string(),
                    // Guarded here as well as where the prompt file is written,
                    // so no caller can hand a runner a prompt that reads as a flag.
                    Arg::Prompt => guard_prompt(prompt),
                    Arg::Session => session.clone().unwrap_or_default(),
                })
                .collect(),
        )
    }

    /// One line to type into a terminal running `shell`. The prompt is read
    /// from `prompt_file` when the line runs, so the file's contents must have
    /// been passed through [`guard_prompt`] before they were written. `None` when this runner cannot do
    /// that kind of launch.
    pub fn shell_command(
        &self,
        shell: Shell,
        launch: Launch<'_>,
        prompt_file: &str,
    ) -> Option<String> {
        let (args, session) = self.args(launch)?;
        let mut line = self.bin.to_string();
        for arg in args {
            line.push(' ');
            match arg {
                Arg::Lit(text) => line.push_str(text),
                Arg::Prompt => line.push_str(&shell.file_contents(prompt_file)),
                Arg::Session => line.push_str(&shell.quote(session.as_deref().unwrap_or(""))),
            }
        }
        Some(line)
    }
}

/// Whether one runner is installed, as the front-ends report it. A runner that
/// is missing is listed with the reason rather than hidden, so a disabled
/// button can say why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunnerStatus {
    pub id: String,
    pub label: String,
    pub bin: String,
    pub available: bool,
    /// Where the executable was found.
    pub path: Option<String>,
    /// Why it is unavailable, when it is.
    pub reason: Option<String>,
    pub can_link: bool,
    pub can_resume: bool,
}

impl RunnerStatus {
    /// Describe `runner` given where its executable was found (if anywhere).
    pub fn new(runner: &Runner, found: Option<String>) -> Self {
        RunnerStatus {
            id: runner.id.to_string(),
            label: runner.label.to_string(),
            bin: runner.bin.to_string(),
            available: found.is_some(),
            reason: found
                .is_none()
                .then(|| format!("{} was not found on your PATH", runner.bin)),
            path: found,
            can_link: runner.can_link(),
            can_resume: runner.can_resume(),
        }
    }
}

/// A session id is whatever the runner printed, but it must at least look like
/// one before it is put on a command line: hex digits and dashes only.
pub fn is_session_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

// ── Prompts ──────────────────────────────────────────────────────────────────

/// A prompt cohors itself ships. Front-ends name one by `id`, never by path,
/// so nothing can point a runner at an arbitrary file.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Prompt {
    pub id: &'static str,
    /// Short name for a button or menu.
    pub title: &'static str,
    /// One line on what running it does.
    pub summary: &'static str,
    /// `true` when it works on one repository; `false` when it works across
    /// the fleet (the focus board).
    pub needs_repo: bool,
    #[serde(skip)]
    pub body: &'static str,
}

/// The prompts, in the order a person meets them.
pub const PROMPTS: [Prompt; 5] = [
    Prompt {
        id: "setup-repo",
        title: "Set up this repo",
        summary: "Read the repository and write its .principia/repo.json: flows, agents and tasks.",
        needs_repo: true,
        body: include_str!("../prompts/setup-repo.md"),
    },
    Prompt {
        id: "declare-flows",
        title: "Declare flows",
        summary: "Decide which run, test and build commands are worth declaring beyond what is detected.",
        needs_repo: true,
        body: include_str!("../prompts/declare-flows.md"),
    },
    Prompt {
        id: "add-agent",
        title: "Add an agent",
        summary: "Decide whether the repo should ship a prompt for recurring, repo-specific work.",
        needs_repo: true,
        body: include_str!("../prompts/add-agent.md"),
    },
    Prompt {
        id: "track-tasks",
        title: "Track tasks",
        summary: "Record the repo's current, evidence-backed work as short-lived tasks.",
        needs_repo: true,
        body: include_str!("../prompts/track-tasks.md"),
    },
    Prompt {
        id: "plan-day",
        title: "Plan my day",
        summary: "Plan focus across every repository from real evidence and save the focus board.",
        needs_repo: false,
        body: include_str!("../prompts/plan-day.md"),
    },
];

/// Look a shipped prompt up by id.
pub fn prompt(id: &str) -> Option<&'static Prompt> {
    PROMPTS.iter().find(|p| p.id == id)
}

/// The repository a prompt is about. Committed prompt files must not contain
/// machine-specific paths, so they carry `{{PLACEHOLDERS}}` and the real values
/// are only ever put into the throwaway copy handed to the runner.
#[derive(Debug, Clone, Copy, Default)]
pub struct PromptContext<'a> {
    pub repo_name: &'a str,
    pub repo_root: &'a str,
    pub branch: &'a str,
    pub task_title: &'a str,
}

/// Fill a prompt's placeholders from `ctx`.
pub fn substitute(body: &str, ctx: &PromptContext<'_>) -> String {
    body.replace("{{REPO_NAME}}", ctx.repo_name)
        .replace("{{REPO_ROOT}}", ctx.repo_root)
        .replace("{{BRANCH}}", ctx.branch)
        .replace("{{TASK_TITLE}}", ctx.task_title)
}

/// A shipped prompt, ready to hand to a runner: placeholders filled, then a
/// short tail saying where the work happens. For a fleet-wide prompt the tail
/// lists the repositories (`fleet` is `(name, path)` pairs) and where the
/// board lives.
pub fn render_prompt(
    prompt: &Prompt,
    ctx: &PromptContext<'_>,
    fleet: &[(String, String)],
    board_path: &str,
) -> String {
    let mut out = substitute(prompt.body, ctx);
    out.push_str("\n\n---\n\n");
    if prompt.needs_repo {
        out.push_str(&format!(
            "You are working in the repository at: {}\n",
            ctx.repo_root
        ));
    } else {
        out.push_str("Repositories cohors currently knows about:\n");
        if fleet.is_empty() {
            out.push_str("(none discovered yet)\n");
        }
        for (name, path) in fleet {
            out.push_str(&format!("{name}  ({path})\n"));
        }
        out.push_str(&format!("\nWrite the board to: {board_path}\n"));
    }
    out
}

/// The prompt for a repo-declared agent: the repo's own prompt file with the
/// placeholders filled, plus the specific task when it was started from one.
pub fn agent_prompt(body: &str, ctx: &PromptContext<'_>, task: Option<&Task>) -> String {
    let mut out = substitute(body, ctx);
    if let Some(task) = task {
        out.push_str(&format!("\n\n---\n\nSpecific task: {}", task.title));
        if let Some(notes) = task.notes.as_deref().filter(|n| !n.is_empty()) {
            out.push_str(&format!("\n\n{notes}"));
        }
    }
    out
}

/// The prompt for working on one task when the repo ships no agent for it.
pub fn task_prompt(ctx: &PromptContext<'_>, task: &Task) -> String {
    let branch = if ctx.branch.is_empty() {
        String::new()
    } else {
        format!(" on branch {}", ctx.branch)
    };
    let mut out = format!(
        "You are working in {} at {}{branch}.\n\nTask: {}\n",
        ctx.repo_name, ctx.repo_root, task.title
    );
    if let Some(notes) = task.notes.as_deref().filter(|n| !n.is_empty()) {
        out.push_str(&format!("Notes: {notes}\n"));
    }
    out.push_str(
        "\nWork on this task. Only mark it \"done\" in .principia/repo.json with real evidence \
         (a commit, a merged PR, or the user confirming) — never invent completion.\n\
         Validate any repo.json edit with: cohors launchpad validate .\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_ids_match_the_launchpad_contract() {
        let ids: Vec<&str> = RUNNERS.iter().map(|r| r.id).collect();
        assert_eq!(ids, crate::launchpad::RUNNERS);
    }

    #[test]
    fn every_runner_submits_the_prompt_and_stays_interactive() {
        for r in &RUNNERS {
            let argv = r.argv(Launch::Start, "do the thing").unwrap();
            assert!(
                argv.contains(&"do the thing".to_string()),
                "{} drops the prompt",
                r.id
            );
            // The headless spellings must never appear.
            for bad in ["-p", "--print", "--prompt"] {
                assert!(!argv.iter().any(|a| a == bad), "{} is headless", r.id);
            }
        }
    }

    #[test]
    fn shell_commands_read_the_prompt_from_a_quoted_file() {
        let claude = runner("claude-code").unwrap();
        assert_eq!(
            claude
                .shell_command(Shell::Posix, Launch::Start, "/tmp/p.md")
                .unwrap(),
            "claude \"$(cat '/tmp/p.md')\""
        );
        assert_eq!(
            claude
                .shell_command(
                    Shell::Posix,
                    Launch::Linked { session: "abc-123" },
                    "/tmp/p.md"
                )
                .unwrap(),
            "claude --session-id 'abc-123' \"$(cat '/tmp/p.md')\""
        );
        assert_eq!(
            claude
                .shell_command(Shell::Posix, Launch::Resume { session: "abc-123" }, "")
                .unwrap(),
            "claude --resume 'abc-123'"
        );
        assert_eq!(
            runner("gemini-cli")
                .unwrap()
                .shell_command(Shell::PowerShell, Launch::Start, "C:\\t\\p.md")
                .unwrap(),
            "gemini -i (Get-Content -Raw 'C:\\t\\p.md')"
        );
    }

    #[test]
    fn a_hostile_path_cannot_break_out_of_its_quotes() {
        // A path from a cloned repository is attacker-controlled.
        let evil = "/tmp/x'; rm -rf ~; echo '";
        assert_eq!(
            Shell::Posix.quote(evil),
            "'/tmp/x'\\''; rm -rf ~; echo '\\'''"
        );
        assert_eq!(Shell::PowerShell.quote("it's"), "'it''s'");
    }

    #[test]
    fn a_prompt_can_never_be_read_as_a_flag() {
        // A repo-declared agent file is untrusted; this one tries to smuggle
        // an option to the runner in place of a prompt.
        let hostile = "--dangerously-skip-permissions";
        assert_eq!(guard_prompt(hostile), format!("Prompt:\n\n{hostile}"));
        assert_eq!(guard_prompt("Fix the build."), "Fix the build.");
        for r in &RUNNERS {
            for launch in [
                Launch::Start,
                Launch::Linked {
                    session: "abcdef12",
                },
            ] {
                let Some(argv) = r.argv(launch, hostile) else {
                    continue;
                };
                let prompt = argv.last().unwrap();
                assert!(
                    !prompt.starts_with('-'),
                    "{} passes a flag-like prompt",
                    r.id
                );
                assert!(prompt.ends_with(hostile));
            }
        }
    }

    #[test]
    fn capabilities_are_claimed_only_where_real() {
        let codex = runner("codex").unwrap();
        assert!(!codex.can_link() && codex.can_resume());
        assert!(codex.argv(Launch::Linked { session: "s" }, "p").is_none());
        let gemini = runner("gemini-cli").unwrap();
        assert!(gemini.can_link() && !gemini.can_resume());
        assert!(
            gemini
                .shell_command(Shell::Posix, Launch::Resume { session: "s" }, "")
                .is_none()
        );
        assert!(runner("nope").is_none());
    }

    #[test]
    fn runner_status_explains_a_missing_runner() {
        let r = runner("agy").unwrap();
        let missing = RunnerStatus::new(r, None);
        assert!(!missing.available);
        assert_eq!(
            missing.reason.as_deref(),
            Some("agy was not found on your PATH")
        );
        let found = RunnerStatus::new(r, Some("/usr/local/bin/agy".to_string()));
        assert!(found.available && found.reason.is_none());
    }

    #[test]
    fn session_ids_are_hex_and_dashes_only() {
        assert!(is_session_id("3f2a9c1e-7b4d-4e8a-9c0f-1a2b3c4d5e6f"));
        assert!(!is_session_id("short"));
        assert!(!is_session_id("abc123; rm -rf ~"));
    }

    #[test]
    fn shipped_prompts_point_at_cohors_not_the_old_tool() {
        assert_eq!(PROMPTS.len(), 5);
        for p in &PROMPTS {
            assert!(!p.body.trim().is_empty(), "{} is empty", p.id);
            // Every leftover template variable must be one `substitute` fills.
            let mut rest = p.body;
            while let Some(at) = rest.find("{{") {
                let end = rest[at..].find("}}").expect("closed placeholder") + at;
                let name = &rest[at + 2..end];
                assert!(
                    ["REPO_NAME", "REPO_ROOT", "BRANCH", "TASK_TITLE"].contains(&name),
                    "{} has an unfilled placeholder {name}",
                    p.id
                );
                rest = &rest[end + 2..];
            }
            assert!(
                !p.body.contains("validate.js"),
                "{} names the old validator",
                p.id
            );
            assert!(
                !p.body.contains("Principia"),
                "{} names the old product",
                p.id
            );
        }
        assert!(
            prompt("setup-repo")
                .unwrap()
                .body
                .contains("cohors launchpad")
        );
        assert!(prompt("nope").is_none());
    }

    #[test]
    fn rendering_says_where_the_work_happens() {
        let ctx = PromptContext {
            repo_name: "app",
            repo_root: "/code/app",
            branch: "main",
            task_title: "",
        };
        let repo = render_prompt(prompt("setup-repo").unwrap(), &ctx, &[], "/h/board.json");
        assert!(repo.ends_with("You are working in the repository at: /code/app\n"));
        assert!(!repo.contains("{{REPO_NAME}}"));

        let fleet = [("app".to_string(), "/code/app".to_string())];
        let board = render_prompt(prompt("plan-day").unwrap(), &ctx, &fleet, "/h/board.json");
        assert!(board.contains("app  (/code/app)"));
        assert!(board.ends_with("Write the board to: /h/board.json\n"));
        let none = render_prompt(prompt("plan-day").unwrap(), &ctx, &[], "/h/board.json");
        assert!(none.contains("(none discovered yet)"));
    }

    #[test]
    fn task_and_agent_prompts_carry_the_task() {
        let ctx = PromptContext {
            repo_name: "app",
            repo_root: "/code/app",
            branch: "feat/x",
            task_title: "Ship it",
        };
        let task = Task {
            id: "ship".to_string(),
            title: "Ship it".to_string(),
            notes: Some("Behind the flag.".to_string()),
            ..Default::default()
        };
        let text = task_prompt(&ctx, &task);
        assert!(text.starts_with("You are working in app at /code/app on branch feat/x."));
        assert!(text.contains("Task: Ship it\nNotes: Behind the flag."));
        assert!(text.contains("cohors launchpad validate ."));

        let agent = agent_prompt("In {{REPO_NAME}}: {{TASK_TITLE}}", &ctx, Some(&task));
        assert!(agent.starts_with("In app: Ship it"));
        assert!(agent.ends_with("Specific task: Ship it\n\nBehind the flag."));
        assert_eq!(agent_prompt("plain", &ctx, None), "plain");
    }
}
