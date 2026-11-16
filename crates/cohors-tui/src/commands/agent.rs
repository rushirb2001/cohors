//! `cohors agent` — start a coding agent on a repo, a task, or the whole fleet.
//!
//! cohors is built to be used *with* agents. This is the one place every
//! surface (this CLI, the dashboard, the editor extension) gets an agent
//! started, so they all behave identically:
//!
//! - `cohors agent [WHAT]` — start it in this terminal. WHAT is a shipped
//!   prompt (`setup-repo`, `plan-day`, …), a task id, or an agent the repo
//!   declares; cohors works out which. Bare, it starts a session, first
//!   offering the recent sessions in the repo to resume at a terminal.
//! - `cohors agent --resume [ID|N]` — reopen a session; with no value, pick
//!   one from the recent sessions in this repo (ranked by `cohors_core::suggest`).
//! - `cohors agent WHAT --print` — print the launch as JSON, for an editor that
//!   types the command into its own terminal.
//! - `cohors agent ls` — what you can start.
//!
//! The 0.7 forms (`agent run|command <what> --task/--agent/--session`,
//! `agent runners`, `agent prompts`) still work, hidden: the editor extension
//! and existing scripts use them.
//!
//! The runner table and prompts are pure data in `cohors_core::agents`; the
//! planning (prompt file, conversation link, activity log) is in the launchpad
//! adapter, reached through the fleet facade.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use cohors_core::RepoSnapshot;
use cohors_core::agents::{PROMPTS, RunnerStatus};
use cohors_fleet::launchpad::{self as lp, LaunchKind, LaunchPlan, LaunchRequest, RepoTarget};
use serde_json::json;

use super::launchpad::{head_branch, repo_root};
use super::{bold, dim, guide_row, now_secs, section, sgr, wants_color};
use crate::cli::{AgentArgs, AgentCmd, AgentLaunch, Cli};

pub fn run(cli: &Cli, args: &AgentArgs) -> Result<()> {
    let Some(action) = &args.cmd else {
        return launch_new(cli, args);
    };
    match action {
        AgentCmd::Ls { json } => ls(cli, args.repo.as_deref(), *json),
        AgentCmd::Runners { json } => runners(*json),
        AgentCmd::Prompts { json } => prompts(*json),
        AgentCmd::Command(launch) => {
            let plan = plan(cli, launch)?;
            println!("{}", serde_json::to_string_pretty(&plan.to_json())?);
            Ok(())
        }
        AgentCmd::Run(launch) => {
            let plan = plan(cli, launch)?;
            start(&plan)
        }
        AgentCmd::Exec {
            runner,
            prompt_file,
            session,
        } => exec(runner, prompt_file, session.as_deref()),
    }
}

/// `cohors agent exec` — start `runner_id` on a prompt file cohors wrote, in
/// the current directory. The prompt text goes from the file straight into the
/// runner's argument list: no shell handles it, so nothing in it can be read
/// as a second argument or a flag.
fn exec(runner_id: &str, prompt_file: &str, session: Option<&str>) -> Result<()> {
    use cohors_core::agents::{Launch, is_session_id, runner};
    let home = lp::launchpad_home().context("no home directory for the launchpad")?;
    let body = lp::read_prompt_file(&home, Path::new(prompt_file))?;
    let found = runner(runner_id).with_context(|| format!("unknown runner `{runner_id}`"))?;
    let launch = match session {
        Some(id) if !is_session_id(id) => bail!("that does not look like a session id"),
        Some(session) => Launch::Linked { session },
        None => Launch::Start,
    };
    let args = found
        .argv(launch, &body)
        .with_context(|| format!("{} cannot start that kind of session", found.label))?;
    let status = lp::detect_runners()
        .into_iter()
        .find(|s| s.id == runner_id && s.available)
        .with_context(|| format!("{} is not installed", found.label))?;
    let program = status.path.unwrap_or_else(|| found.bin.to_string());
    let exit = std::process::Command::new(&program)
        .args(&args)
        .status()
        .with_context(|| format!("starting {program}"))?;
    if !exit.success() {
        std::process::exit(exit.code().unwrap_or(1));
    }
    Ok(())
}

