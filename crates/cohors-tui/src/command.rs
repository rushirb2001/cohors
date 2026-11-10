//! Pure parser for the `:`-style command mode.
//!
//! Turns a raw input line (e.g. `:sort name`, `/wip`, `cohors`) into a typed
//! [`Command`]. This module is intentionally pure — no I/O, no UI — so it can be
//! unit-tested in isolation and wired into the TUI separately.

use cohors_core::SortMode;

/// A command the user can issue from command mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Fetch,
    Pull,
    Push,
    /// Commit staged + unstaged changes across the target repos with this message
    /// (`:commit <message>`). Opens a confirmation modal before acting.
    Commit(String),
    Refresh,
    Standup,
    /// Open the launchpad view (`:launchpad` / `:tasks`).
    Launchpad,
    Help,
    Quit,
    DirtyOnly,
    Sort(SortMode),
    Filter(String),
    Jump(String),
    /// Run an arbitrary shell command across the target repos (`:!<cmd>`).
    Run(String),
}

/// Parse a command-mode input line into a [`Command`].
///
/// The input is trimmed and an optional leading `:` is stripped. Only the verb
/// token is lowercased; filter/jump arguments keep their original case. Returns
/// `None` for empty input or an unrecognized command.
pub fn parse(input: &str) -> Option<Command> {
    // Trim, then strip a single optional leading ':'.
    let trimmed = input.trim();
    let body = trimmed.strip_prefix(':').unwrap_or(trimmed).trim();

    if body.is_empty() {
        return None;
    }

    // `/text` is shorthand for `filter text` (case preserved).
    if let Some(rest) = body.strip_prefix('/') {
        return Some(Command::Filter(rest.to_string()));
    }

    // `!cmd` runs a shell command across the target repos (case preserved).
    if let Some(rest) = body.strip_prefix('!') {
        let cmd = rest.trim();
        return (!cmd.is_empty()).then(|| Command::Run(cmd.to_string()));
    }

    // Split into the verb token and the (optional, case-preserved) argument.
    let (verb_raw, arg) = match body.split_once(char::is_whitespace) {
        Some((v, a)) => (v, a.trim()),
        None => (body, ""),
    };
    let verb = verb_raw.to_lowercase();

    match verb.as_str() {
        "fetch" | "f" => Some(Command::Fetch),
        "pull" | "pl" => Some(Command::Pull),
        "push" | "p" => Some(Command::Push),
        // `commit` needs a message; with none it's invalid (not a repo jump).
        "commit" | "ci" => (!arg.is_empty()).then(|| Command::Commit(arg.to_string())),
        "refresh" | "r" => Some(Command::Refresh),
        "standup" | "st" => Some(Command::Standup),
        "launchpad" | "tasks" | "lp" => Some(Command::Launchpad),
        "help" | "h" | "?" => Some(Command::Help),
        "quit" | "q" => Some(Command::Quit),
        "dirty" => Some(Command::DirtyOnly),
        "sort" => parse_sort(arg).map(Command::Sort),
        "filter" => Some(Command::Filter(arg.to_string())),
        // `:jump <repo>` is what the command sheet shows; the name alone also
        // works (the catch-all below).
        "jump" | "j" if !arg.is_empty() => Some(Command::Jump(arg.to_string())),
        // Any other bare token (no argument) jumps to a repo by name.
        _ if arg.is_empty() => Some(Command::Jump(verb_raw.to_string())),
        // A known-shaped command with an unexpected argument is not valid.
        _ => None,
    }
}

/// What to tell the user about a line [`parse`] turned down: which command it
/// was and what it is missing, rather than a bare "unknown command".
pub fn explain(input: &str) -> String {
    let body = input.trim().trim_start_matches(':').trim();
    let verb = body.split_whitespace().next().unwrap_or("").to_lowercase();
    match verb.as_str() {
        "" => "nothing typed".to_string(),
        "commit" | "ci" => "commit needs a message: :commit <message>".to_string(),
        "sort" => "sort needs a mode: :sort name, :sort dirty or :sort recent".to_string(),
        _ if body.starts_with('!') => "nothing to run after !".to_string(),
        _ => format!("unknown command: {verb}  (? lists them)"),
    }
}

/// Map a human-friendly sort word to a [`SortMode`], or `None` if unrecognized.
fn parse_sort(arg: &str) -> Option<SortMode> {
    match arg.trim().to_lowercase().as_str() {
        "dirty" => Some(SortMode::DirtyFirst),
        "name" => Some(SortMode::Name),
        "recent" | "activity" => Some(SortMode::Recent),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_representative_commands() {
        assert_eq!(parse(":f"), Some(Command::Fetch));
        assert_eq!(parse("quit"), Some(Command::Quit));
        assert_eq!(parse(":sort recent"), Some(Command::Sort(SortMode::Recent)));
        assert_eq!(
            parse("filter MyRepo"),
            Some(Command::Filter("MyRepo".into()))
        );
        assert_eq!(parse("/WIP"), Some(Command::Filter("WIP".into())));
        assert_eq!(parse("cohors"), Some(Command::Jump("cohors".into())));
        // The spelled-out form the command sheet shows, with the name's case kept.
        assert_eq!(parse(":jump MyRepo"), Some(Command::Jump("MyRepo".into())));
        assert_eq!(
            parse(":!git status"),
            Some(Command::Run("git status".into()))
        );
        assert_eq!(parse(":tasks"), Some(Command::Launchpad));
        assert_eq!(parse("   "), None);
        assert_eq!(parse("sort sideways"), None);
    }

    #[test]
    fn a_rejected_line_says_what_was_wrong_with_it() {
        for (line, says) in [
            (":commit", "needs a message"),
            ("sort", "needs a mode"),
            ("sort sideways", "needs a mode"),
            ("frobnicate now", "unknown command: frobnicate"),
            ("!", "nothing to run"),
        ] {
            assert_eq!(parse(line), None, "{line}");
            assert!(explain(line).contains(says), "{line}: {}", explain(line));
        }
    }

    #[test]
    fn commit_needs_a_message() {
        assert_eq!(
            parse(":commit snapshot wip"),
            Some(Command::Commit("snapshot wip".into()))
        );
        assert_eq!(parse(":ci done"), Some(Command::Commit("done".into())));
        // No message ⇒ not a valid commit (and not a repo jump either).
        assert_eq!(parse(":commit"), None);
    }
}
