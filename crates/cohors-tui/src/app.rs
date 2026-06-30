//! Dashboard state and key handling.
//!
//! The state is deliberately small and rendering-free: the ordered, filtered
//! view is *derived* each frame via [`cohors_core::compute_view`], so the
//! "what to show, in what order" logic stays in the shared core. Key handling
//! returns a [`Cmd`] telling the event loop what side effect to run (quit,
//! rescan), keeping I/O out of the state.

use std::collections::HashSet;

use cohors_core::{
    RemoteDetail, RepoDetail, RepoId, RepoSnapshot, SortMode, StandupCommit, StandupWindow,
    ViewParams, ViewRow, compute_view, group_commits,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Which input mode the dashboard is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Typing into the fuzzy filter.
    Filter,
    /// Typing a `:`-command (vim/k9s-style command line).
    Command,
    /// The help overlay is open.
    Help,
    /// The weekly-standup view is open.
    Standup,
    /// The per-repo command-run results view.
    CommandRun,
    /// A yes/no confirmation modal for a destructive bulk action.
    Confirm,
    /// The "Open with…" picker (choose an editor / reveal / lazygit).
    OpenWith,
    /// The per-repo drill-in detail pane (commits / files / branches / stashes).
    Detail,
}

/// A side effect for the event loop to perform after a key is handled. Actions
/// that target a repo operate on the current selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    None,
    Quit,
    /// Re-run the scan.
    Refresh,
    /// Fetch the selected repo.
    FetchSelected,
    /// Fetch every repo.
    FetchAll,
    /// Pull (fast-forward-only) the selected repo.
    PullSelected,
    /// Push the selected repo's current branch to its upstream.
    PushSelected,
    /// Open the per-repo detail pane for the current repo (lazy-loads its data).
    OpenDetail,
    /// Open the selected repo in lazygit.
    Lazygit,
    /// Copy the selected repo's path to the clipboard.
    CopyPath,
    /// Open the weekly-standup view (collect commits).
    OpenStandup,
    /// Cycle the standup window (today ↔ this week) and re-collect.
    StandupNextWindow,
    /// Copy the standup markdown to the clipboard.
    CopyStandup,
    /// Run the typed command across the target repos.
    RunCommand,
    /// Copy the focused repo's command output to the clipboard.
    CopyRunOutput,
    /// The user accepted the pending confirmation; run its action.
    ConfirmAccept,
    /// Open the "Open with…" picker for the current repo.
    OpenWith,
    /// Run the picker's highlighted opener (editor / reveal / lazygit).
    OpenWithAccept,
    /// Remember the picker's highlighted editor as the default.
    OpenWithSetDefault,
    /// Adopt the auto-detected roots from the empty-state rescue prompt.
    UseSuggestedRoots,
}

/// How a registry action verb is reached from the TUI. This is the TUI half of
/// the cross-surface parity test: every `cohors_actions::registry()` verb must
/// map to a binding, so adding an action without wiring the TUI fails the build.
/// Test-only scaffolding for now — it encodes the wiring the parity test checks.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerbBinding {
    /// A key (or `:`-command) that returns this event-loop [`Cmd`].
    Direct(Cmd),
    /// A confirm-gated action: opens a modal, then `ConfirmAccept` runs it.
    Confirmed,
}

/// Map a registry action verb to how the TUI triggers it. `None` means the verb
/// isn't wired in the TUI — which the parity test rejects. The `Direct` arms name
/// real [`Cmd`] variants, so removing one breaks this match at compile time.
#[cfg(test)]
pub fn verb_binding(verb: &str) -> Option<VerbBinding> {
    Some(match verb {
        "fetch" => VerbBinding::Direct(Cmd::FetchSelected),
        "pull" => VerbBinding::Direct(Cmd::PullSelected),
        "push" => VerbBinding::Direct(Cmd::PushSelected),
        "run" => VerbBinding::Direct(Cmd::RunCommand),
        // Confirm-gated: 'S' / `:commit <msg>` open a modal, then ConfirmAccept.
        "stash" | "commit" => VerbBinding::Confirmed,
        _ => return None,
    })
}

/// A destructive bulk action awaiting confirmation.
#[derive(Debug, Clone)]
pub struct Pending {
    /// The question shown in the modal, e.g. "Stash changes in 4 repos?".
    pub prompt: String,
    pub action: ConfirmAction,
}

/// What a confirmed modal will do. One variant per destructive bulk action.
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    /// `git stash push` across these repos.
    BulkStash(Vec<RepoId>),
    /// `git add -A && git commit -m <message>` across these repos.
    BulkCommit { ids: Vec<RepoId>, message: String },
}

/// One repo's slot in a command run.
#[derive(Debug, Clone)]
pub struct RunResult {
    pub id: RepoId,
    pub name: String,
    pub state: RunState,
}

/// Where a repo is in a command run.
#[derive(Debug, Clone)]
pub enum RunState {
    Running,
    /// Finished with this exit code and captured output (`code` = -1 if the
    /// process couldn't even spawn).
    Done {
        code: i32,
        stdout: String,
        stderr: String,
    },
}

/// The in-flight / last command run.
pub struct CommandRun {
    /// Monotonic id so the event loop can discard a previous run's late results.
    pub run_id: u64,
    /// The raw command line the user typed.
    pub command: String,
    /// One slot per target repo, in view order.
    pub results: Vec<RunResult>,
    /// Which repo's output is expanded in the right pane (index into `results`).
    pub focus: usize,
    /// Vertical scroll offset within the focused repo's output.
    pub scroll: u16,
    /// Max scroll, cached from the last render so key handling can clamp without
    /// knowing the viewport (mirrors the standup overlay).
    max_scroll: std::cell::Cell<u16>,
}