/// `cohors agent runners`.
fn runners(as_json: bool) -> Result<()> {
    let statuses = lp::detect_runners();
    let human = super::wants_report(as_json);
    if !human {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "runners": statuses }))?
        );
        return Ok(());
    }
    print!("{}", runners_report(&statuses, wants_color(true)));
    Ok(())
}

/// The human-facing runner list. Pure, so it is unit-testable.
fn runners_report(statuses: &[RunnerStatus], color: bool) -> String {
    let installed = statuses.iter().filter(|s| s.available).count();
    let mut out = String::from("\n");
    out.push_str(&section(
        "agent runners",
        &format!("{installed} of {} installed", statuses.len()),
        color,
    ));
    let width = statuses
        .iter()
        .map(|s| s.label.chars().count())
        .max()
        .unwrap_or(0);
    for s in statuses {
        // Shape first, so it still reads with colour off.
        let (mark, detail) = match (&s.path, &s.reason) {
            (Some(path), _) => (sgr(color, "32", "●"), path.clone()),
            (None, reason) => (
                dim(color, "○"),
                reason
                    .clone()
                    .unwrap_or_else(|| "not installed".to_string()),
            ),
        };
        out.push_str(&format!(
            "    {mark} {}  {}\n",
            bold(color, &format!("{:<width$}", s.label)),
            dim(color, &detail),
        ));
    }
    out.push('\n');
    if installed == 0 {
        out.push_str(&format!(
            "  {}\n\n",
            dim(
                color,
                "install any one of these, then run `cohors agent setup-repo` in a repo"
            )
        ));
    }
    out
}

/// `cohors agent prompts`.
fn prompts(as_json: bool) -> Result<()> {
    let human = super::wants_report(as_json);
    if !human {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "prompts": PROMPTS }))?
        );
        return Ok(());
    }
    let color = wants_color(true);
    let mut out = String::from("\n");
    out.push_str(&section(
        "prompts",
        "start one with `cohors agent <prompt>`",
        color,
    ));
    let width = PROMPTS.iter().map(|p| p.id.len()).max().unwrap_or(0);
    for p in &PROMPTS {
        out.push_str(&guide_row(p.id, p.summary, width, color));
    }
    out.push('\n');
    print!("{out}");
    Ok(())
}

/// Map the CLI's `<what>` and flags to the launch the adapter understands.
fn launch_kind(launch: &AgentLaunch) -> Result<LaunchKind> {
    let need = |flag: &str, value: &Option<String>| -> Result<String> {
        value
            .clone()
            .with_context(|| format!("`{}` needs --{flag}", launch.what))
    };
    Ok(match launch.what.as_str() {
        "agent" => LaunchKind::Agent(need("agent", &launch.agent)?),
        "task" => LaunchKind::Task(need("task", &launch.task)?),
        "start" => LaunchKind::Start,
        "resume" => LaunchKind::Resume(need("session", &launch.session)?),
        prompt => LaunchKind::Prompt(prompt.to_string()),
    })
}

/// Find the repository a launch is about: `--repo` as a path, a repo name or
/// an id; otherwise the repository containing the current directory. Returns
/// its display name, root and branch.
fn resolve_repo(
    wanted: Option<&str>,
    fleet: &[RepoSnapshot],
) -> Result<Option<(String, PathBuf, String)>> {
    let from_snapshot = |s: &RepoSnapshot| {
        s.path.as_ref().map(|p| {
            let branch = match &s.branch {
                cohors_core::Branch::Named(name) => name.clone(),
                _ => String::new(),
            };
            (s.name.clone(), p.as_std_path().to_path_buf(), branch)
        })
    };
    let start = match wanted {
        // A name or id from the fleet wins over a same-named relative path.
        Some(key) => {
            if let Some(found) = fleet
                .iter()
                .find(|s| s.name == key || s.id.0 == key)
                .and_then(from_snapshot)
            {
                return Ok(Some(found));
            }
            let path = PathBuf::from(key);
            if !path.is_dir() {
                bail!("no repository `{key}`: not a repo name in the fleet and not a directory");
            }
            path
        }
        None => std::env::current_dir().context("reading the current directory")?,
    };
    let Some(root) = repo_root(&start) else {
        // An explicit --repo that is not a git repo is an error; an implicit
        // current directory that is not one just means "no repo".
        if wanted.is_some() {
            bail!("{} is not inside a git repository", start.display());
        }
        return Ok(None);
    };
    // Prefer the fleet's view of it (alias, branch); fall back to the folder.
    let canonical = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
    let known = fleet.iter().find(|s| {
        s.path
            .as_ref()
            .is_some_and(|p| p.as_std_path() == root || p.as_std_path() == canonical)
    });
    Ok(Some(known.and_then(from_snapshot).unwrap_or_else(|| {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let branch = head_branch(&root);
        (name, root, branch)
    })))
}

