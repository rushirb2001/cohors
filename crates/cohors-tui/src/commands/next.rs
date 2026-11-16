//! `cohors next` — what to do now, with the command for each (ADR-062).
//!
//! For someone unsure where they left off, or who would rather not type: the
//! agent sessions held in each repo (which to resume), the fleet's "do next"
//! steps and the focus board, ranked by `cohors_core::suggest` from the repo
//! you are in. Bare, it lists them numbered; `cohors next N` runs the Nth;
//! `--json` gives the same list to scripts and agents (the MCP `next` tool
//! returns the same thing).
//!
//! After the real work come at most two setup tips (ADR-065): parts of cohors
//! this machine is not using yet. They are numbered like the rest, so `cohors
//! next N` carries one out, but only when it is cohors itself; a tip for
//! another program (`code`, `claude`) is printed for you to run.
//! `--dismiss <id>` hides a tip for good.

use std::io::IsTerminal;

use anyhow::Result;
use cohors_core::suggest::{Suggestion, SuggestionKind, TIPS_AT_ONCE};
use cohors_fleet::tips;

use super::{ExitCode, bold, brand, dim, masthead, now_secs, plural, wants_color};
use crate::cli::Cli;

pub fn run(
    cli: &Cli,
    pick: Option<usize>,
    limit: usize,
    json: bool,
    dismiss: Option<&str>,
) -> Result<()> {
    if let Some(id) = dismiss {
        return dismiss_tip(id);
    }
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
        limit.max(1),
    );
    // The tips come after the real work, whatever it is: never above it.
    let env = tips::TipEnv::from_env();
    let shown_tips = env
        .as_ref()
        .map(|env| tips::current(env, now, TIPS_AT_ONCE))
        .unwrap_or_default();
    let mut list = found.list.clone();
    list.extend(shown_tips.iter().cloned());

    if let Some(n) = pick {
        // The number is the one `cohors next` printed with the same flags.
        let Some(item) = n.checked_sub(1).and_then(|i| list.get(i)) else {
            eprintln!(
                "cohors: there {} {}; `cohors next` lists them",
                if list.len() == 1 { "is" } else { "are" },
                plural(list.len(), "suggestion", "suggestions")
            );
            return Err(ExitCode(2).into());
        };
        return run_suggestion(item);
    }

    let human = !json && std::io::stdout().is_terminal();
    if !human {
        let out = serde_json::json!({
            "here": here,
            "suggestions": list,
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
        report(&list, &found.overview.headline, &here_name, color)
    );
    // Only a person reading the list counts as having seen a tip.
    if let Some(env) = &env {
        tips::note_shown(env, &shown_tips, now);
    }
    Ok(())
}

/// `cohors next --dismiss <id>`: hide one setup tip for good.
fn dismiss_tip(id: &str) -> Result<()> {
    let Some(env) = tips::TipEnv::from_env() else {
        eprintln!("cohors: tips are turned off here (COHORS_NO_TIPS), so there is none to hide");
        return Ok(());
    };
    match tips::dismiss(&env, id) {
        Ok(label) => {
            let color = wants_color(std::io::stdout().is_terminal());
            println!(
                "  {} {}",
                dim(color, "hidden for good:"),
                bold(color, &format!("the tip to {label}"))
            );
            Ok(())
        }
        Err(tips::DismissError::Unknown(_)) => {
            eprintln!(
                "cohors: there is no tip called {:?}; the tips are {}",
                crate::ui::text::safe(id),
                cohors_core::suggest::TIP_IDS.join(", ")
            );
            Err(ExitCode(2).into())
        }
        Err(e) => Err(e.into()),
    }
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
            SuggestionKind::Setup => "tip",
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
        // A tip says how to be rid of it, so it never has to be put up with.
        if let Some(id) = &s.tip {
            out.push_str(&format!(
                "              {}\n",
                dim(
                    color,
                    &format!("cohors next --dismiss {id}  hides this tip")
                )
            ));
        }
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
///
/// Only cohors itself is ever run. A tip for another program (`code
/// --install-extension …`, `claude mcp add …`) is printed for the person to
/// run: cohors does not install things into other tools on its own.
fn run_suggestion(item: &Suggestion) -> Result<()> {
    if !item.runs_cohors() {
        eprint!(
            "{}",
            not_run(item, wants_color(std::io::stderr().is_terminal()))
        );
        return Err(ExitCode(2).into());
    }
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

/// Why a suggestion for another program was not run, and its command to run
/// by hand. Pure, so it is unit-testable.
fn not_run(item: &Suggestion, color: bool) -> String {
    format!(
        "  {}\n  {}\n\n    {}\n\n",
        bold(
            color,
            &format!(
                "cohors next only runs cohors itself; this one is for `{}`.",
                crate::ui::text::safe(&item.program)
            )
        ),
        dim(color, "Run it yourself if you want it:"),
        brand(color, &crate::ui::text::safe(&item.command)),
    )
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

    #[test]
    fn tips_come_last_say_how_to_hide_them_and_another_program_is_never_run() {
        use cohors_core::suggest::{EditorFacts, SetupFacts, setup_tips};
        let now = 1_700_000_000;
        let o = cohors_core::overview(
            &cohors_core::demo::fleet(now),
            &[],
            &Default::default(),
            now,
        );
        let mut list = cohors_core::suggest::suggest(&o, &[], None, now, 3);
        let facts = SetupFacts {
            editors: vec![EditorFacts {
                name: "VS Code".into(),
                cli: "code".into(),
                has_extension: false,
            }],
            ..Default::default()
        };
        list.extend(setup_tips(&facts));
        let r = report(&list, &o.headline, "across your fleet", false);
        assert!(
            r.contains(" 4  tip     Open your fleet in the browser"),
            "{r}"
        );
        assert!(
            r.contains("cohors next --dismiss web  hides this tip"),
            "{r}"
        );
        assert!(
            r.contains(" 5  tip     Install the cohors extension for VS Code"),
            "{r}"
        );
        assert!(
            r.contains("code --install-extension rushirb2001.cohors"),
            "{r}"
        );

        // `cohors next 5` would be `code …`: refused, with the command to copy.
        let extension = &list[4];
        assert!(!extension.runs_cohors());
        let refusal = not_run(extension, false);
        assert!(refusal.contains("only runs cohors itself; this one is for `code`"));
        assert!(refusal.contains("    code --install-extension rushirb2001.cohors\n"));
        assert!(matches!(
            run_suggestion(extension)
                .unwrap_err()
                .downcast_ref::<ExitCode>(),
            Some(ExitCode(2))
        ));
    }
}
