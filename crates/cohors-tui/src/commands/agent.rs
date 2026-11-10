//! `cohors agent` — start a coding agent on a repo, a task, or the whole fleet.
//!
//! cohors is built to be used *with* agents. These commands are the one place
//! every surface (this CLI, the dashboard, the editor extension) gets an
//! agent started, so they all behave identically:
//!
//! - `cohors agent runners` — which agent tools are installed, and why one is not.
//! - `cohors agent prompts` — the prompts cohors ships.
//! - `cohors agent run <what>` — start an agent in this terminal.
//! - `cohors agent command <what>` — print, as JSON, the command that would
//!   start it. An editor reads this and types the command into its own
//!   terminal.
//!
//! `<what>` is a shipped prompt (`setup-repo`, `declare-flows`, `add-agent`,
//! `track-tasks`, `plan-day`) or one of `agent` (a repo-declared agent, with
//! `--agent`), `task` (a repo task, with `--task`; it resumes the task's
//! conversation when there is one), `start` (just open a session in the repo)
//! and `resume` (re-enter a session, with `--session`).
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
use crate::cli::{AgentCmd, AgentLaunch, Cli};

pub fn run(cli: &Cli, action: &AgentCmd) -> Result<()> {
    match action {
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
    }
}

/// `cohors agent runners`.
fn runners(as_json: bool) -> Result<()> {
    let statuses = lp::detect_runners();
    let human = !as_json && std::io::stdout().is_terminal();
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
                "install any one of these, then run `cohors agent run setup-repo` in a repo"
            )
        ));
    }
    out
}

/// `cohors agent prompts`.
fn prompts(as_json: bool) -> Result<()> {
    let human = !as_json && std::io::stdout().is_terminal();
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
        "start one with `cohors agent run <prompt>`",
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

/// Build the launch plan for `launch`.
fn plan(cli: &Cli, launch: &AgentLaunch) -> Result<LaunchPlan> {
    let home = lp::launchpad_home().context("no home directory for the launchpad")?;
    let kind = launch_kind(launch)?;
    let fleet = crate::scan::from_cli(cli)?.scan();
    let repo = resolve_repo(launch.repo.as_deref(), &fleet)?;
    let listing: Vec<(String, String)> = fleet
        .iter()
        .filter_map(|s| Some((s.name.clone(), s.path.as_ref()?.to_string())))
        .collect();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let request = LaunchRequest {
        kind,
        runner: launch.runner.as_deref(),
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