/// Build the launch plan for `launch` (the 0.7 form).
fn plan(cli: &Cli, launch: &AgentLaunch) -> Result<LaunchPlan> {
    let kind = launch_kind(launch)?;
    let fleet = crate::scan::from_cli(cli)?.scan();
    let repo = resolve_repo(launch.repo.as_deref(), &fleet)?;
    plan_for(kind, launch.runner.as_deref(), repo, &fleet)
}

/// The launch plan for `kind` with `runner` in `repo`.
fn plan_for(
    kind: LaunchKind,
    runner: Option<&str>,
    repo: Option<(String, PathBuf, String)>,
    fleet: &[RepoSnapshot],
) -> Result<LaunchPlan> {
    let home = lp::launchpad_home().context("no home directory for the launchpad")?;
    let listing: Vec<(String, String)> = fleet
        .iter()
        .filter_map(|s| Some((s.name.clone(), s.path.as_ref()?.to_string())))
        .collect();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let request = LaunchRequest {
        kind,
        runner,
        repo: repo
            .as_ref()
            .map(|(name, root, branch)| RepoTarget { name, root, branch }),
        fleet: &listing,
        fallback_cwd: &cwd,
    };
    Ok(lp::plan_launch(
        &home,
        &request,
        &lp::detect_runners(),
        now_secs(),
    )?)
}

/// `cohors agent [WHAT] [--resume [ID|N]]`: work out what to start, then
/// start it (or print it, with `--print`).
fn launch_new(cli: &Cli, args: &AgentArgs) -> Result<()> {
    let fleet = crate::scan::from_cli(cli)?.scan();
    let repo = resolve_repo(args.repo.as_deref(), &fleet)?;
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let mut runner = args.runner.clone();

    let kind = if let Some(wanted) = &args.resume {
        let session = pick_session(&fleet, repo.as_ref(), wanted, interactive)?;
        // The runner that owns the session reopens it, unless one was named.
        if runner.is_none() {
            runner = Some(session.runner.clone());
        }
        LaunchKind::Resume(session.id)
    } else {
        match &args.what {
            Some(what) => detect_kind(what, args, repo.as_ref())?,
            // Bare: at a terminal, offer the recent sessions here first.
            None if interactive && !args.print => match offer_resume(&fleet, repo.as_ref())? {
                Offer::Resume(session) => {
                    if runner.is_none() {
                        runner = Some(session.runner.clone());
                    }
                    LaunchKind::Resume(session.id)
                }
                Offer::New => LaunchKind::Start,
                Offer::Quit => return Ok(()),
            },
            None => LaunchKind::Start,
        }
    };
    let plan = plan_for(kind, runner.as_deref(), repo, &fleet)?;
    if args.print {
        println!("{}", serde_json::to_string_pretty(&plan.to_json())?);
        return Ok(());
    }
    start(&plan)
}

