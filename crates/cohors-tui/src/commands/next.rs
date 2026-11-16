//! `cohors next` — what to do now, with the command for each (ADR-062).
//!
//! For someone unsure where they left off, or who would rather not type: the
//! agent sessions held in each repo (which to resume), the fleet's "do next"
//! steps and the focus board, ranked by `cohors_core::suggest` from the repo
//! you are in. Bare, it lists them numbered; `cohors next N` runs the Nth;
//! `--json` gives the same list to scripts and agents (the MCP `next` tool
//! returns the same thing).

use std::io::IsTerminal;

use anyhow::Result;
use cohors_core::suggest::{Suggestion, SuggestionKind};

use super::{ExitCode, bold, brand, dim, masthead, now_secs, plural, wants_color};
use crate::cli::Cli;

pub fn run(cli: &Cli, pick: Option<usize>, limit: usize, json: bool) -> Result<()> {
    let scanner = crate::scan::from_cli(cli)?;
    let fleet = scanner.scan();
    let now = now_secs();
    let home = cohors_config::paths::home_dir().ok();
    let here = std::env::current_dir()
        .ok()
        .and_then(|d| cohors_fleet::repo_containing(&fleet, &d));
    let found = cohors_fleet::suggestions(
        &fleet,
        here.as_deref(),
        home.as_deref().map(|h| h.as_std_path()),
        cohors_fleet::launchpad::launchpad_home().as_deref(),
        now,
        limit.max(pick.unwrap_or(0)).max(1),
    );

    if let Some(n) = pick {
        let Some(item) = n.checked_sub(1).and_then(|i| found.list.get(i)) else {
            eprintln!(
                "cohors: there {} {}; `cohors next` lists them",
                if found.list.len() == 1 { "is" } else { "are" },
                plural(found.list.len(), "suggestion", "suggestions")
            );
            return Err(ExitCode(2).into());
        };
        return run_suggestion(item);
    }

    let human = !json && std::io::stdout().is_terminal();
    if !human {
        let out = serde_json::json!({
            "here": here,
            "suggestions": found.list,
            "sessions": found.sessions.iter().take(20).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    let color = wants_color(true);
    let here_name = here
        .as_ref()
        .and_then(|id| found.overview.repos.iter().find(|r| &r.id == id))
        .map(|r| format!("from {}", r.name))
        .unwrap_or_else(|| "across your fleet".to_string());
    print!(
        "{}",
        report(&found.list, &found.overview.headline, &here_name, color)
    );
    Ok(())
}

/// The numbered list. Pure, so it is unit-testable.
fn report(list: &[Suggestion], headline: &str, here: &str, color: bool) -> String {
    let mut out = masthead(headline, &format!("what to do next, {here}"), color);
    for (i, s) in list.iter().enumerate() {
        let tag = match s.kind {
            SuggestionKind::Resume => "resume",
            SuggestionKind::Act => "do",
            SuggestionKind::Focus => "focus",
            SuggestionKind::Start => "start",
        };
        out.push_str(&format!(
            "  {}  {}  {}\n",
            bold(color, &format!("{:>2}", i + 1)),
            dim(color, &format!("{tag:<6}")),
            bold(color, &crate::ui::text::safe(&s.title)),
        ));
        out.push_str(&format!(
            "              {}\n",
            dim(color, &crate::ui::text::safe(&s.why))
        ));
        out.push_str(&format!(
            "              {}\n",
            brand(color, &crate::ui::text::safe(&s.command))
        ));
    }
    out.push('\n');
    out.push_str(&super::key_bar(
        &[
            ("cohors next <N>", "do one"),
            ("--json", "for scripts and agents"),
        ],
        color,
    ));
    out
}

/// Run a suggestion: this same cohors binary with the suggestion's
/// arguments, directly (no shell, so nothing in a title, id or repo name can
/// be read as shell syntax). The exit status is the command's.
fn run_suggestion(item: &Suggestion) -> Result<()> {
    let color = wants_color(std::io::stderr().is_terminal());
    eprintln!(
        "  {} {}",
        dim(color, "running"),
        bold(color, &crate::ui::text::safe(&item.command))
    );
    let exe = std::env::current_exe()?;
    let status = std::process::Command::new(exe).args(&item.args).status()?;
    if !status.success() {
        return Err(ExitCode(status.code().unwrap_or(1)).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_numbered_with_a_command_under_each() {
        let now = 1_700_000_000;
        let fleet = cohors_core::demo::fleet(now);
        let o = cohors_core::overview(&fleet, &[], &Default::default(), now);
        let sessions = vec![cohors_core::suggest::AgentSession {
            runner: "claude-code".into(),
            id: "3f2a9c10-1111-4222-8333-444455556666".into(),
            repo: o.repos[0].id.clone(),
            cwd: o.repos[0].id.clone(),
            branch: Some("main".into()),
            title: "Back off on \x1b[2Jrate limits".into(),
            last_active: now - 600,
        }];
        let list = cohors_core::suggest::suggest(&o, &sessions, Some(&o.repos[0].id), now, 4);
        let r = report(&list, &o.headline, "from here", false);
        assert!(r.contains(" 1  resume  Resume"), "{r}");
        assert!(
            r.contains("cohors agent --resume 3f2a9c10-1111-4222-8333-444455556666"),
            "{r}"
        );
        assert!(r.contains(" 2  "), "{r}");
        assert!(
            !r.contains('\x1b'),
            "control characters never reach the terminal"
        );
        assert!(r.contains("cohors next <N>"));
    }
}
