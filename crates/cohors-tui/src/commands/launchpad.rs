//! `cohors launchpad` — the launchpad from the command line.
//!
//! - `cohors launchpad` reports what is waiting across the fleet: the focus
//!   board, every repo's open tasks, and which repos have not opted in yet.
//! - `cohors launchpad validate [PATH]` checks one repo's `.cohors/repo.json`.
//! - `cohors launchpad contract` prints the contract an agent must satisfy.
//! - `cohors launchpad record` appends an activity event; the Claude Code
//!   plugin's session hooks call it.
//!
//! Like `scan`, the report is styled for a human at a terminal and plain JSON
//! when piped or given `--json`.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use cohors_core::SortMode;
use cohors_core::launchpad::{ActivityEvent, LaunchpadEntry, Task};
use cohors_fleet::launchpad as lp;
use serde_json::json;

use super::{bold, brand, clip, dim, guide_row, masthead, now_secs, parse_selector, section};
use crate::cli::{Cli, LaunchpadCmd};

pub fn run(
    cli: &Cli,
    action: Option<&LaunchpadCmd>,
    select: Option<&str>,
    json: bool,
) -> Result<()> {
    match action {
        None => report(cli, select, json),
        Some(LaunchpadCmd::Validate { path }) => validate(path.as_deref()),
        Some(LaunchpadCmd::Contract) => {
            let contract = cohors_core::launchpad::contract();
            println!("{}", serde_json::to_string_pretty(&contract)?);
            Ok(())
        }
        Some(LaunchpadCmd::Record {
            phase,
            runner,
            agent,
            repo,
        }) => {
            // Called from session hooks: a failure here must never get in the
            // way of the agent session, so it is logged and swallowed.
            if let Err(e) = record(phase, runner, agent.as_deref(), repo.as_deref()) {
                tracing::warn!("launchpad record failed: {e:#}");
            }
            Ok(())
        }
    }
}