/// What WHAT names: a shipped prompt, a task in the repo, or one of the repo's
/// agents. A flag (`--task`, `--agent`, `--prompt`) settles it outright; a name
/// that fits several kinds asks for one; a name that fits none says what does.
fn detect_kind(
    what: &str,
    args: &AgentArgs,
    repo: Option<&(String, PathBuf, String)>,
) -> Result<LaunchKind> {
    if args.task {
        return Ok(LaunchKind::Task(what.to_string()));
    }
    if args.agent {
        return Ok(LaunchKind::Agent(what.to_string()));
    }
    if args.prompt {
        return Ok(LaunchKind::Prompt(what.to_string()));
    }
    // The 0.7 keyword for a plain session still means that.
    if what == "start" {
        return Ok(LaunchKind::Start);
    }
    let (tasks, agents) = repo_offers(repo);
    let is_prompt = PROMPTS.iter().any(|p| p.id == what);
    let is_task = tasks.iter().any(|(id, _)| id == what);
    let is_agent = agents.iter().any(|(id, _)| id == what);
    let mut kinds: Vec<&str> = Vec::new();
    if is_prompt {
        kinds.push("a shipped prompt (--prompt)");
    }
    if is_task {
        kinds.push("a task (--task)");
    }
    if is_agent {
        kinds.push("one of the repo's agents (--agent)");
    }
    match kinds.len() {
        1 if is_prompt => Ok(LaunchKind::Prompt(what.to_string())),
        1 if is_task => Ok(LaunchKind::Task(what.to_string())),
        1 => Ok(LaunchKind::Agent(what.to_string())),
        0 => {
            let prompts: Vec<&str> = PROMPTS.iter().map(|p| p.id).collect();
            let place = repo
                .map(|(name, ..)| format!("a task or agent in {name}"))
                .unwrap_or_else(|| "a task or agent (no repo here: pass -r)".to_string());
            bail!(
                "`{what}` is not a shipped prompt ({}) or {place}.\n  See what you can start: cohors agent ls",
                prompts.join(", ")
            )
        }
        _ => bail!(
            "`{what}` could be {}; say which with that flag",
            kinds.join(" or ")
        ),
    }
}

/// An id and its title (a task's) or label (an agent's).
type IdTitle = (String, String);

/// The tasks and agents `repo` declares, as `(id, title)`.
fn repo_offers(repo: Option<&(String, PathBuf, String)>) -> (Vec<IdTitle>, Vec<IdTitle>) {
    let Some((_, root, _)) = repo else {
        return (Vec::new(), Vec::new());
    };
    match lp::read_contribution(root) {
        lp::ContributionState::Ok(doc) => (
            doc.tasks
                .iter()
                .map(|t| (t.id.clone(), t.title.clone()))
                .collect(),
            doc.agents
                .iter()
                .map(|a| (a.id.clone(), a.label.clone()))
                .collect(),
        ),
        _ => (Vec::new(), Vec::new()),
    }
}

/// The recent agent sessions worth resuming, most likely first: in `repo`
/// when there is one, else across the fleet.
fn ranked_sessions(
    fleet: &[RepoSnapshot],
    repo: Option<&(String, PathBuf, String)>,
) -> Vec<cohors_core::suggest::AgentSession> {
    let now = now_secs();
    let Ok(home) = cohors_config::paths::home_dir() else {
        return Vec::new();
    };
    let sessions = cohors_fleet::agent_sessions(fleet, home.as_std_path(), now);
    let overview = cohors_fleet::overview(fleet, lp::launchpad_home().as_deref(), now);
    let here = repo.and_then(|(_, root, _)| cohors_fleet::repo_containing(fleet, root));
    cohors_core::suggest::rank_sessions(&sessions, &overview, here.as_deref(), here.is_some(), now)
        .into_iter()
        .map(|(s, _)| s.clone())
        .collect()
}

/// One line per session for a pick list: number, title, runner, age, branch.
fn session_lines(sessions: &[cohors_core::suggest::AgentSession], color: bool) -> String {
    let now = now_secs();
    let width = sessions
        .iter()
        .map(|s| s.title.chars().count().min(56))
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for (i, s) in sessions.iter().enumerate() {
        let title = super::clip(&crate::ui::text::safe(&s.title), 56);
        let mut why = vec![
            s.runner.clone(),
            cohors_core::time::relative(s.last_active, now),
        ];
        if let Some(branch) = &s.branch {
            why.push(crate::ui::text::safe(branch));
        }
        out.push_str(&format!(
            "    {}  {}  {}\n",
            bold(color, &format!("{:>2}", i + 1)),
            format_args!("{title:<width$}"),
            dim(color, &why.join(" · "))
        ));
    }
    out
}

