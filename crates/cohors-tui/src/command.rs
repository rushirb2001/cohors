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

/// The commands [`parse`] understands, as tab completion offers them, and
/// whether each takes an argument (so completion can leave a space after it).
const VERBS: [(&str, bool); 13] = [
    ("fetch", false),
    ("pull", false),
    ("push", false),
    ("commit", true),
    ("sort", true),
    ("dirty", false),
    ("filter", true),
    ("jump", true),
    ("standup", false),
    ("tasks", false),
    ("refresh", false),
    ("help", false),
    ("quit", false),
];

/// The words `:sort` takes.
const SORTS: [&str; 3] = ["name", "dirty", "recent"];

/// Every way to finish `line`, each as the whole line it would become.
///
/// A first word is finished as a command, then as a repository name (a bare
/// name jumps to that repository). After `sort` the sort modes are offered,
/// after `jump` the repository names. Shell lines (`!…`) and filters (`/…`)
/// are free text and are not completed.
pub fn completions(line: &str, repos: &[String]) -> Vec<String> {
    let body = line.trim_start();
    if body.starts_with('!') || body.starts_with('/') {
        return Vec::new();
    }
    let starts = |name: &str, typed: &str| name.to_lowercase().starts_with(&typed.to_lowercase());
    match body.split_once(char::is_whitespace) {
        None => {
            let mut found: Vec<String> = VERBS
                .iter()
                .filter(|(verb, _)| starts(verb, body))
                .map(|(verb, takes_argument)| {
                    if *takes_argument {
                        format!("{verb} ")
                    } else {
                        verb.to_string()
                    }
                })
                .collect();
            // With nothing typed, the commands alone are list enough.
            if !body.is_empty() {
                found.extend(repos.iter().filter(|name| starts(name, body)).cloned());
            }
            found
        }
        Some((verb, argument)) => {
            let argument = argument.trim_start();
            match verb.to_lowercase().as_str() {
                "sort" => SORTS
                    .iter()
                    .filter(|mode| starts(mode, argument))
                    .map(|mode| format!("sort {mode}"))
                    .collect(),
                "jump" | "j" => repos
                    .iter()
                    .filter(|name| starts(name, argument))
                    .map(|name| format!("jump {name}"))
                    .collect(),
                _ => Vec::new(),
            }
        }
    }
}

/// How many past lines the command line remembers.
const HISTORY_LIMIT: usize = 50;

/// Everything about the command line except its text: where the cursor is,
/// the lines run before, and a tab completion in progress.
///
/// The text itself stays a plain `String` owned by the caller, and every
/// method here takes it as `line`. That keeps the line easy to read, set and
/// test from outside, and keeps this type about editing only.
#[derive(Debug, Clone, Default)]
pub struct LineState {
    /// How many characters are to the RIGHT of the cursor. Counting from the
    /// end means zero is "at the end of the line", which is where the cursor
    /// belongs whenever the text is replaced wholesale.
    after: usize,
    /// Lines that were run, oldest first.
    history: Vec<String>,
    /// Which remembered line is showing, counted back from the newest
    /// (`None` while the line being typed is showing).
    recalled: Option<usize>,
    /// The line being typed, set aside while a remembered one is showing.
    draft: String,
    /// The completions being cycled through, and which one is showing.
    cycle: Option<(Vec<String>, usize)>,
}

impl LineState {
    /// A fresh line: the cursor at the end, nothing recalled, no completion.
    /// The history is kept.
    pub fn start(&mut self) {
        self.after = 0;
        self.recalled = None;
        self.cycle = None;
        self.draft.clear();
    }

    /// The cursor's position in `line`, in characters from the start.
    fn cursor(&self, line: &str) -> usize {
        line.chars().count().saturating_sub(self.after)
    }

    /// The byte offset of the cursor in `line` (strings are indexed in
    /// bytes, and a character can be several).
    fn cursor_byte(&self, line: &str) -> usize {
        line.char_indices()
            .nth(self.cursor(line))
            .map_or(line.len(), |(byte, _)| byte)
    }

