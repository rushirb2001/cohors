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
    Refresh,
    Standup,
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
        "refresh" | "r" => Some(Command::Refresh),
        "standup" | "st" => Some(Command::Standup),
        "help" | "h" | "?" => Some(Command::Help),
        "quit" | "q" => Some(Command::Quit),
        "dirty" => Some(Command::DirtyOnly),
        "sort" => parse_sort(arg).map(Command::Sort),
        "filter" => Some(Command::Filter(arg.to_string())),
        // Any other bare token (no argument) jumps to a repo by name.
        _ if arg.is_empty() => Some(Command::Jump(verb_raw.to_string())),
        // A known-shaped command with an unexpected argument is not valid.
        _ => None,
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
        assert_eq!(
            parse(":!git status"),
            Some(Command::Run("git status".into()))
        );
        assert_eq!(parse("   "), None);
        assert_eq!(parse("sort sideways"), None);
    }
}