impl CommandRun {
    /// A fresh run: all repos `Running`, focus/scroll at the top.
    pub fn new(run_id: u64, command: String, results: Vec<RunResult>) -> Self {
        Self {
            run_id,
            command,
            results,
            focus: 0,
            scroll: 0,
            max_scroll: std::cell::Cell::new(0),
        }
    }

    /// Tally of `(passed, failed, still running)` for the summary line.
    pub fn summary(&self) -> (usize, usize, usize) {
        let mut ok = 0;
        let mut fail = 0;
        let mut running = 0;
        for r in &self.results {
            match &r.state {
                RunState::Running => running += 1,
                RunState::Done { code: 0, .. } => ok += 1,
                RunState::Done { .. } => fail += 1,
            }
        }
        (ok, fail, running)
    }

    /// The focused repo's combined output (stdout, then stderr), for copy.
    pub fn focused_output(&self) -> String {
        match self.results.get(self.focus).map(|r| &r.state) {
            Some(RunState::Done { stdout, stderr, .. }) => {
                let mut out = stdout.clone();
                if !stderr.is_empty() {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str(stderr);
                }
                out
            }
            _ => String::new(),
        }
    }

    /// Cache the focused output's max scroll (the view calls this each frame).
    pub fn set_max_scroll(&self, max: u16) {
        self.max_scroll.set(max);
    }
}

/// The weekly-standup view: the user's commits grouped per repo, shown as a
/// repo list + the focused repo's scrollable commits (mirrors `CommandRun`).
pub struct StandupView {
    /// The raw commits, kept for the markdown copy (`y`).
    pub commits: Vec<StandupCommit>,
    /// Commits grouped by repo, most-active first (shared ordering with the
    /// markdown digest via `group_commits`).
    pub groups: Vec<(String, Vec<StandupCommit>)>,
    /// Which repo's commits are shown (index into `groups`).
    pub focus: usize,
    /// The highlighted commit within the focused repo (index into its commits);
    /// movement keeps it visible, so scrolling is contextual.
    pub commit_cursor: usize,
    /// Which pane the keys drive: `false` = the repo list (↑/↓ switch repos),
    /// `true` = the commits (↑/↓ move the highlighted commit). Lets arrow-only
    /// users read the full history without PgUp/PgDn.
    pub commits_focused: bool,
    /// Render-managed scroll offset for the commits pane (kept so the cursor
    /// stays visible); interior mutability like the other scroll states.
    commit_offset: std::cell::Cell<u16>,
}

impl StandupView {
    pub fn new(commits: Vec<StandupCommit>) -> Self {
        let groups = group_commits(&commits);
        Self {
            commits,
            groups,
            focus: 0,
            commit_cursor: 0,
            commits_focused: false,
            commit_offset: std::cell::Cell::new(0),
        }
    }

    /// Number of commits in the focused repo.
    pub fn focused_len(&self) -> usize {
        self.groups.get(self.focus).map_or(0, |(_, c)| c.len())
    }

    /// The commits-pane scroll offset (render reads/writes it each frame).
    pub fn offset(&self) -> u16 {
        self.commit_offset.get()
    }
    pub fn set_offset(&self, offset: u16) {
        self.commit_offset.set(offset);
    }
}

/// All dashboard state.
pub struct App {
    pub repos: Vec<RepoSnapshot>,
    pub sort: SortMode,
    pub dirty_only: bool,
    pub mode: Mode,
    pub filter: String,
    /// Index into the current (filtered/sorted) view, not into `repos`.
    pub selected: usize,
    /// A scan is in flight (drives the header spinner / loading state).
    pub scanning: bool,
    /// Transient status line (action results); cleared on the next action.
    pub status: Option<String>,
    /// Animation tick for the spinner.
    pub spinner: usize,
    /// Repos with an action (fetch/pull) in flight — shown with a row spinner.
    pub busy: HashSet<RepoId>,
    /// Repos the user has explicitly marked (Space) for bulk actions, keyed by
    /// id so the set survives re-sort / filter / refresh — exactly like `busy`.
    pub selection: HashSet<RepoId>,
    /// Configured roots, for the empty/loading states.
    pub roots: Vec<String>,
    /// Config file path, for the help overlay.
    pub config_path: String,
    /// The standup view (`None` while collecting, or not opened).
    pub standup: Option<StandupView>,
    /// The standup time window.
    pub standup_window: StandupWindow,
    /// The command being typed in `CommandInput` mode (mirrors `filter`).
    pub command_input: String,
    /// The `:`-command being typed in `Command` mode.
    pub command_line: String,
    /// The in-flight / last command run, shown in `CommandRun` mode.
    pub run: Option<CommandRun>,
    /// The destructive action awaiting confirmation (`Some` ⇒ `Mode::Confirm`).
    pub confirm: Option<Pending>,
    /// Collapse the footer hint boxes to a single divider line (toggled with `h`).
    pub hints_hidden: bool,
    /// The "Open with…" picker (`Some` ⇒ `Mode::OpenWith`).
    pub open_with: Option<OpenWith>,
    /// The repo table's scroll offset, cached from the last render (interior
    /// mutability, like the overlay scroll states) so the panel can show a
    /// "… N more" affordance when the list overflows the window.
    pub repos_scroll: std::cell::Cell<usize>,
    /// The drill-in detail pane (`Some` ⇒ `Mode::Detail`).
    pub detail: Option<DetailView>,
    /// Repos auto-detected elsewhere when the configured roots came up empty —
    /// drives the first-run rescue prompt in the empty state. Empty otherwise.
    pub suggested_roots: Vec<String>,
    /// Configured glyph mode for the dashboard (auto / ascii / unicode / nerd).
    pub icons: cohors_config::IconMode,
}