/// The session `--resume` names: an id, a number in the ranked list, or
/// (empty) one the person picks at a terminal.
fn pick_session(
    fleet: &[RepoSnapshot],
    repo: Option<&(String, PathBuf, String)>,
    wanted: &str,
    interactive: bool,
) -> Result<cohors_core::suggest::AgentSession> {
    let ranked = ranked_sessions(fleet, repo);
    let wanted = wanted.trim();
    if let Ok(n) = wanted.parse::<usize>()
        && n >= 1
        && n <= ranked.len()
        && wanted.len() <= 3
    {
        return Ok(ranked[n - 1].clone());
    }
    if !wanted.is_empty() {
        // An id: the owning runner comes from the stores, when found there.
        let all = cohors_config::paths::home_dir()
            .map(|h| cohors_fleet::agent_sessions(fleet, h.as_std_path(), now_secs()))
            .unwrap_or_default();
        if let Some(found) = all.into_iter().find(|s| s.id == wanted) {
            return Ok(found);
        }
        return Ok(cohors_core::suggest::AgentSession {
            runner: "claude-code".to_string(),
            id: wanted.to_string(),
            repo: String::new(),
            cwd: String::new(),
            branch: None,
            title: String::new(),
            last_active: 0,
        });
    }
    if ranked.is_empty() {
        bail!(
            "no recent agent sessions {} (Claude Code and Codex sessions from the last 30 days are found)",
            repo.map(|(n, ..)| format!("in {n}"))
                .unwrap_or_else(|| "in your fleet".to_string())
        );
    }
    let shown = &ranked[..ranked.len().min(9)];
    let color = wants_color(std::io::stdout().is_terminal());
    print!(
        "\n{}",
        section(
            "resume a session",
            &format!(
                "{}, most likely first",
                repo.map(|(n, ..)| n.clone())
                    .unwrap_or_else(|| "your fleet".to_string())
            ),
            color
        )
    );
    print!("{}", session_lines(shown, color));
    if !interactive {
        println!("\n  pick one: cohors agent --resume <N|ID>\n");
        return Err(super::ExitCode(2).into());
    }
    print!(
        "\n{}",
        super::key_bar(
            &[("enter", "resume 1"), ("1-9", "pick"), ("q", "quit")],
            color
        )
    );
    match read_choice(shown.len())? {
        Choice::Enter => Ok(shown[0].clone()),
        Choice::Pick(n) => Ok(shown[n - 1].clone()),
        Choice::New | Choice::Quit => Err(super::ExitCode(0).into()),
    }
}

/// What a bare `cohors agent` at a terminal should do.
enum Offer {
    Resume(cohors_core::suggest::AgentSession),
    New,
    Quit,
}

/// Bare `cohors agent` at a terminal: when there are recent sessions here,
/// list them and let one key decide (Enter = a new session, the fast path).
fn offer_resume(fleet: &[RepoSnapshot], repo: Option<&(String, PathBuf, String)>) -> Result<Offer> {
    let Some(repo) = repo else {
        return Ok(Offer::New);
    };
    let ranked = ranked_sessions(fleet, Some(repo));
    if ranked.is_empty() {
        return Ok(Offer::New);
    }
    let shown = &ranked[..ranked.len().min(5)];
    let color = wants_color(true);
    print!(
        "\n{}",
        section(
            "recent sessions here",
            &format!("{}, most likely first", repo.0),
            color
        )
    );
    print!("{}", session_lines(shown, color));
    print!(
        "\n{}",
        super::key_bar(
            &[("enter", "new session"), ("1-5", "resume"), ("q", "quit")],
            color
        )
    );
    Ok(match read_choice(shown.len())? {
        Choice::Pick(n) => Offer::Resume(shown[n - 1].clone()),
        Choice::Enter | Choice::New => Offer::New,
        Choice::Quit => Offer::Quit,
    })
}