    /// The text before and after the cursor, for drawing the cursor between.
    pub fn split<'a>(&self, line: &'a str) -> (&'a str, &'a str) {
        line.split_at(self.cursor_byte(line))
    }

    /// Any edit ends a completion cycle and a recall: what is on the line is
    /// now the line being typed.
    fn edited(&mut self) {
        self.cycle = None;
        self.recalled = None;
    }

    pub fn insert(&mut self, line: &mut String, c: char) {
        line.insert(self.cursor_byte(line), c);
        self.edited();
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self, line: &mut String) {
        let at = self.cursor(line);
        if at == 0 {
            return;
        }
        let start = line.char_indices().nth(at - 1).map_or(0, |(byte, _)| byte);
        line.remove(start);
        self.edited();
    }

    /// Delete the character under the cursor.
    pub fn delete(&mut self, line: &mut String) {
        if self.after == 0 {
            return;
        }
        line.remove(self.cursor_byte(line));
        self.after -= 1;
        self.edited();
    }

    pub fn left(&mut self, line: &str) {
        self.after = (self.after + 1).min(line.chars().count());
    }

    pub fn right(&mut self) {
        self.after = self.after.saturating_sub(1);
    }

    pub fn home(&mut self, line: &str) {
        self.after = line.chars().count();
    }

    pub fn end(&mut self) {
        self.after = 0;
    }

    /// Delete everything before the cursor (Ctrl-U).
    pub fn kill_before(&mut self, line: &mut String) {
        line.drain(..self.cursor_byte(line));
        self.edited();
    }

    /// Delete the word before the cursor, and any spaces after it (Ctrl-W).
    pub fn kill_word(&mut self, line: &mut String) {
        let end = self.cursor_byte(line);
        let before = &line[..end];
        let word_end = before.trim_end().len();
        let start = before[..word_end]
            .rfind(char::is_whitespace)
            .map_or(0, |space| space + 1);
        line.drain(start..end);
        self.edited();
    }

    /// Remember a line that was run, unless it repeats the one before it.
    pub fn remember(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() || self.history.last().is_some_and(|last| last == line) {
            return;
        }
        self.history.push(line.to_string());
        if self.history.len() > HISTORY_LIMIT {
            self.history.remove(0);
        }
    }

    /// Show the line run before the one showing (Up).
    pub fn older(&mut self, line: &mut String) {
        let next = match self.recalled {
            None if self.history.is_empty() => return,
            None => {
                self.draft = line.clone();
                0
            }
            Some(back) if back + 1 < self.history.len() => back + 1,
            Some(_) => return, // already at the oldest
        };
        self.show(line, Some(next));
    }

    /// Show the line run after the one showing, ending back at the line that
    /// was being typed (Down).
    pub fn newer(&mut self, line: &mut String) {
        match self.recalled {
            None => {}
            Some(0) => self.show(line, None),
            Some(back) => self.show(line, Some(back - 1)),
        }
    }

    fn show(&mut self, line: &mut String, recalled: Option<usize>) {
        *line = match recalled {
            Some(back) => self.history[self.history.len() - 1 - back].clone(),
            None => std::mem::take(&mut self.draft),
        };
        self.recalled = recalled;
        self.after = 0;
        self.cycle = None;
    }

    /// Finish the line (Tab). Pressing it again moves to the next way of
    /// finishing it, or the previous one with `backwards` (Shift-Tab), and
    /// wraps around. Does nothing when there is no way to finish it.
    pub fn complete(&mut self, line: &mut String, repos: &[String], backwards: bool) {
        // Still on a completion from the last press: step through the list.
        let (options, index) = match self.cycle.take() {
            Some((options, index)) if options.get(index) == Some(&*line) => {
                let count = options.len();
                let next = if backwards {
                    (index + count - 1) % count
                } else {
                    (index + 1) % count
                };
                (options, next)
            }
            _ => {
                let options = completions(line, repos);
                if options.is_empty() {
                    return;
                }
                let first = if backwards { options.len() - 1 } else { 0 };
                (options, first)
            }
        };
        *line = options[index].clone();
        self.after = 0;
        self.recalled = None;
        self.cycle = Some((options, index));
    }

    /// While completions are being cycled: which one is showing and how many
    /// there are, as `(2, 5)`.
    pub fn completing(&self) -> Option<(usize, usize)> {
        self.cycle
            .as_ref()
            .map(|(options, index)| (index + 1, options.len()))
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

    fn repos() -> Vec<String> {
        ["payments", "pay-gateway", "web-app", "Fetcher"]
            .map(String::from)
            .to_vec()
    }

    #[test]
    fn completion_offers_commands_then_repositories() {
        let repos = repos();
        // A command that takes an argument is left ready for it.
        assert_eq!(completions("so", &repos), ["sort "]);
        assert_eq!(completions("st", &repos), ["standup"]);
        // A first word can also be a repository (a bare name jumps to it),
        // matched whatever its case.
        assert_eq!(completions("fet", &repos), ["fetch", "Fetcher"]);
        assert_eq!(completions("pay", &repos), ["payments", "pay-gateway"]);
        // Arguments.
        assert_eq!(completions("sort r", &repos), ["sort recent"]);
        assert_eq!(completions("sort ", &repos).len(), 3);
        assert_eq!(completions("jump w", &repos), ["jump web-app"]);
        // Nothing typed lists the commands, not the whole fleet.
        assert_eq!(completions("", &repos).len(), VERBS.len());
        // Free text is left alone.
        assert!(completions("!git st", &repos).is_empty());
        assert!(completions("/pay", &repos).is_empty());
        assert!(completions("commit fix the", &repos).is_empty());
        // Everything completion offers is something `parse` accepts.
        for (verb, takes_argument) in VERBS {
            let line = if takes_argument {
                format!("{verb} name")
            } else {
                verb.to_string()
            };
            assert!(parse(&line).is_some(), "{line}");
        }
    }

    #[test]
    fn tab_cycles_through_the_completions_both_ways() {
        let repos = repos();
        let mut state = LineState::default();
        let mut line = "pay".to_string();
        state.complete(&mut line, &repos, false);
        assert_eq!(line, "payments");
        assert_eq!(state.completing(), Some((1, 2)));
        state.complete(&mut line, &repos, false);
        assert_eq!(line, "pay-gateway");
        state.complete(&mut line, &repos, false);
        assert_eq!(line, "payments"); // wraps
        state.complete(&mut line, &repos, true);
        assert_eq!(line, "pay-gateway");
        // Typing ends the cycle; the next Tab completes what is there now.
        state.insert(&mut line, 'x');
        assert_eq!(state.completing(), None);
        state.complete(&mut line, &repos, false);
        assert_eq!(line, "pay-gatewayx"); // nothing matches: left alone
    }

    #[test]
    fn the_cursor_moves_and_edits_in_the_middle_of_the_line() {
        let mut state = LineState::default();
        let mut line = String::new();
        for c in "sort nme".chars() {
            state.insert(&mut line, c);
        }
        // Back two, add the missing letter.
        state.left(&line);
        state.left(&line);
        state.insert(&mut line, 'a');
        assert_eq!(line, "sort name");
        assert_eq!(state.split(&line), ("sort na", "me"));
        state.backspace(&mut line);
        state.delete(&mut line);
        assert_eq!(line, "sort ne");
        // Home and End, and the ends are safe to push against.
        state.home(&line);
        state.left(&line);
        state.backspace(&mut line);
        assert_eq!(state.split(&line), ("", "sort ne"));
        state.end();
        state.right();
        state.delete(&mut line);
        assert_eq!(state.split(&line), ("sort ne", ""));
        // Ctrl-W takes the word before the cursor, Ctrl-U everything before it.
        let mut line = "commit fix the  ".to_string();
        state.start();
        state.kill_word(&mut line);
        assert_eq!(line, "commit fix ");
        state.left(&line);
        state.kill_before(&mut line);
        assert_eq!(line, " ");
        // A character of several bytes is one step.
        let mut line = "jump café".to_string();
        state.start();
        state.backspace(&mut line);
        assert_eq!(line, "jump caf");
    }

    #[test]
    fn up_and_down_walk_the_lines_run_before() {
        let mut state = LineState::default();
        let mut line = String::new();
        // Nothing run yet: Up does nothing.
        state.older(&mut line);
        assert_eq!(line, "");
        for run in ["fetch", "sort name", "sort name", "  ", "!git status"] {
            state.remember(run);
        }
        // A repeat and a blank line are not remembered.
        line = "pu".to_string();
        state.start();
        state.older(&mut line);
        assert_eq!(line, "!git status");
        state.older(&mut line);
        assert_eq!(line, "sort name");
        state.older(&mut line);
        state.older(&mut line); // already at the oldest
        assert_eq!(line, "fetch");
        // Down comes back, ending at what was being typed.
        state.newer(&mut line);
        state.newer(&mut line);
        assert_eq!(line, "!git status");
        state.newer(&mut line);
        assert_eq!(line, "pu");
        state.newer(&mut line);
        assert_eq!(line, "pu");
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