/// The per-repo detail pane: lazily-loaded git facts for one repo, scrollable.
pub struct DetailView {
    /// Which repo this is for (to ignore late background results after a switch).
    pub repo_id: RepoId,
    pub repo_name: String,
    /// `None` while the local git data is still being read in the background.
    pub detail: Option<RepoDetail>,
    /// GitHub PRs + contributors, fetched when the pane opens (`None` until then,
    /// or when there's no GitHub remote / token).
    pub remote: Option<RemoteDetail>,
    /// A remote fetch is in flight (drives the "loading…" state).
    pub remote_pending: bool,
    /// Vertical scroll offset (the left/state pane).
    pub scroll: u16,
    /// Max scroll, cached from the last render for clamp-without-viewport.
    max_scroll: std::cell::Cell<u16>,
}

impl DetailView {
    pub fn new(repo_id: RepoId, repo_name: String) -> Self {
        Self {
            repo_id,
            repo_name,
            detail: None,
            remote: None,
            remote_pending: false,
            scroll: 0,
            max_scroll: std::cell::Cell::new(0),
        }
    }

    pub fn set_max_scroll(&self, max: u16) {
        self.max_scroll.set(max);
    }
}

/// One choice in the "Open with…" picker.
pub enum Opener {
    /// Launch an editor by command, opening the repo folder.
    Editor { command: String, label: String },
    /// Reveal the repo in the OS file manager.
    Reveal,
    /// Open the repo in lazygit.
    Lazygit,
}

/// State for the "Open with…" picker: the openers, the cursor, and which editor
/// command is currently the default (so the list can mark it).
pub struct OpenWith {
    pub openers: Vec<Opener>,
    pub cursor: usize,
    pub default_command: Option<String>,
}

impl OpenWith {
    pub fn new(openers: Vec<Opener>, default_command: Option<String>) -> Self {
        // Start the cursor on the current default editor when it's in the list.
        let cursor = default_command
            .as_deref()
            .and_then(|d| {
                openers
                    .iter()
                    .position(|o| matches!(o, Opener::Editor { command, .. } if command == d))
            })
            .unwrap_or(0);
        Self {
            openers,
            cursor,
            default_command,
        }
    }
}

impl App {
    pub fn new(roots: Vec<String>, config_path: String) -> Self {
        Self {
            repos: Vec::new(),
            sort: SortMode::default(),
            dirty_only: false,
            mode: Mode::Normal,
            filter: String::new(),
            selected: 0,
            scanning: false,
            status: None,
            spinner: 0,
            busy: HashSet::new(),
            selection: HashSet::new(),
            roots,
            config_path,
            standup: None,
            standup_window: StandupWindow::Week,
            command_input: String::new(),
            command_line: String::new(),
            run: None,
            confirm: None,
            hints_hidden: false,
            open_with: None,
            repos_scroll: std::cell::Cell::new(0),
            detail: None,
            suggested_roots: Vec::new(),
            icons: cohors_config::IconMode::default(),
        }
    }

    /// True when the fleet is empty but we detected repos elsewhere, so the
    /// empty state offers a one-key rescue.
    pub fn empty_picker_active(&self) -> bool {
        self.repos.is_empty() && !self.scanning && !self.suggested_roots.is_empty()
    }

    /// The ordered, filtered rows to render this frame.
    pub fn view(&self) -> Vec<ViewRow> {
        compute_view(
            &self.repos,
            &ViewParams {
                sort: self.sort,
                dirty_only: self.dirty_only,
                query: &self.filter,
            },
        )
    }

    /// Number of currently-visible repos.
    pub fn visible_len(&self) -> usize {
        self.view().len()
    }

    /// The repo under the cursor, if any.
    pub fn selected_repo(&self) -> Option<&RepoSnapshot> {
        self.view()
            .get(self.selected)
            .map(|row| &self.repos[row.index])
    }

    /// The repos an action should operate on: the marked selection (intersected
    /// with what's currently visible, in view order) if any are marked, else the
    /// repo under the cursor. Callers still drop error/path-less repos, exactly
    /// as the single-repo action paths already do.
    pub fn action_targets(&self) -> Vec<RepoId> {
        if self.selection.is_empty() {
            return self
                .selected_repo()
                .map(|r| r.id.clone())
                .into_iter()
                .collect();
        }
        self.view()
            .iter()
            .map(|vr| self.repos[vr.index].id.clone())
            .filter(|id| self.selection.contains(id))
            .collect()
    }

    /// Toggle the cursor repo's membership in the selection. No-op on an error
    /// or path-less repo — those can't be acted on, so they can't be marked.
    fn toggle_selection(&mut self) {
        let Some(repo) = self.selected_repo() else {
            return;
        };
        if repo.has_error() {
            return;
        }
        let id = repo.id.clone();
        if !self.selection.remove(&id) {
            self.selection.insert(id);
        }
    }

    /// Mark every visible, actionable repo — or clear them all if they're
    /// already marked (toggle-all).
    fn select_all_visible(&mut self) {
        let ids: Vec<RepoId> = self
            .view()
            .iter()
            .map(|vr| &self.repos[vr.index])
            .filter(|r| !r.has_error())
            .map(|r| r.id.clone())
            .collect();
        let all_marked = !ids.is_empty() && ids.iter().all(|id| self.selection.contains(id));
        for id in ids {
            if all_marked {
                self.selection.remove(&id);
            } else {
                self.selection.insert(id);
            }
        }
    }

    /// Replace the repo set (e.g. after a scan) and keep the selection in range.
    pub fn set_repos(&mut self, repos: Vec<RepoSnapshot>) {
        // Keep the cursor on the *same repo* across a re-scan, not the same row
        // index. A `--watch` re-scan re-sorts the fleet (e.g. a repo you just
        // pushed goes from "ahead" to clean and drops down the dirty-first list);
        // anchoring to the index would leave the cursor — and the detail dock —
        // pointing at a different repo. We remember the selected repo's id and
        // restore the cursor to its new position. Marked selection + busy already
        // survive re-sorts because they're keyed by id; this gives the cursor the
        // same stability.
        let keep = self.selected_repo().map(|r| r.id.clone());
        self.repos = repos;
        if let Some(id) = keep
            && let Some(pos) = self
                .view()
                .iter()
                .position(|row| self.repos[row.index].id == id)
        {
            self.selected = pos;
        }
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        let n = self.visible_len();
        self.selected = if n == 0 { 0 } else { self.selected.min(n - 1) };
    }