enum Choice {
    Enter,
    Pick(usize),
    New,
    Quit,
}

/// One key, in raw mode: Enter, a digit within `max`, `n`, or q/Esc/Ctrl-C.
fn read_choice(max: usize) -> Result<Choice> {
    use ratatui::crossterm::event::{self, Event, KeyCode, KeyModifiers};
    use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode};
    enable_raw_mode().context("entering raw mode for the pick list")?;
    let choice = loop {
        match event::read() {
            Ok(Event::Key(k)) => match k.code {
                KeyCode::Enter => break Choice::Enter,
                KeyCode::Char('n') | KeyCode::Char('N') => break Choice::New,
                KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => break Choice::Quit,
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    break Choice::Quit;
                }
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    let n = c.to_digit(10).unwrap_or(0) as usize;
                    if n >= 1 && n <= max {
                        break Choice::Pick(n);
                    }
                }
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break Choice::Quit,
        }
    };
    let _ = disable_raw_mode();
    println!();
    Ok(choice)
}

/// `cohors agent ls`: runners (the default marked), prompts, and what the repo
/// here declares and has recently worked on.
fn ls(cli: &Cli, repo_arg: Option<&str>, as_json: bool) -> Result<()> {
    let statuses = lp::detect_runners();
    let default = statuses.iter().find(|s| s.available).map(|s| s.id.clone());
    let fleet = crate::scan::from_cli(cli)?.scan();
    let repo = resolve_repo(repo_arg, &fleet).ok().flatten();
    let (tasks, agents) = repo_offers(repo.as_ref());
    let sessions = ranked_sessions(&fleet, repo.as_ref());
    let human = super::wants_report(as_json);
    if !human {
        let out = json!({
            "default_runner": default,
            "runners": statuses,
            "prompts": PROMPTS,
            "repo": repo.as_ref().map(|(name, root, _)| json!({
                "name": name,
                "path": root,
                "tasks": tasks.iter().map(|(id, title)| json!({"id": id, "title": title})).collect::<Vec<_>>(),
                "agents": agents.iter().map(|(id, label)| json!({"id": id, "label": label})).collect::<Vec<_>>(),
            })),
            "sessions": sessions.iter().take(10).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    let color = wants_color(true);
    let mut out = runners_report(&statuses, color);
    if let Some(id) = &default {
        out.push_str(&format!(
            "  {}\n\n",
            dim(color, &format!("{id} is used unless you pass -w <runner>"))
        ));
    }
    let width = PROMPTS.iter().map(|p| p.id.len()).max().unwrap_or(0);
    out.push_str(&section("prompts", "cohors agent <prompt>", color));
    for p in &PROMPTS {
        out.push_str(&guide_row(p.id, p.summary, width, color));
    }
    out.push('\n');
    if let Some((name, ..)) = &repo {
        if !tasks.is_empty() {
            out.push_str(&section(
                &format!("tasks in {name}"),
                "cohors agent <task>",
                color,
            ));
            let w = tasks.iter().map(|(id, _)| id.len()).max().unwrap_or(0);
            for (id, title) in &tasks {
                out.push_str(&guide_row(id, &crate::ui::text::safe(title), w, color));
            }
            out.push('\n');
        }
        if !agents.is_empty() {
            out.push_str(&section(
                &format!("agents in {name}"),
                "cohors agent <agent>",
                color,
            ));
            let w = agents.iter().map(|(id, _)| id.len()).max().unwrap_or(0);
            for (id, label) in &agents {
                out.push_str(&guide_row(id, &crate::ui::text::safe(label), w, color));
            }
            out.push('\n');
        }
    }
    if !sessions.is_empty() {
        out.push_str(&section(
            "recent sessions",
            "cohors agent --resume <N>",
            color,
        ));
        out.push_str(&session_lines(&sessions[..sessions.len().min(5)], color));
        out.push('\n');
    }
    print!("{out}");
    Ok(())
}

/// Run the plan in this terminal: spawn the runner directly (no shell), hand
/// it the terminal, and wait. The exit code is the runner's.
fn start(plan: &LaunchPlan) -> Result<()> {
    let color = wants_color(std::io::stderr().is_terminal());
    eprintln!("  {} {}", dim(color, "starting"), bold(color, &plan.label),);
    eprintln!("  {}", dim(color, &display_dir(&plan.cwd)));
    let status = std::process::Command::new(&plan.program)
        .args(&plan.args)
        .current_dir(&plan.cwd)
        .status()
        .with_context(|| format!("starting {}", plan.program))?;
    if !status.success() {
        // Mirror the runner's failure rather than reporting success.
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

fn display_dir(path: &Path) -> String {
    crate::ui::abbrev_home(&path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cohors_core::agents::RUNNERS;

    fn launch(what: &str) -> AgentLaunch {
        AgentLaunch {
            what: what.to_string(),
            runner: None,
            repo: None,
            agent: None,
            task: None,
            session: None,
        }
    }

    #[test]
    fn what_maps_to_a_launch_kind_and_names_a_missing_flag() {
        assert_eq!(
            launch_kind(&launch("setup-repo")).unwrap(),
            LaunchKind::Prompt("setup-repo".to_string())
        );
        assert_eq!(launch_kind(&launch("start")).unwrap(), LaunchKind::Start);
        let err = launch_kind(&launch("task")).unwrap_err().to_string();
        assert!(err.contains("--task"), "{err}");
        let err = launch_kind(&launch("resume")).unwrap_err().to_string();
        assert!(err.contains("--session"), "{err}");
        let mut with_agent = launch("agent");
        with_agent.agent = Some("triage".to_string());
        assert_eq!(
            launch_kind(&with_agent).unwrap(),
            LaunchKind::Agent("triage".to_string())
        );
    }

    #[test]
    fn repo_resolves_by_fleet_name_then_by_path() {
        let now = 1_700_000_000;
        let fleet = cohors_core::demo::fleet(now);
        let first = &fleet[0];
        // By name: the fleet's own name, path and branch come back.
        let (name, root, _) = resolve_repo(Some(&first.name), &fleet).unwrap().unwrap();
        assert_eq!(name, first.name);
        assert_eq!(root, first.path.as_ref().unwrap().as_std_path());
        // An unknown name that is not a directory is an error that says so.
        let err = resolve_repo(Some("no-such-repo-anywhere"), &fleet).unwrap_err();
        assert!(err.to_string().contains("not a repo name"));

        // By path: a real git folder outside the fleet uses its folder name.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/feat/x\n").unwrap();
        let (name, root, branch) = resolve_repo(dir.path().to_str(), &fleet).unwrap().unwrap();
        assert_eq!(root, dir.path());
        assert_eq!(branch, "feat/x");
        assert_eq!(name, dir.path().file_name().unwrap().to_string_lossy());
        // A directory that is not a repo is refused when named explicitly.
        let plain = tempfile::tempdir().unwrap();
        assert!(resolve_repo(plain.path().to_str(), &fleet).is_err());
    }

    #[test]
    fn runner_report_marks_installed_and_explains_missing() {
        let statuses: Vec<RunnerStatus> = RUNNERS
            .iter()
            .enumerate()
            .map(|(i, r)| RunnerStatus::new(r, (i == 0).then(|| "/bin/claude".to_string())))
            .collect();
        let plain = runners_report(&statuses, false);
        assert!(plain.contains("1 of 4 installed"));
        assert!(plain.contains("● Claude Code") && plain.contains("/bin/claude"));
        assert!(plain.contains("○ Codex") && plain.contains("codex was not found on your PATH"));
        assert!(!plain.contains('\x1b'));
        let none: Vec<RunnerStatus> = RUNNERS.iter().map(|r| RunnerStatus::new(r, None)).collect();
        assert!(runners_report(&none, false).contains("install any one of these"));
    }
}