fn report(cli: &Cli, select: Option<&str>, json: bool) -> Result<()> {
    let human = !json && std::io::stdout().is_terminal();
    let color = super::wants_color(human);
    let scanner = crate::scan::from_cli(cli)?;
    let snapshots = scanner.scan();
    let now = now_secs();

    // Without a selector the launchpad covers the whole fleet (it only reads).
    let selector = match select {
        Some(query) => parse_selector(query)?,
        None => cohors_core::Selector {
            all: true,
            ..Default::default()
        },
    };
    let order = cohors_core::resolve(&snapshots, &selector, SortMode::Name, now);
    let selected: Vec<cohors_core::RepoSnapshot> = order
        .iter()
        .filter_map(|id| snapshots.iter().find(|s| &s.id == id).cloned())
        .collect();

    // The shared read every surface uses (entries sorted by name, the focus
    // board, recent activity), so this report can't drift from the dashboard.
    let home = lp::launchpad_home();
    let bundle = cohors_fleet::launchpad_bundle(&selected, home.as_deref());
    let (entries, board, activity) = (bundle.entries, bundle.board, bundle.recent_activity);

    if !human {
        let out = json!({
            "summary": summary_counts(&entries),
            "repos": entries,
            "board": board,
            "recent_activity": activity,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    print!(
        "{}",
        render_report(&entries, &board, super::term_width(), color)
    );
    Ok(())
}

fn summary_counts(entries: &[LaunchpadEntry]) -> serde_json::Value {
    json!({
        "repos": entries.len(),
        "configured": entries.iter().filter(|e| e.configured).count(),
        "open_tasks": entries.iter().map(|e| e.open_tasks().len()).sum::<usize>(),
    })
}

/// Drop control characters (ESC included) from text that came out of a repo's
/// files. A task title or repo name is untrusted, and printed raw it could
/// carry terminal escape sequences that recolour, rewrite, or hide output.
fn safe(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// A task or focus item's status as a shape, so it reads without colour:
/// `◆` doing, `✕` blocked, `○` to do.
fn status_mark(status: &str) -> &'static str {
    match status {
        "doing" => "◆",
        "blocked" => "✕",
        _ => "○",
    }
}

/// One task line: a status mark, the title, and a priority tag when it is not
/// the default. Shape carries the status so it still reads without colour.
fn task_row(task: &Task, width: usize, color: bool) -> String {
    let mark = status_mark(task.status());
    // indent + mark, then room for the "  high" / "  blocked" tags after it.
    let title_w = width.saturating_sub(6 + 2 + 18).max(16);
    let mut line = format!("      {} {}", mark, clip(&safe(&task.title), title_w));
    match task.priority.as_deref() {
        Some("high") => line.push_str(&format!("  {}", bold(color, "high"))),
        Some("low") => line.push_str(&format!("  {}", dim(color, "low"))),
        _ => {}
    }
    if task.status() != "todo" {
        line.push_str(&format!("  {}", dim(color, &safe(task.status()))));
    }
    line.push('\n');
    line
}

/// The human report. Pure (data in, string out) so it can be snapshot-tested.
pub(crate) fn render_report(
    entries: &[LaunchpadEntry],
    board: &cohors_core::launchpad::Board,
    width: usize,
    color: bool,
) -> String {
    let configured = entries.iter().filter(|e| e.configured).count();
    let open: usize = entries.iter().map(|e| e.open_tasks().len()).sum();
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut out = masthead(
        "launchpad: flows, agents and tasks across your repos",
        &format!(
            "{}, {configured} set up, {}",
            plural(entries.len(), "repository", "repositories"),
            plural(open, "open task", "open tasks")
        ),
        color,
    );

    let focus: Vec<_> = board
        .focus
        .iter()
        .filter(|f| f.status.as_deref() != Some("done"))
        .collect();
    if !focus.is_empty() {
        out.push_str(&section(
            "focus",
            "your cross-repo board, the short list you chose",
            color,
        ));
        // Status as the same shape the task rows use; then the title; then
        // which repo it lives in.
        let title_w = focus
            .iter()
            .map(|f| safe(&f.title).chars().count())
            .max()
            .unwrap_or(0)
            // indent + mark + gaps, leaving room for the repo name after it.
            .min(width.saturating_sub(4 + 2 + 2 + 24).max(16));
        for item in focus {
            let repo = safe(item.repo.as_deref().unwrap_or(""));
            out.push_str(&format!(
                "    {} {}  {}\n",
                status_mark(item.status.as_deref().unwrap_or("todo")),
                bold(
                    color,
                    &format!("{:<title_w$}", clip(&safe(&item.title), title_w))
                ),
                dim(color, &clip(&repo, 22)),
            ));
        }
        out.push('\n');
    }

    let with_tasks: Vec<&LaunchpadEntry> = entries
        .iter()
        .filter(|e| !e.open_tasks().is_empty())
        .collect();
    if !with_tasks.is_empty() {
        out.push_str(&section(
            "open tasks",
            "◆ doing, ✕ blocked, ○ to do; most urgent first",
            color,
        ));
        for entry in with_tasks {
            let tasks = entry.open_tasks();
            out.push_str(&format!(
                "    {} {}  {}\n",
                dim(color, "▸"),
                brand(color, &safe(&entry.name)),
                dim(color, &format!("{} open", tasks.len())),
            ));
            for task in tasks {
                out.push_str(&task_row(task, width, color));
            }
        }
        out.push('\n');
    }

    let broken: Vec<&LaunchpadEntry> = entries.iter().filter(|e| e.invalid.is_some()).collect();
    if !broken.is_empty() {
        out.push_str(&section(
            "needs fixing",
            "a .cohors/repo.json that could not be read",
            color,
        ));
        let width = broken
            .iter()
            .map(|e| e.name.chars().count())
            .max()
            .unwrap_or(0);
        for entry in broken {
            out.push_str(&guide_row(
                &safe(&entry.name),
                &safe(entry.invalid.as_deref().unwrap_or("")),
                width,
                color,
            ));
        }
        out.push('\n');
    }

    let unset: Vec<String> = entries
        .iter()
        .filter(|e| !e.configured && e.invalid.is_none())
        .map(|e| safe(&e.name))
        .collect();
    if !unset.is_empty() {
        out.push_str(&section(
            "not set up yet",
            &format!(
                "{} with no launchpad file",
                plural(unset.len(), "repository", "repositories")
            ),
            color,
        ));
        // Every name, wrapped: clipping the list would hide which repos they are.
        let mut line = String::new();
        for name in &unset {
            if !line.is_empty()
                && line.chars().count() + name.chars().count() + 2 > width.saturating_sub(6)
            {
                out.push_str(&format!("    {}\n", dim(color, &format!("{line},"))));
                line.clear();
            }
            if !line.is_empty() {
                line.push_str(", ");
            }
            line.push_str(name);
        }
        out.push_str(&format!("    {}\n", dim(color, &line)));
        // The command on its own line, its explanation under it, so neither
        // is cut on a narrow terminal.
        out.push_str(&format!(
            "    {} {}\n      {}\n",
            dim(color, "▸"),
            bold(color, "cohors agent run setup-repo"),
            dim(color, "run it inside one; your agent writes the file"),
        ));
        out.push('\n');
    }

    if entries.is_empty() {
        out.push_str(&section(
            "no repositories",
            "run `cohors init`, or pass --root <dir>",
            color,
        ));
        out.push('\n');
    }
    out
}

fn validate(path: Option<&str>) -> Result<()> {
    let root = PathBuf::from(path.unwrap_or("."));
    let color = super::wants_color(std::io::stdout().is_terminal());
    let Some(validation) = lp::validate_repo(&root) else {
        bail!(
            "no .cohors/repo.json in {} — this repo has not been set up for the launchpad",
            root.display()
        );
    };
    for warning in &validation.warnings {
        println!("  {} {warning}", dim(color, "warning"));
    }
    if validation.is_ok() {
        println!("  {} .cohors/repo.json is valid", bold(color, "ok"));
        return Ok(());
    }
    for error in &validation.errors {
        println!("  {} {error}", bold(color, "error"));
    }
    bail!(
        "{} problem(s) in .cohors/repo.json",
        validation.errors.len()
    );
}

/// Walk up from `start` to the directory holding `.git`.
pub(super) fn repo_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

/// The checked-out branch, read straight from `.git/HEAD` (no subprocess, so a
/// session hook stays fast). Empty when detached or unreadable.
pub(super) fn head_branch(root: &Path) -> String {
    std::fs::read_to_string(root.join(".git/HEAD"))
        .ok()
        .and_then(|head| {
            head.trim()
                .strip_prefix("ref: refs/heads/")
                .map(str::to_string)
        })
        .unwrap_or_default()
}

fn record(phase: &str, runner: &str, agent: Option<&str>, repo: Option<&str>) -> Result<()> {
    let home = lp::launchpad_home().context("no home directory for the launchpad")?;
    let start = match repo {
        Some(path) => PathBuf::from(path),
        None => std::env::current_dir().context("reading the current directory")?,
    };
    let root = repo_root(&start);
    let event = ActivityEvent {
        phase: phase.to_string(),
        runner: runner.to_string(),
        repo: root
            .as_deref()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        branch: root.as_deref().map(head_branch).unwrap_or_default(),
        agent: agent.unwrap_or_default().to_string(),
        source: "cohors-cli".to_string(),
        ..Default::default()
    };
    lp::record_activity(&home, event, now_secs())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cohors_core::launchpad::{Board, Contribution, FocusItem};

    fn task(id: &str, status: &str, priority: &str) -> Task {
        Task {
            id: id.into(),
            title: format!("Task {id}"),
            status: Some(status.into()),
            priority: Some(priority.into()),
            ..Task::default()
        }
    }

    fn fleet() -> Vec<LaunchpadEntry> {
        vec![
            LaunchpadEntry {
                id: "1".into(),
                name: "pay-api".into(),
                configured: true,
                contribution: Some(Contribution {
                    tasks: vec![
                        task("a", "todo", "low"),
                        task("b", "doing", "high"),
                        task("c", "done", "normal"),
                        task("d", "blocked", "normal"),
                    ],
                    ..Contribution::default()
                }),
                ..LaunchpadEntry::default()
            },
            LaunchpadEntry {
                id: "2".into(),
                name: "web".into(),
                ..LaunchpadEntry::default()
            },
            LaunchpadEntry {
                id: "3".into(),
                name: "infra".into(),
                invalid: Some("repo.json is not valid JSON".into()),
                ..LaunchpadEntry::default()
            },
        ]
    }

    #[test]
    fn report_renders_every_section_in_plain_text() {
        let board = Board {
            focus: vec![FocusItem {
                id: "f-1".into(),
                title: "Ship retries".into(),
                repo: Some("pay-api".into()),
                status: Some("doing".into()),
                ..FocusItem::default()
            }],
            ..Board::default()
        };
        insta::assert_snapshot!(render_report(&fleet(), &board, 100, false));
    }

    #[test]
    fn repo_authored_text_cannot_inject_terminal_escapes() {
        let mut entries = fleet();
        entries[0].name = "pay\x1b[2Japi".into();
        if let Some(c) = entries[0].contribution.as_mut() {
            c.tasks[1].title = "evil\x1b]0;pwned\x07title\r\n".into();
        }
        entries[2].invalid = Some("bad\x1b[31m".into());
        let board = Board {
            focus: vec![FocusItem {
                id: "f".into(),
                title: "x\x1b[0m".into(),
                repo: Some("r\x1b".into()),
                ..FocusItem::default()
            }],
            ..Board::default()
        };
        // Colour off, so any ESC in the output could only have come from data.
        let out = render_report(&entries, &board, 100, false);
        assert!(!out.contains('\x1b') && !out.contains('\x07') && !out.contains('\r'));
        assert!(out.contains("pay[2Japi"));
    }

    /// No line of the report is wider than the terminal, at any width.
    #[test]
    fn report_fits_the_terminal_width() {
        let board = Board {
            focus: vec![FocusItem {
                id: "f".into(),
                title: "A focus item with a deliberately long title that would not fit a narrow terminal".into(),
                repo: Some("pay-api".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        for width in [60, 80, 100, 140] {
            for line in render_report(&fleet(), &board, width, false).lines() {
                assert!(
                    line.chars().count() <= width,
                    "{} columns at width {width}: {line}",
                    line.chars().count()
                );
            }
        }
    }

    #[test]
    fn empty_fleet_says_how_to_start() {
        let out = render_report(&[], &Board::default(), 100, false);
        assert!(out.contains("No repositories"));
        assert!(out.contains("cohors init"));
    }

    #[test]
    fn record_finds_the_repo_and_branch() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("src/deep");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/feat/x\n").unwrap();
        let root = repo_root(&nested).unwrap();
        assert_eq!(root, dir.path());
        assert_eq!(head_branch(&root), "feat/x");
        // Detached HEAD has no branch.
        std::fs::write(dir.path().join(".git/HEAD"), "abc123\n").unwrap();
        assert_eq!(head_branch(&root), "");
    }
}