    /// Handle a key press and report any side effect for the event loop.
    pub fn on_key(&mut self, key: KeyEvent) -> Cmd {
        // Ctrl-C quits from any mode (raw mode delivers it as a key, not a signal).
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Cmd::Quit;
        }
        match self.mode {
            Mode::Normal => self.on_key_normal(key),
            Mode::Filter => self.on_key_filter(key),
            Mode::Command => self.on_key_command_mode(key),
            Mode::Help => {
                self.on_key_help(key);
                Cmd::None
            }
            Mode::Standup => self.on_key_standup(key),
            Mode::CommandRun => self.on_key_command_run(key),
            Mode::Confirm => self.on_key_confirm(key),
            Mode::OpenWith => self.on_key_open_with(key),
            Mode::Detail => self.on_key_detail(key),
        }
    }

    fn on_key_detail(&mut self, key: KeyEvent) -> Cmd {
        let max = self.detail.as_ref().map_or(0, |d| d.max_scroll.get());
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left => {
                self.mode = Mode::Normal;
                self.detail = None;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(d) = &mut self.detail {
                    d.scroll = d.scroll.saturating_add(1).min(max);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(d) = &mut self.detail {
                    d.scroll = d.scroll.saturating_sub(1);
                }
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                if let Some(d) = &mut self.detail {
                    d.scroll = d.scroll.saturating_add(10).min(max);
                }
            }
            KeyCode::PageUp => {
                if let Some(d) = &mut self.detail {
                    d.scroll = d.scroll.saturating_sub(10);
                }
            }
            KeyCode::Char('g') => {
                if let Some(d) = &mut self.detail {
                    d.scroll = 0;
                }
            }
            KeyCode::Char('G') => {
                if let Some(d) = &mut self.detail {
                    d.scroll = max;
                }
            }
            _ => {}
        }
        Cmd::None
    }

    fn on_key_open_with(&mut self, key: KeyEvent) -> Cmd {
        let n = self.open_with.as_ref().map_or(0, |o| o.openers.len());
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Normal;
                self.open_with = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(o) = &mut self.open_with {
                    o.cursor = o.cursor.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(o) = &mut self.open_with
                    && o.cursor + 1 < n
                {
                    o.cursor += 1;
                }
            }
            KeyCode::Enter => return Cmd::OpenWithAccept,
            KeyCode::Char('d') => return Cmd::OpenWithSetDefault,
            _ => {}
        }
        Cmd::None
    }

    fn on_key_normal(&mut self, key: KeyEvent) -> Cmd {
        match key.code {
            KeyCode::Char('q') => return Cmd::Quit,
            // First-run rescue: adopt the auto-detected roots (empty state only).
            KeyCode::Char('u') if self.empty_picker_active() => {
                return Cmd::UseSuggestedRoots;
            }
            // Movement is arrow-keys only (Home/End jump to top/bottom).
            KeyCode::Down => self.move_down(),
            KeyCode::Up => self.move_up(),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.select_last(),
            // Multi-select for bulk actions.
            KeyCode::Char(' ') => self.toggle_selection(),
            KeyCode::Char('a') => self.select_all_visible(),
            KeyCode::Esc => self.selection.clear(),
            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.command_line.clear();
            }
            KeyCode::Char('d') => {
                self.dirty_only = !self.dirty_only;
                self.clamp_selection();
            }
            KeyCode::Char('s') => {
                self.sort = self.sort.next();
                self.clamp_selection();
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('h') => self.hints_hidden = !self.hints_hidden,
            KeyCode::Char('r') => return Cmd::Refresh,
            // Actions (operate on the selection / all repos).
            KeyCode::Char('f') => return Cmd::FetchSelected,
            KeyCode::Char('F') => return Cmd::FetchAll,
            KeyCode::Char('p') => return Cmd::PullSelected,
            KeyCode::Char('P') => return Cmd::PushSelected,
            // `!` is the shell shortcut into the unified command palette:
            // open it pre-seeded with `!` so the next keystroke starts the command.
            KeyCode::Char('!') => {
                self.mode = Mode::Command;
                self.command_line = "!".to_string();
            }
            KeyCode::Char('S') => self.request_bulk_stash(),
            KeyCode::Enter => return Cmd::OpenDetail,
            KeyCode::Char('o') => return Cmd::OpenWith,
            KeyCode::Char('L') => return Cmd::Lazygit,
            KeyCode::Char('y') => return Cmd::CopyPath,
            KeyCode::Tab => {
                self.mode = Mode::Standup;
                self.standup = None;
                return Cmd::OpenStandup;
            }
            _ => {}
        }
        Cmd::None
    }

    fn on_key_filter(&mut self, key: KeyEvent) -> Cmd {
        match key.code {
            // Esc clears the filter and leaves filter mode.
            KeyCode::Esc => {
                self.filter.clear();
                self.selected = 0;
                self.mode = Mode::Normal;
            }
            // Enter confirms, keeping the filter active.
            KeyCode::Enter => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                self.filter.pop();
                self.selected = 0;
            }
            // Arrows navigate the filtered list while typing.
            KeyCode::Down => self.move_down(),
            KeyCode::Up => self.move_up(),
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.selected = 0;
            }
            _ => {}
        }
        self.clamp_selection();
        Cmd::None
    }

    fn on_key_command_mode(&mut self, key: KeyEvent) -> Cmd {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.command_line.clear();
            }
            KeyCode::Enter => {
                let parsed = crate::command::parse(&self.command_line);
                self.command_line.clear();
                self.mode = Mode::Normal;
                return self.apply_command(parsed);
            }
            KeyCode::Backspace => {
                self.command_line.pop();
            }
            KeyCode::Char(c) => self.command_line.push(c),
            _ => {}
        }
        Cmd::None
    }

    /// Apply a parsed `:`-command: state changes happen here; actions return a
    /// `Cmd` for the event loop (reusing the same handlers as the keybindings).
    fn apply_command(&mut self, parsed: Option<crate::command::Command>) -> Cmd {
        use crate::command::Command as C;
        match parsed {
            Some(C::Fetch) => return Cmd::FetchSelected,
            Some(C::Pull) => return Cmd::PullSelected,
            Some(C::Push) => return Cmd::PushSelected,
            Some(C::Commit(message)) => self.request_bulk_commit(message),
            Some(C::Refresh) => return Cmd::Refresh,
            Some(C::Standup) => {
                self.mode = Mode::Standup;
                self.standup = None;
                return Cmd::OpenStandup;
            }
            Some(C::Help) => self.mode = Mode::Help,
            Some(C::Quit) => return Cmd::Quit,
            Some(C::DirtyOnly) => {
                self.dirty_only = !self.dirty_only;
                self.clamp_selection();
            }
            Some(C::Sort(mode)) => {
                self.sort = mode;
                self.clamp_selection();
            }
            Some(C::Filter(f)) => {
                self.filter = f;
                self.selected = 0;
                self.clamp_selection();
            }
            Some(C::Jump(name)) => self.jump_to(&name),
            Some(C::Run(cmd)) => {
                if self.action_targets().is_empty() {
                    self.status = Some("no repos selected".to_string());
                } else {
                    // Feed the shell runner: same path as the `!` key.
                    self.command_input = cmd;
                    self.mode = Mode::CommandRun;
                    return Cmd::RunCommand;
                }
            }
            None => self.status = Some("unknown command".to_string()),
        }
        Cmd::None
    }

    /// Move the cursor to the first visible repo whose name contains `name`.
    fn jump_to(&mut self, name: &str) {
        let needle = name.to_lowercase();
        let hit = self
            .view()
            .iter()
            .position(|vr| self.repos[vr.index].name.to_lowercase().contains(&needle));
        match hit {
            Some(i) => self.selected = i,
            None => self.status = Some(format!("no repo matching '{name}'")),
        }
    }

    fn on_key_help(&mut self, key: KeyEvent) {
        if matches!(
            key.code,
            KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q')
        ) {
            self.mode = Mode::Normal;
        }
    }

    fn on_key_standup(&mut self, key: KeyEvent) -> Cmd {
        let focused = self.standup.as_ref().is_some_and(|s| s.commits_focused);
        match key.code {
            KeyCode::Char('q') => self.mode = Mode::Normal,
            // Esc steps out of the commits pane first, then closes.
            KeyCode::Esc => {
                if focused {
                    if let Some(s) = &mut self.standup {
                        s.commits_focused = false;
                    }
                } else {
                    self.mode = Mode::Normal;
                }
            }
            KeyCode::Char('w') => {
                self.standup_window = self.standup_window.next();
                self.standup = None;
                return Cmd::StandupNextWindow;
            }
            KeyCode::Char('y') => return Cmd::CopyStandup,
            // Tab toggles which pane the keys drive; →/⏎ enter commits, ← goes back.
            KeyCode::Tab => {
                if let Some(s) = &mut self.standup {
                    s.commits_focused = !s.commits_focused;
                }
            }
            KeyCode::Right | KeyCode::Enter => {
                if let Some(s) = &mut self.standup
                    && !s.groups.is_empty()
                {
                    s.commits_focused = true;
                }
            }
            KeyCode::Left => {
                if let Some(s) = &mut self.standup {
                    s.commits_focused = false;
                }
            }
            // ↑/↓ act on the focused pane: move the commit cursor, or switch repos.
            KeyCode::Down | KeyCode::Char('j') => {
                if focused {
                    self.move_commit_cursor(1);
                } else {
                    self.standup_focus_step(1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if focused {
                    self.move_commit_cursor(-1);
                } else {
                    self.standup_focus_step(-1);
                }
            }
            // PgUp/PgDn/g/G always move the commit cursor (a bonus for those keys).
            KeyCode::PageDown | KeyCode::Char(' ') => self.move_commit_cursor(10),
            KeyCode::PageUp => self.move_commit_cursor(-10),
            KeyCode::Char('g') => {
                if let Some(s) = &mut self.standup {
                    s.commit_cursor = 0;
                }
            }
            KeyCode::Char('G') => {
                if let Some(s) = &mut self.standup {
                    s.commit_cursor = s.focused_len().saturating_sub(1);
                }
            }
            _ => {}
        }
        Cmd::None
    }

    /// Move the standup focus by `delta` repos, clamped, resetting the commit
    /// scroll to the top of the newly-focused repo.
    fn standup_focus_step(&mut self, delta: isize) {
        if let Some(s) = &mut self.standup {
            if s.groups.is_empty() {
                return;
            }
            let last = s.groups.len() - 1;
            s.focus = (s.focus as isize + delta).clamp(0, last as isize) as usize;
            // New repo → start at its first commit, scrolled to the top.
            s.commit_cursor = 0;
            s.set_offset(0);
        }
    }

    /// Move the highlighted commit within the focused repo, clamped to its range.
    fn move_commit_cursor(&mut self, delta: isize) {
        if let Some(s) = &mut self.standup {
            let n = s.focused_len();
            if n == 0 {
                return;
            }
            s.commit_cursor = (s.commit_cursor as isize + delta).clamp(0, n as isize - 1) as usize;
        }
    }

    fn on_key_command_run(&mut self, key: KeyEvent) -> Cmd {
        let max = self.run.as_ref().map_or(0, |r| r.max_scroll.get());
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
            KeyCode::Down | KeyCode::Char('j') => self.run_focus_step(1),
            KeyCode::Up | KeyCode::Char('k') => self.run_focus_step(-1),
            KeyCode::PageDown | KeyCode::Char(' ') => {
                if let Some(run) = &mut self.run {
                    run.scroll = run.scroll.saturating_add(10).min(max);
                }
            }
            KeyCode::PageUp => {
                if let Some(run) = &mut self.run {
                    run.scroll = run.scroll.saturating_sub(10);
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                if let Some(run) = &mut self.run {
                    run.scroll = 0;
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                if let Some(run) = &mut self.run {
                    run.scroll = max;
                }
            }
            KeyCode::Char('y') => return Cmd::CopyRunOutput,
            _ => {}
        }
        Cmd::None
    }

    fn on_key_confirm(&mut self, key: KeyEvent) -> Cmd {
        // Default is No: only an explicit y/Y proceeds; anything else cancels.
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.mode = Mode::Normal;
                Cmd::ConfirmAccept // tui.rs reads `self.confirm` to run it
            }
            _ => {
                self.mode = Mode::Normal;
                self.confirm = None;
                Cmd::None
            }
        }
    }

    /// Open the confirmation modal for stashing the target repos' changes.
    fn request_bulk_stash(&mut self) {
        let ids: Vec<RepoId> = self
            .action_targets()
            .into_iter()
            .filter(|id| self.repos.iter().any(|r| &r.id == id && !r.has_error()))
            .collect();
        if ids.is_empty() {
            self.status = Some("no repos to stash".to_string());
            return;
        }
        let n = ids.len();
        let s = if n == 1 { "" } else { "s" };
        self.confirm = Some(Pending {
            prompt: format!("Stash changes in {n} repo{s}?"),
            action: ConfirmAction::BulkStash(ids),
        });
        self.mode = Mode::Confirm;
    }

    /// Open the confirmation modal for committing the target repos' changes with
    /// `message` (mirrors [`Self::request_bulk_stash`] — commit is confirm-gated
    /// since it writes history, ADR-008).
    fn request_bulk_commit(&mut self, message: String) {
        let ids: Vec<RepoId> = self
            .action_targets()
            .into_iter()
            .filter(|id| self.repos.iter().any(|r| &r.id == id && !r.has_error()))
            .collect();
        if ids.is_empty() {
            self.status = Some("no repos to commit".to_string());
            return;
        }
        let n = ids.len();
        let s = if n == 1 { "" } else { "s" };
        self.confirm = Some(Pending {
            prompt: format!("Commit changes in {n} repo{s}?"),
            action: ConfirmAction::BulkCommit { ids, message },
        });
        self.mode = Mode::Confirm;
    }

    /// Move the run-view focus by `delta` repos, clamped, resetting the output
    /// scroll to the top of the newly-focused repo.
    fn run_focus_step(&mut self, delta: isize) {
        if let Some(run) = &mut self.run {
            if run.results.is_empty() {
                return;
            }
            let last = run.results.len() - 1;
            let next = (run.focus as isize + delta).clamp(0, last as isize) as usize;
            run.focus = next;
            run.scroll = 0;
        }
    }

    /// Handle a mouse-wheel / trackpad scroll. Reversed to match the user's
    /// trackpad direction: a `ScrollUp` event moves toward the bottom, a
    /// `ScrollDown` event toward the top.
    pub fn on_mouse_scroll(&mut self, scroll_up: bool) {
        let toward_top = !scroll_up;
        match self.mode {
            Mode::Normal | Mode::Filter => {
                if toward_top {
                    self.move_up();
                } else {
                    self.move_down();
                }
            }
            Mode::Standup => {
                // Wheel moves the highlighted commit (scroll follows it).
                self.move_commit_cursor(if toward_top { -3 } else { 3 });
            }
            Mode::CommandRun => {
                if let Some(run) = &mut self.run {
                    let max = run.max_scroll.get();
                    run.scroll = if toward_top {
                        run.scroll.saturating_sub(3)
                    } else {
                        run.scroll.saturating_add(3).min(max)
                    };
                }
            }
            _ => {}
        }
    }

    fn move_down(&mut self) {
        let n = self.visible_len();
        if n > 0 {
            self.selected = (self.selected + 1).min(n - 1);
        }
    }

    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn select_last(&mut self) {
        self.selected = self.visible_len().saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cohors_core::{Branch, CommitMeta, RepoId, WorktreeStatus};

    fn snap(name: &str, dirty: bool) -> RepoSnapshot {
        RepoSnapshot {
            id: RepoId(name.to_string()),
            name: name.to_string(),
            path: None,
            branch: Branch::Named("main".to_string()),
            upstream: None,
            worktree: if dirty {
                WorktreeStatus {
                    staged: 0,
                    modified: 1,
                    untracked: 0,
                }
            } else {
                WorktreeStatus::default()
            },
            stash_count: 0,
            stash_latest: None,
            remote_url: None,
            remote: None,
            last_commit: Some(CommitMeta {
                short_id: "abc1234".to_string(),
                author: "Dev".to_string(),
                timestamp: 1_700_000_000,
                summary: "msg".to_string(),
            }),
            error: None,
            activity: Vec::new(),
            groups: Vec::new(),
        }
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn code(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn app_with(names: &[(&str, bool)]) -> App {
        let mut app = App::new(vec![], String::new());
        app.set_repos(names.iter().map(|(n, d)| snap(n, *d)).collect());
        app
    }

    #[test]
    fn cursor_follows_the_same_repo_across_a_rescan() {
        // `beta` is dirty, so it sorts to the top and is selected.
        let mut app = app_with(&[("alpha", false), ("beta", true)]);
        assert_eq!(app.selected_repo().unwrap().name, "beta");

        // A `--watch` re-scan flips the dirty states, re-sorting the fleet. The
        // cursor must stay on `beta` (so the dock keeps showing it), not on the
        // repo that now happens to sit at the old row index.
        app.set_repos(vec![snap("alpha", true), snap("beta", false)]);
        assert_eq!(app.selected_repo().unwrap().name, "beta");
    }

    #[test]
    fn navigation_moves_and_clamps() {
        let mut app = app_with(&[("a", false), ("b", false), ("c", false)]);
        assert_eq!(app.selected, 0);
        app.on_key(code(KeyCode::Down));
        assert_eq!(app.selected, 1);
        app.on_key(code(KeyCode::End));
        assert_eq!(app.selected, 2);
        app.on_key(code(KeyCode::Down)); // clamp at bottom
        assert_eq!(app.selected, 2);
        app.on_key(code(KeyCode::Home));
        assert_eq!(app.selected, 0);
        app.on_key(code(KeyCode::Up)); // clamp at top
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn sort_key_cycles_modes() {
        let mut app = app_with(&[("a", false)]);
        assert_eq!(app.sort, SortMode::DirtyFirst);
        app.on_key(key('s'));
        assert_eq!(app.sort, SortMode::Recent);
    }

    #[test]
    fn dirty_only_toggle_filters_view() {
        let mut app = app_with(&[("clean", false), ("dirty", true)]);
        assert_eq!(app.visible_len(), 2);
        app.on_key(key('d'));
        assert_eq!(app.visible_len(), 1);
        assert_eq!(app.selected_repo().unwrap().name, "dirty");
    }

    #[test]
    fn space_toggles_selection_membership() {
        let mut app = app_with(&[("a", false), ("b", false)]);
        let current = app.selected_repo().unwrap().id.clone();
        app.on_key(key(' '));
        assert!(app.selection.contains(&current));
        app.on_key(key(' '));
        assert!(!app.selection.contains(&current));
    }

    #[test]
    fn select_all_then_toggle_clears() {
        let mut app = app_with(&[("a", false), ("b", false), ("c", false)]);
        app.on_key(key('a'));
        assert_eq!(app.selection.len(), 3);
        app.on_key(key('a')); // already all marked → clears
        assert!(app.selection.is_empty());
    }

    #[test]
    fn esc_clears_selection() {
        let mut app = app_with(&[("a", false)]);
        app.on_key(key('a'));
        assert_eq!(app.selection.len(), 1);
        app.on_key(code(KeyCode::Esc));
        assert!(app.selection.is_empty());
    }

    #[test]
    fn error_repo_cannot_be_marked() {
        let mut app = App::new(vec![], String::new());
        let mut bad = snap("bad", false);
        bad.error = Some("unreadable".to_string());
        app.set_repos(vec![bad]);
        app.on_key(key(' '));
        assert!(app.selection.is_empty());
    }

    #[test]
    fn action_targets_falls_back_to_current_when_unmarked() {
        let app = app_with(&[("a", false), ("b", false)]);
        let current = app.selected_repo().unwrap().id.clone();
        assert_eq!(app.action_targets(), vec![current]);
    }

    #[test]
    fn action_targets_is_marked_intersect_visible_in_view_order() {
        let mut app = app_with(&[("clean", false), ("dirty", true)]);
        app.selection.insert(RepoId("clean".to_string()));
        app.selection.insert(RepoId("dirty".to_string()));
        // The dirty-only filter hides "clean", so only "dirty" is targetable.
        app.on_key(key('d'));
        assert_eq!(app.action_targets(), vec![RepoId("dirty".to_string())]);
    }

    #[test]
    fn selection_survives_set_repos_and_dead_ids_drop() {
        let mut app = app_with(&[("a", false), ("b", false)]);
        app.selection.insert(RepoId("a".to_string()));
        // Re-scan with the same ids reordered: the mark persists.
        app.set_repos(vec![snap("b", false), snap("a", false)]);
        assert_eq!(app.action_targets(), vec![RepoId("a".to_string())]);
        // Re-scan dropping "a": it's no longer a target (intersect with view).
        app.set_repos(vec![snap("b", false)]);
        assert!(app.action_targets().is_empty());
    }

    #[test]
    fn bang_opens_command_palette_prefilled() {
        let mut app = app_with(&[("a", false)]);
        app.on_key(key('!'));
        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.command_line, "!");
    }

    #[test]
    fn command_input_enter_runs_nonempty() {
        let mut app = app_with(&[("a", false)]);
        app.on_key(key('!'));
        for c in "git status".chars() {
            app.on_key(key(c));
        }
        let cmd = app.on_key(code(KeyCode::Enter));
        assert_eq!(cmd, Cmd::RunCommand);
        assert_eq!(app.mode, Mode::CommandRun);
        assert_eq!(app.command_input, "git status");
    }

    #[test]
    fn bang_then_empty_enter_cancels() {
        let mut app = app_with(&[("a", false)]);
        app.on_key(key('!'));
        let cmd = app.on_key(code(KeyCode::Enter));
        assert_eq!(cmd, Cmd::None);
        assert_eq!(app.mode, Mode::Normal);
    }

    fn run_result(name: &str, state: RunState) -> RunResult {
        RunResult {
            id: RepoId(name.to_string()),
            name: name.to_string(),
            state,
        }
    }

    #[test]
    fn command_run_summary_counts() {
        let done = |code| RunState::Done {
            code,
            stdout: String::new(),
            stderr: String::new(),
        };
        let run = CommandRun::new(
            1,
            "x".to_string(),
            vec![
                run_result("a", done(0)),
                run_result("b", done(1)),
                run_result("c", RunState::Running),
            ],
        );
        assert_eq!(run.summary(), (1, 1, 1));
    }

    #[test]
    fn run_focus_step_moves_and_clamps() {
        let mut app = app_with(&[("a", false)]);
        app.run = Some(CommandRun::new(
            1,
            "x".to_string(),
            vec![
                run_result("a", RunState::Running),
                run_result("b", RunState::Running),
            ],
        ));
        app.mode = Mode::CommandRun;
        app.on_key(code(KeyCode::Up)); // already at top → clamp
        assert_eq!(app.run.as_ref().unwrap().focus, 0);
        app.on_key(code(KeyCode::Down));
        assert_eq!(app.run.as_ref().unwrap().focus, 1);
        app.on_key(code(KeyCode::Down)); // at bottom → clamp
        assert_eq!(app.run.as_ref().unwrap().focus, 1);
    }

    #[test]
    fn mouse_scroll_is_reversed() {
        // ScrollDown moves the cursor toward the top, ScrollUp toward the bottom.
        let mut app = app_with(&[("a", false), ("b", false), ("c", false)]);
        app.on_key(code(KeyCode::End)); // cursor at bottom
        let bottom = app.selected;
        app.on_mouse_scroll(false); // ScrollDown → up
        assert!(app.selected < bottom);
        app.on_mouse_scroll(true); // ScrollUp → down
        assert_eq!(app.selected, bottom);
    }

    #[test]
    fn stash_key_opens_confirm_not_action() {
        let mut app = app_with(&[("a", true)]);
        let cmd = app.on_key(key('S'));
        assert_eq!(cmd, Cmd::None); // opens the modal, runs nothing yet
        assert_eq!(app.mode, Mode::Confirm);
        assert!(app.confirm.is_some());
    }

    #[test]
    fn confirm_y_accepts_and_keeps_pending_for_loop() {
        let mut app = app_with(&[("a", true)]);
        app.on_key(key('S'));
        let cmd = app.on_key(key('y'));
        assert_eq!(cmd, Cmd::ConfirmAccept);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.confirm.is_some()); // the loop consumes it via take()
    }

    #[test]
    fn confirm_default_is_no_n_esc_and_other_cancel() {
        for cancel in [key('n'), code(KeyCode::Esc), key('x')] {
            let mut app = app_with(&[("a", true)]);
            app.on_key(key('S'));
            let cmd = app.on_key(cancel);
            assert_eq!(cmd, Cmd::None);
            assert_eq!(app.mode, Mode::Normal);
            assert!(app.confirm.is_none());
        }
    }

    #[test]
    fn filter_narrows_then_esc_clears() {
        let mut app = app_with(&[("payments", false), ("auth", false)]);
        app.on_key(key('/'));
        assert_eq!(app.mode, Mode::Filter);
        for c in "pay".chars() {
            app.on_key(key(c));
        }
        assert_eq!(app.visible_len(), 1);
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.filter.is_empty());
        assert_eq!(app.visible_len(), 2);
    }

    #[test]
    fn quit_on_q_and_ctrl_c() {
        let mut app = app_with(&[("a", false)]);
        assert_eq!(app.on_key(key('q')), Cmd::Quit);
        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Cmd::Quit
        );
    }

    #[test]
    fn refresh_key_requests_rescan() {
        let mut app = app_with(&[("a", false)]);
        assert_eq!(app.on_key(key('r')), Cmd::Refresh);
    }

    #[test]
    fn action_keys_map_to_commands() {
        let mut app = app_with(&[("a", false)]);
        assert_eq!(app.on_key(key('f')), Cmd::FetchSelected);
        assert_eq!(app.on_key(key('F')), Cmd::FetchAll);
        assert_eq!(app.on_key(key('p')), Cmd::PullSelected);
        assert_eq!(app.on_key(key('P')), Cmd::PushSelected);
        assert_eq!(app.on_key(key('o')), Cmd::OpenWith);
        assert_eq!(app.on_key(key('L')), Cmd::Lazygit);
        assert_eq!(app.on_key(key('y')), Cmd::CopyPath);
        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Cmd::OpenDetail
        );
    }

    /// Structural parity, TUI half: every action in the shared registry must be
    /// reachable from the TUI (the MCP half is enforced in `cohors-mcp`). Adding a
    /// verb to `cohors_actions::registry()` without wiring the TUI fails here.
    #[test]
    fn registry_verbs_are_all_wired_in_the_tui() {
        for def in cohors_actions::registry() {
            assert!(
                super::verb_binding(def.verb).is_some(),
                "TUI has no binding for action `{}`",
                def.verb
            );
        }
        // The confirm-gated verbs really go through the modal path, not a Cmd.
        assert_eq!(
            super::verb_binding("commit"),
            Some(super::VerbBinding::Confirmed)
        );
        assert_eq!(
            super::verb_binding("stash"),
            Some(super::VerbBinding::Confirmed)
        );
    }

    #[test]
    fn empty_picker_offers_use_suggested() {
        let mut app = App::new(vec!["~/projects".to_string()], "cfg".to_string());
        // Empty fleet, not scanning, repos detected elsewhere → rescue is live.
        app.suggested_roots = vec!["~/code".to_string()];
        assert!(app.empty_picker_active());
        assert_eq!(app.on_key(key('u')), Cmd::UseSuggestedRoots);

        // No suggestions → the picker is inactive and `u` is inert.
        app.suggested_roots.clear();
        assert!(!app.empty_picker_active());
        assert_eq!(app.on_key(key('u')), Cmd::None);
    }

    #[test]
    fn help_toggles() {
        let mut app = app_with(&[("a", false)]);
        app.on_key(key('?'));
        assert_eq!(app.mode, Mode::Help);
        app.on_key(key('?'));
        assert_eq!(app.mode, Mode::Normal);
    }
}
