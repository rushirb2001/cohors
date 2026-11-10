//! Dashboard state and key handling.
//!
//! The state is deliberately small and rendering-free: the ordered, filtered
//! view is *derived* each frame via [`cohors_core::compute_view`], so the
//! "what to show, in what order" logic stays in the shared core. Key handling
//! returns a [`Cmd`] telling the event loop what side effect to run (quit,
//! rescan), keeping I/O out of the state.

use std::collections::{HashMap, HashSet};

use cohors_core::launchpad::LaunchpadEntry;
use cohors_core::{
    ActionKind, NextAction, Overview, RemoteDetail, RepoDetail, RepoId, RepoLine, RepoSnapshot,
    SortMode, StandupCommit, StandupWindow, ViewParams, ViewRow, compute_view, group_commits,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Which screen sits underneath the overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// The overview: the header, the key figures and the five numbered cards.
    /// This is what cohors opens on.
    Overview,
    /// The detail view: every repository as a two-line list, with three cards
    /// about the selected one underneath. Opened from card 5 with Enter.
    Repos,
}

/// The five numbered cards of the overview. The number is the key that focuses
/// the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Card {
    DoNext,
    AtRisk,
    Tasks,
    Commits,
    Repos,
}

impl Card {
    /// Every card, in reading order (also the order Tab cycles through).
    pub const ALL: [Card; 5] = [
        Card::DoNext,
        Card::AtRisk,
        Card::Tasks,
        Card::Commits,
        Card::Repos,
    ];

    /// The number shown in the card's title: 1 to 5.
    pub fn number(self) -> u8 {
        match self {
            Card::DoNext => 1,
            Card::AtRisk => 2,
            Card::Tasks => 3,
            Card::Commits => 4,
            Card::Repos => 5,
        }
    }

    /// The card a number key focuses, if the key is `1` to `5`.
    pub fn from_key(key: char) -> Option<Card> {
        Card::ALL
            .into_iter()
            .find(|card| char::from(b'0' + card.number()) == key)
    }

    /// The card `step` places further along, wrapping at both ends.
    fn cycle(self, step: isize) -> Card {
        let count = Card::ALL.len() as isize;
        let index = isize::from(self.number()) - 1;
        Card::ALL[(index + step).rem_euclid(count) as usize]
    }
}

/// The four cards of the detail view: the list, and three about the selected
/// repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    List,
    State,
    Signals,
    Tasks,
}

impl Pane {
    const ALL: [Pane; 4] = [Pane::List, Pane::State, Pane::Signals, Pane::Tasks];

    fn cycle(self, step: isize) -> Pane {
        let count = Pane::ALL.len() as isize;
        let index = Pane::ALL.iter().position(|p| *p == self).unwrap_or(0) as isize;
        Pane::ALL[(index + step).rem_euclid(count) as usize]
    }
}

/// A git action that runs over several repositories with no question asked.
/// Fetch, fast-forward pull and push cannot lose local work, which is why they
/// are the only three here; commit and stash go through a confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkVerb {
    Fetch,
    Pull,
    Push,
}

impl BulkVerb {
    /// The typed verb for an action-registry verb name, if it is one of the
    /// three that run without a confirmation.
    fn from_registry(verb: &str) -> Option<Self> {
        match verb {
            "fetch" => Some(BulkVerb::Fetch),
            "pull" => Some(BulkVerb::Pull),
            "push" => Some(BulkVerb::Push),
            _ => None,
        }
    }
}

/// A set of repositories the table is narrowed to, e.g. the ones a "Do next"
/// step is about. `Esc` on the table clears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// What the set is, in a few words: `Review stashes`.
    pub label: String,
    pub ids: HashSet<RepoId>,
}

/// The cursor row of each overview card that is not the table (the table's
/// cursor is [`App::selected`], which the detail view shares).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursors {
    pub next: usize,
    pub risk: usize,
    pub tasks: usize,
    pub commits: usize,
}

/// The first visible row of each scrolling list, remembered between frames so
/// a list only scrolls when the cursor would leave it. The renderer owns these
/// (it is the one that knows how many rows fit), hence the `Cell`s.
#[derive(Debug, Default)]
pub struct Scrolls {
    pub next: std::cell::Cell<usize>,
    pub risk: std::cell::Cell<usize>,
    pub tasks: std::cell::Cell<usize>,
    pub commits: std::cell::Cell<usize>,
    /// The table in card 5.
    pub table: std::cell::Cell<usize>,
    /// The two-line list of the detail view.
    pub list: std::cell::Cell<usize>,
    /// How far the focused card of the detail view may scroll (set each frame).
    pub pane_max: std::cell::Cell<usize>,
}

/// The key shown beside a "Do next" step, and the key that carries it out.
///
/// A letter is one verb on every card: `f` fetches, `p` pulls, `P` pushes,
/// `t` opens the tasks. So a step has a letter only when it IS one of those
/// verbs, and the letter is the same one the repository table uses. Push and
/// Publish share `P` because both are a push. A step that needs a person to
/// look (conflicts, a diverged branch, stashes, uncommitted files) has no
/// letter: `S` and `c` mean "stash" and "commit" everywhere, and a step that
/// only shows you the repositories must not borrow them. Those steps are
/// opened with Enter.
pub fn action_key(kind: ActionKind) -> Option<char> {
    match kind {
        ActionKind::Push | ActionKind::Publish => Some('P'),
        ActionKind::Pull => Some('p'),
        ActionKind::Fetch => Some('f'),
        ActionKind::StartTask => Some('t'),
        ActionKind::Unreadable
        | ActionKind::Resolve
        | ActionKind::Reconcile
        | ActionKind::ReviewStash
        | ActionKind::Commit => None,
    }
}

/// What a repository key does, in a word, or `None` if `key` is not one. It
/// names the keys [`App::on_key_repo`] answers to, so "Do next" (whose rows
/// are steps, not repositories) can say why such a key did nothing there.
fn repo_key_verb(key: char) -> Option<&'static str> {
    Some(match key {
        'f' => "fetch",
        'p' => "pull",
        'P' => "push",
        'c' => "commit",
        'S' => "stash",
        'a' => "set up",
        'o' => "open",
        'L' => "open in lazygit",
        'y' => "copy the path of",
        _ => return None,
    })
}

/// Which input mode the dashboard is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// No overlay: keys go to the current [`Screen`].
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
    /// The "push which branch?" picker for a repo with several local branches.
    PushBranch,
    /// The per-repo drill-in detail pane (commits / files / branches / stashes).
    Detail,
    /// The launchpad view: the focus board and open tasks across the fleet.
    Launchpad,
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
    /// Open the launchpad view (read every repo's `.cohors/` contribution).
    OpenLaunchpad,
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
    /// Push the branch highlighted in the multi-branch push picker.
    PushBranchAccept,
    /// Adopt the auto-detected roots from the empty-state rescue prompt.
    UseSuggestedRoots,
    /// Run a bulk git action over exactly these repositories — a "Do next"
    /// step, whose targets come from the overview rather than the selection.
    Act {
        verb: BulkVerb,
        ids: Vec<RepoId>,
    },
    /// Collect the standup and copy it as markdown without opening the view.
    CopyStandupMarkdown,
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
    /// Hand this repo to the user's coding agent with the `setup-repo` prompt
    /// (`cohors agent run setup-repo`). Not destructive, but it starts an agent
    /// session with a prompt already submitted, so it asks first.
    AgentSetup(RepoId),
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

/// The launchpad view: every repo's contribution plus the focus board, shown
/// as one scrolling column.
#[derive(Debug, Clone)]
pub struct LaunchpadView {
    pub entries: Vec<cohors_core::launchpad::LaunchpadEntry>,
    pub board: cohors_core::launchpad::Board,
    /// First visible line.
    pub scroll: usize,
    /// The furthest `scroll` may go. Set by the renderer (which knows the
    /// viewport height) through a `Cell`, so key handling can clamp without
    /// knowing the layout.
    pub max_scroll: std::cell::Cell<usize>,
}

impl LaunchpadView {
    pub fn new(
        entries: Vec<cohors_core::launchpad::LaunchpadEntry>,
        board: cohors_core::launchpad::Board,
    ) -> Self {
        Self {
            entries,
            board,
            scroll: 0,
            max_scroll: std::cell::Cell::new(0),
        }
    }
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
    /// Unix seconds, refreshed once per event-loop tick — the single clock the
    /// view filter and renderers share (WASM-safe core takes it injected).
    pub now: i64,
    pub repos: Vec<RepoSnapshot>,
    /// The shared overview model: "Do next", the key figures, one line per
    /// repository, tasks and commits. Computed off the UI thread after every
    /// scan, so it is `None` for the first moments of a run.
    pub overview: Option<Overview>,
    /// Each repository's launchpad entry (task titles and run commands), read
    /// alongside the overview. The detail view's tasks card shows them.
    pub entries: Vec<LaunchpadEntry>,
    /// When the fleet was last scanned (Unix seconds), for the header.
    pub scanned_at: Option<i64>,
    /// Which screen is showing under any overlay.
    pub screen: Screen,
    /// The focused card of the overview.
    pub focus: Card,
    /// The focused card of the detail view.
    pub pane: Pane,
    /// Cursor rows of the overview's cards 1 to 4.
    pub cursors: Cursors,
    /// First visible rows of the scrolling lists (owned by the renderer).
    pub scrolls: Scrolls,
    /// Scroll offset of the focused card in the detail view.
    pub pane_scroll: usize,
    /// When set, the table lists only these repositories.
    pub scope: Option<Scope>,
    /// The repository the standup should open on (set by Enter in card 4).
    pub standup_jump: Option<String>,
    /// The repository the launchpad should open on (set by Enter in card 3).
    pub launchpad_jump: Option<String>,
    /// `y` in card 4 was pressed: copy the standup as soon as it is collected.
    pub copy_standup_pending: bool,
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
    /// The launchpad view (`None` while loading, or not opened).
    pub launchpad: Option<LaunchpadView>,
    /// The command being typed in `CommandInput` mode (mirrors `filter`).
    pub command_input: String,
    /// The `:`-command being typed in `Command` mode.
    pub command_line: String,
    /// The command line's cursor, the lines run before, and a tab completion
    /// in progress (the text itself is `command_line`).
    pub command_edit: crate::command::LineState,
    /// The in-flight / last command run, shown in `CommandRun` mode.
    pub run: Option<CommandRun>,
    /// The destructive action awaiting confirmation (`Some` ⇒ `Mode::Confirm`).
    pub confirm: Option<Pending>,
    /// The "Open with…" picker (`Some` ⇒ `Mode::OpenWith`).
    pub open_with: Option<OpenWith>,
    /// The "push which branch?" picker (`Some` ⇒ `Mode::PushBranch`).
    pub push_pick: Option<PushPick>,
    /// The drill-in detail pane (`Some` ⇒ `Mode::Detail`).
    pub detail: Option<DetailView>,
    /// Repos auto-detected elsewhere when the configured roots came up empty —
    /// drives the first-run rescue prompt in the empty state. Empty otherwise.
    pub suggested_roots: Vec<String>,
    /// Configured glyph mode for the dashboard (auto / ascii / unicode / nerd).
    pub icons: cohors_config::IconMode,
    /// Whether to treat the terminal's background as dark. A guess unless
    /// `ground` is known.
    pub dark: bool,
    /// The terminal's actual background colour, when it is known (it answered
    /// the question at startup, or the user's `theme` setting says which kind
    /// it is). `None` means not known: the dashboard then paints nothing that
    /// would only look right on one kind of background.
    pub ground: Option<(u8, u8, u8)>,
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

/// State for the "push which branch?" picker: the repo it targets, its local
/// branches (current first), and the highlighted row. The repo's path and name are
/// resolved from `App::repos` by `id` when the push actually runs.
pub struct PushPick {
    pub id: RepoId,
    pub name: String,
    pub branches: Vec<cohors_core::BranchInfo>,
    pub cursor: usize,
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
            now: 0,
            repos: Vec::new(),
            overview: None,
            entries: Vec::new(),
            scanned_at: None,
            screen: Screen::Overview,
            focus: Card::DoNext,
            pane: Pane::List,
            cursors: Cursors::default(),
            scrolls: Scrolls::default(),
            pane_scroll: 0,
            scope: None,
            standup_jump: None,
            launchpad_jump: None,
            copy_standup_pending: false,
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
            launchpad: None,
            command_input: String::new(),
            command_line: String::new(),
            command_edit: crate::command::LineState::default(),
            run: None,
            confirm: None,
            open_with: None,
            push_pick: None,
            detail: None,
            suggested_roots: Vec::new(),
            icons: cohors_config::IconMode::default(),
            dark: false,
            ground: None,
        }
    }

    /// True when the fleet is empty but we detected repos elsewhere, so the
    /// empty state offers a one-key rescue.
    pub fn empty_picker_active(&self) -> bool {
        self.repos.is_empty() && !self.scanning && !self.suggested_roots.is_empty()
    }

    /// The ordered, filtered rows to render this frame.
    pub fn view(&self) -> Vec<ViewRow> {
        let mut rows = compute_view(
            &self.repos,
            &ViewParams {
                sort: self.sort,
                dirty_only: self.dirty_only,
                query: &self.filter,
                now: self.now,
            },
        );
        if let Some(scope) = &self.scope {
            rows.retain(|row| scope.ids.contains(&self.repos[row.index].id));
        }
        // With the default sort and nothing typed, list repositories in the
        // overview's own order (most urgent first, then by name) so the table
        // agrees with "Work at risk" above it. The sort is stable, so a
        // repository the overview has not seen yet keeps its place at the end.
        if self.sort == SortMode::default()
            && self.filter.trim().is_empty()
            && let Some(overview) = &self.overview
        {
            let rank: HashMap<&str, usize> = overview
                .repos
                .iter()
                .enumerate()
                .map(|(i, line)| (line.id.as_str(), i))
                .collect();
            rows.sort_by_key(|row| {
                let id = self.repos[row.index].id.0.as_str();
                rank.get(id).copied().unwrap_or(usize::MAX)
            });
        }
        rows
    }

    /// The overview's line for a repository: its severity, reasons, suggested
    /// step and task counts. `None` until the overview has been computed.
    pub fn line(&self, id: &RepoId) -> Option<&RepoLine> {
        self.overview
            .as_ref()?
            .repos
            .iter()
            .find(|line| line.id == id.0)
    }

    /// The launchpad entry (task titles, run commands) for a repository.
    pub fn entry(&self, id: &RepoId) -> Option<&LaunchpadEntry> {
        self.entries.iter().find(|entry| entry.id == id.0)
    }

    /// The repositories in "Work at risk", most urgent first.
    pub fn at_risk(&self) -> Vec<&RepoLine> {
        self.overview
            .as_ref()
            .map(|overview| overview.at_risk().collect())
            .unwrap_or_default()
    }

    /// The "Do next" steps (empty until the overview has been computed).
    pub fn actions(&self) -> &[NextAction] {
        self.overview
            .as_ref()
            .map_or(&[], |overview| overview.actions.as_slice())
    }

    /// Install a freshly computed overview and the launchpad entries it was
    /// built from, keeping every cursor on a row that still exists. The table
    /// cursor stays on the same repository even though the order may change.
    pub fn set_overview(&mut self, overview: Overview, entries: Vec<LaunchpadEntry>) {
        // A cursor still resting on the first row has not been moved by the
        // person, so it stays on the first row of the new order (the most
        // urgent repository) rather than following whatever was there before.
        let untouched = self.overview.is_none() && self.selected == 0;
        let keep = self.selected_repo().map(|r| r.id.clone());
        self.overview = Some(overview);
        self.entries = entries;
        if !untouched {
            self.restore_selection(keep);
        }
        self.clamp_selection();
        self.clamp_cursors();
    }

    /// Keep the four card cursors inside their lists.
    fn clamp_cursors(&mut self) {
        let clamp = |cursor: usize, len: usize| cursor.min(len.saturating_sub(1));
        let (next, risk, tasks, commits) = (
            self.actions().len(),
            self.at_risk().len(),
            self.overview.as_ref().map_or(0, |o| o.work.len()),
            self.overview.as_ref().map_or(0, |o| o.commits.len()),
        );
        self.cursors.next = clamp(self.cursors.next, next);
        self.cursors.risk = clamp(self.cursors.risk, risk);
        self.cursors.tasks = clamp(self.cursors.tasks, tasks);
        self.cursors.commits = clamp(self.cursors.commits, commits);
    }

    /// Put the table cursor back on the repository `id`, if it is still listed.
    fn restore_selection(&mut self, id: Option<RepoId>) {
        if let Some(id) = id
            && let Some(pos) = self
                .view()
                .iter()
                .position(|row| self.repos[row.index].id == id)
        {
            self.selected = pos;
        }
        self.clamp_selection();
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

    /// Whether the cursor that matters is the one on "Work at risk".
    fn on_at_risk_card(&self) -> bool {
        self.mode != Mode::Filter && self.screen == Screen::Overview && self.focus == Card::AtRisk
    }

    /// The repository an action key acts on: the row under the cursor of the
    /// card that has the focus. That is the "Work at risk" row while that card
    /// is focused, and the repository table's row everywhere else.
    pub fn current_repo(&self) -> Option<&RepoSnapshot> {
        if self.on_at_risk_card() {
            let id = &self.at_risk().get(self.cursors.risk)?.id;
            return self.repos.iter().find(|repo| &repo.id.0 == id);
        }
        self.selected_repo()
    }

    /// The repos an action should operate on: the marked selection (intersected
    /// with what's currently visible, in view order) if any are marked, else the
    /// repo under the cursor. Callers still drop error/path-less repos, exactly
    /// as the single-repo action paths already do.
    pub fn action_targets(&self) -> Vec<RepoId> {
        // Marks belong to the repository table. On "Work at risk" an action
        // is always about the row the cursor is on there.
        if self.selection.is_empty() || self.on_at_risk_card() {
            return self
                .current_repo()
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
        self.restore_selection(keep);
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
            Mode::PushBranch => self.on_key_push_branch(key),
            Mode::Detail => self.on_key_detail(key),
            Mode::Launchpad => self.on_key_launchpad(key),
        }
    }

    /// Switch to the launchpad view; the event loop loads its data.
    fn open_launchpad(&mut self) -> Cmd {
        self.mode = Mode::Launchpad;
        self.launchpad = None;
        Cmd::OpenLaunchpad
    }

    fn on_key_launchpad(&mut self, key: KeyEvent) -> Cmd {
        let Some(view) = &mut self.launchpad else {
            // Still loading: only closing makes sense.
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q' | 't')) {
                self.mode = Mode::Normal;
            }
            return Cmd::None;
        };
        let max = view.max_scroll.get();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 't') => self.mode = Mode::Normal,
            KeyCode::Down | KeyCode::Char('j') => view.scroll = (view.scroll + 1).min(max),
            KeyCode::Up | KeyCode::Char('k') => view.scroll = view.scroll.saturating_sub(1),
            KeyCode::PageDown => view.scroll = (view.scroll + 10).min(max),
            KeyCode::PageUp => view.scroll = view.scroll.saturating_sub(10),
            KeyCode::Home => view.scroll = 0,
            KeyCode::End => view.scroll = max,
            _ => {}
        }
        Cmd::None
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

    fn on_key_push_branch(&mut self, key: KeyEvent) -> Cmd {
        let n = self.push_pick.as_ref().map_or(0, |p| p.branches.len());
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Normal;
                self.push_pick = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(p) = &mut self.push_pick {
                    p.cursor = p.cursor.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(p) = &mut self.push_pick
                    && p.cursor + 1 < n
                {
                    p.cursor += 1;
                }
            }
            KeyCode::Enter => return Cmd::PushBranchAccept,
            _ => {}
        }
        Cmd::None
    }

    /// Keys with no overlay open go to whichever screen is showing.
    fn on_key_normal(&mut self, key: KeyEvent) -> Cmd {
        if let Some(cmd) = self.on_key_anywhere(key) {
            return cmd;
        }
        match self.screen {
            Screen::Overview => self.on_key_overview(key),
            Screen::Repos => self.on_key_repos_screen(key),
        }
    }

    /// Keys that mean the same thing on both screens and on every card.
    /// Returns `None` when the key is not one of them.
    fn on_key_anywhere(&mut self, key: KeyEvent) -> Option<Cmd> {
        match key.code {
            KeyCode::Char('q') => return Some(Cmd::Quit),
            KeyCode::Char('r') => return Some(Cmd::Refresh),
            // The standup and the launchpad open from anywhere, not only
            // from the cards that summarise them.
            KeyCode::Char('s') => return Some(self.open_standup()),
            KeyCode::Char('t') => return Some(self.open_launchpad()),
            // Fetching everything and copying the standup are about the
            // whole fleet, so they do not depend on where the cursor is.
            KeyCode::Char('F') => return Some(Cmd::FetchAll),
            KeyCode::Char('Y') => {
                self.copy_standup_pending = true;
                return Some(Cmd::CopyStandupMarkdown);
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.command_line.clear();
                self.command_edit.start();
            }
            // `!` is the shell shortcut into the command palette: open it
            // pre-seeded with `!` so the next keystroke starts the command.
            KeyCode::Char('!') => self.open_command_with("!"),
            // Filtering always applies to the repository table.
            KeyCode::Char('/') => {
                self.focus = Card::Repos;
                self.pane = Pane::List;
                self.mode = Mode::Filter;
            }
            // First-run rescue: adopt the auto-detected roots (empty state only).
            KeyCode::Char('u') if self.empty_picker_active() => {
                return Some(Cmd::UseSuggestedRoots);
            }
            // A number key focuses that card, from either screen.
            KeyCode::Char(c) if Card::from_key(c).is_some() => {
                self.focus = Card::from_key(c).unwrap_or(self.focus);
                self.screen = Screen::Overview;
            }
            _ => return None,
        }
        Some(Cmd::None)
    }

    /// Open the command palette with `text` already typed.
    fn open_command_with(&mut self, text: &str) {
        self.mode = Mode::Command;
        self.command_line = text.to_string();
        self.command_edit.start();
    }

    // ── The overview ─────────────────────────────────────────────────────────

    fn on_key_overview(&mut self, key: KeyEvent) -> Cmd {
        match key.code {
            KeyCode::Tab => self.focus = self.focus.cycle(1),
            KeyCode::BackTab => self.focus = self.focus.cycle(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::PageDown => self.move_cursor(10),
            KeyCode::PageUp => self.move_cursor(-10),
            KeyCode::Home => self.move_cursor(isize::MIN / 2),
            KeyCode::End => self.move_cursor(isize::MAX / 2),
            _ => {
                return match self.focus {
                    Card::DoNext => self.on_key_do_next(key),
                    Card::AtRisk => self.on_key_at_risk(key),
                    Card::Tasks => self.on_key_tasks(key),
                    Card::Commits => self.on_key_commits(key),
                    Card::Repos => self.on_key_repos_card(key),
                };
            }
        }
        Cmd::None
    }

    /// How many rows the focused overview card has.
    fn focused_len(&self) -> usize {
        match self.focus {
            Card::DoNext => self.actions().len(),
            Card::AtRisk => self.at_risk().len(),
            Card::Tasks => self.overview.as_ref().map_or(0, |o| o.work.len()),
            Card::Commits => self.overview.as_ref().map_or(0, |o| o.commits.len()),
            Card::Repos => self.visible_len(),
        }
    }

    /// Move the focused card's cursor by `delta` rows, stopping at both ends.
    fn move_cursor(&mut self, delta: isize) {
        let last = self.focused_len().saturating_sub(1) as isize;
        let cursor = match self.focus {
            Card::DoNext => &mut self.cursors.next,
            Card::AtRisk => &mut self.cursors.risk,
            Card::Tasks => &mut self.cursors.tasks,
            Card::Commits => &mut self.cursors.commits,
            Card::Repos => &mut self.selected,
        };
        *cursor = (*cursor as isize).saturating_add(delta).clamp(0, last) as usize;
    }

    /// Card 1: Enter carries out the highlighted step; a step's own key
    /// carries out every step that lists that key.
    fn on_key_do_next(&mut self, key: KeyEvent) -> Cmd {
        match key.code {
            KeyCode::Enter => match self.actions().get(self.cursors.next).cloned() {
                Some(action) => self.perform(&[action]),
                None => Cmd::None,
            },
            KeyCode::Char(c) => {
                let steps = self.steps_for_key(c);
                if !steps.is_empty() {
                    return self.perform(&steps);
                }
                // The rows here are steps, not repositories, so a key that
                // acts on a repository has nothing to act on. Say where it
                // works instead of doing nothing.
                if let Some(verb) = repo_key_verb(c) {
                    self.status = Some(format!(
                        "no step here to {verb}: press 2 or 5 and pick a repository"
                    ));
                }
                Cmd::None
            }
            _ => Cmd::None,
        }
    }

    /// Card 2: Enter opens the highlighted repository in the detail view; the
    /// step keys work as they do in card 1.
    fn on_key_at_risk(&mut self, key: KeyEvent) -> Cmd {
        match key.code {
            KeyCode::Enter => {
                let id = self
                    .at_risk()
                    .get(self.cursors.risk)
                    .map(|line| RepoId(line.id.clone()));
                if let Some(id) = id {
                    self.show_repo(&id);
                }
                Cmd::None
            }
            // Every row is a repository, so the repository keys act on it,
            // exactly as they do in the table.
            _ => self.on_key_repo(key).unwrap_or(Cmd::None),
        }
    }

    /// Card 3: `t` opens the launchpad at the focus board; Enter opens it at
    /// the highlighted repository's tasks.
    fn on_key_tasks(&mut self, key: KeyEvent) -> Cmd {
        match key.code {
            KeyCode::Char('t') => self.open_launchpad(),
            KeyCode::Enter => {
                self.launchpad_jump = self
                    .overview
                    .as_ref()
                    .and_then(|o| o.work.get(self.cursors.tasks))
                    .map(|work| work.name.clone());
                self.open_launchpad()
            }
            _ => Cmd::None,
        }
    }

    /// Card 4: `s` (or Enter, on the highlighted repository) opens the
    /// standup; `y` copies it as markdown without opening it.
    fn on_key_commits(&mut self, key: KeyEvent) -> Cmd {
        match key.code {
            KeyCode::Enter => {
                self.standup_jump = self
                    .overview
                    .as_ref()
                    .and_then(|o| o.commits.get(self.cursors.commits))
                    .map(|commits| commits.name.clone());
                self.open_standup()
            }
            _ => Cmd::None,
        }
    }

    /// Card 5: the table keys, plus Enter for the detail view and Esc to peel
    /// back one narrowing at a time.
    fn on_key_repos_card(&mut self, key: KeyEvent) -> Cmd {
        if let Some(cmd) = self.on_key_table(key) {
            return cmd;
        }
        match key.code {
            KeyCode::Enter if self.selected_repo().is_some() => {
                self.screen = Screen::Repos;
                self.pane = Pane::List;
                self.pane_scroll = 0;
            }
            KeyCode::Esc => self.clear_one_narrowing(),
            KeyCode::Char('t') => return self.open_launchpad(),
            _ => {}
        }
        Cmd::None
    }

    /// Esc on the table undoes the most recent narrowing: the marked
    /// selection first, then a "Do next" scope, then the typed filter.
    fn clear_one_narrowing(&mut self) {
        if !self.selection.is_empty() {
            self.selection.clear();
        } else if self.scope.take().is_none() {
            self.filter.clear();
        }
        self.clamp_selection();
    }

    /// The keys the repository table answers to on both screens. Returns
    /// `None` when the key is not a table key.
    fn on_key_table(&mut self, key: KeyEvent) -> Option<Cmd> {
        let cmd = match key.code {
            // Multi-select for bulk actions.
            KeyCode::Char(' ') => {
                self.toggle_selection();
                Cmd::None
            }
            KeyCode::Char('*') => {
                self.select_all_visible();
                Cmd::None
            }
            KeyCode::Char('>') => {
                self.sort = self.sort.next();
                self.clamp_selection();
                Cmd::None
            }
            _ => return self.on_key_repo(key),
        };
        Some(cmd)
    }

    /// The keys that act on a repository: the one under the cursor (or, in
    /// the table, the marked ones). They mean the same thing on "Work at
    /// risk", in the repository table and in the detail view. Returns `None`
    /// when the key is not one of them. Keep [`repo_key_verb`] in step.
    fn on_key_repo(&mut self, key: KeyEvent) -> Option<Cmd> {
        let cmd = match key.code {
            KeyCode::Char('f') => Cmd::FetchSelected,
            KeyCode::Char('p') => Cmd::PullSelected,
            KeyCode::Char('P') => Cmd::PushSelected,
            // A commit needs a message, so `c` opens the command line with
            // `commit ` typed; Enter then asks for confirmation as before.
            KeyCode::Char('c') => {
                self.open_command_with("commit ");
                Cmd::None
            }
            KeyCode::Char('S') => {
                self.request_bulk_stash();
                Cmd::None
            }
            KeyCode::Char('a') => {
                self.request_agent_setup();
                Cmd::None
            }
            KeyCode::Char('o') => Cmd::OpenWith,
            KeyCode::Char('L') => Cmd::Lazygit,
            KeyCode::Char('y') => Cmd::CopyPath,
            _ => return None,
        };
        Some(cmd)
    }

    /// Every "Do next" step whose key is `key` (Push and Publish share one).
    fn steps_for_key(&self, key: char) -> Vec<NextAction> {
        self.actions()
            .iter()
            .filter(|action| action_key(action.kind) == Some(key))
            .cloned()
            .collect()
    }

    /// Carry out one or more "Do next" steps of the same kind of work.
    ///
    /// A step with a fetch, pull or push verb runs that action over exactly the
    /// step's repositories. A stash step asks first. Everything else needs a
    /// person to look, so it navigates: the launchpad for tasks, otherwise the
    /// repository table narrowed to the step's repositories.
    fn perform(&mut self, steps: &[NextAction]) -> Cmd {
        let Some(first) = steps.first() else {
            return Cmd::None;
        };
        // Several steps can share a key (Push and Publish); act on all their
        // repositories once each, in the order the overview lists them.
        let mut seen = HashSet::new();
        let ids: Vec<RepoId> = steps
            .iter()
            .flat_map(|step| step.repos.iter())
            .filter(|id| seen.insert(id.as_str()))
            .map(|id| RepoId(id.clone()))
            .collect();

        if first.kind == ActionKind::StartTask {
            return self.open_launchpad();
        }
        match first.verb.as_deref() {
            Some("stash") => {
                self.confirm_stash(ids);
                Cmd::None
            }
            Some(verb) => match BulkVerb::from_registry(verb) {
                Some(verb) => Cmd::Act { verb, ids },
                None => self.narrow_to(&first.title, ids),
            },
            None => self.narrow_to(&first.title, ids),
        }
    }

    /// Show only `ids` in the repository table and move the focus there.
    fn narrow_to(&mut self, label: &str, ids: Vec<RepoId>) -> Cmd {
        self.scope = Some(Scope {
            label: label.to_string(),
            ids: ids.into_iter().collect(),
        });
        // A leftover filter or the attention toggle could hide the very
        // repositories the step is about.
        self.filter.clear();
        self.dirty_only = false;
        self.selected = 0;
        self.focus = Card::Repos;
        self.screen = Screen::Overview;
        Cmd::None
    }

    /// Open the detail view with the cursor on `id`. If the current filter or
    /// scope hides that repository, they are cleared so it can be shown.
    fn show_repo(&mut self, id: &RepoId) {
        let position = |app: &App| {
            app.view()
                .iter()
                .position(|row| &app.repos[row.index].id == id)
        };
        if position(self).is_none() {
            self.scope = None;
            self.filter.clear();
            self.dirty_only = false;
        }
        let Some(index) = position(self) else {
            self.status = Some("that repository is no longer in the fleet".to_string());
            return;
        };
        self.selected = index;
        self.focus = Card::Repos;
        self.screen = Screen::Repos;
        self.pane = Pane::List;
        self.pane_scroll = 0;
    }

    /// Switch to the standup view; the event loop collects its commits.
    fn open_standup(&mut self) -> Cmd {
        self.mode = Mode::Standup;
        self.standup = None;
        Cmd::OpenStandup
    }

    /// Show collected standup commits, opening on the repository Enter was
    /// pressed on in card 4 when there is one.
    pub fn show_standup(&mut self, commits: Vec<StandupCommit>) {
        let mut view = StandupView::new(commits);
        if let Some(name) = self.standup_jump.take()
            && let Some(index) = view.groups.iter().position(|(repo, _)| *repo == name)
        {
            view.focus = index;
        }
        self.standup = Some(view);
    }

    // ── The detail view ──────────────────────────────────────────────────────

    fn on_key_repos_screen(&mut self, key: KeyEvent) -> Cmd {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Overview,
            KeyCode::Tab => self.focus_pane(self.pane.cycle(1)),
            KeyCode::BackTab => self.focus_pane(self.pane.cycle(-1)),
            KeyCode::Down | KeyCode::Char('j') => self.move_in_detail(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_in_detail(-1),
            KeyCode::PageDown => self.move_in_detail(10),
            KeyCode::PageUp => self.move_in_detail(-10),
            KeyCode::Home => self.move_in_detail(isize::MIN / 2),
            KeyCode::End => self.move_in_detail(isize::MAX / 2),
            // The full drill-in: commits, branches, changed files, stashes.
            KeyCode::Enter | KeyCode::Char('d') => return Cmd::OpenDetail,
            KeyCode::Char('t') => return self.open_launchpad(),
            _ => return self.on_key_table(key).unwrap_or(Cmd::None),
        }
        Cmd::None
    }

    fn focus_pane(&mut self, pane: Pane) {
        self.pane = pane;
        self.pane_scroll = 0;
    }

    /// Up/down in the detail view: move through the list, or scroll the card
    /// that has the focus.
    fn move_in_detail(&mut self, delta: isize) {
        if self.pane == Pane::List {
            let last = self.visible_len().saturating_sub(1) as isize;
            self.selected = (self.selected as isize)
                .saturating_add(delta)
                .clamp(0, last) as usize;
            self.pane_scroll = 0;
        } else {
            let max = self.scrolls.pane_max.get() as isize;
            self.pane_scroll = (self.pane_scroll as isize)
                .saturating_add(delta)
                .clamp(0, max) as usize;
        }
    }

    fn on_key_filter(&mut self, key: KeyEvent) -> Cmd {
        match key.code {
            // The list filters live as you type. Esc cancels (clears + closes).
            KeyCode::Esc => {
                self.filter.clear();
                self.selected = 0;
                self.mode = Mode::Normal;
            }
            // Enter confirms: keep the filtered list, return to navigation.
            KeyCode::Enter => self.mode = Mode::Normal,
            // Backspace deletes a char; backspacing past the empty query removes
            // the leading `/` and closes the filter (nothing left to filter on).
            KeyCode::Backspace => {
                if self.filter.is_empty() {
                    self.mode = Mode::Normal;
                } else {
                    self.filter.pop();
                    self.selected = 0;
                }
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
        // The text and its editing state are two fields, so both can be
        // borrowed at once: `edit` changes `line`.
        let (line, edit) = (&mut self.command_line, &mut self.command_edit);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.command_line.clear();
            }
            // The readline keys people's fingers already know.
            KeyCode::Char('a') if ctrl => edit.home(line),
            KeyCode::Char('e') if ctrl => edit.end(),
            KeyCode::Char('u') if ctrl => edit.kill_before(line),
            KeyCode::Char('w') if ctrl => edit.kill_word(line),
            KeyCode::Left => edit.left(line),
            KeyCode::Right => edit.right(),
            KeyCode::Home => edit.home(line),
            KeyCode::End => edit.end(),
            KeyCode::Delete => edit.delete(line),
            // Up and Down walk the lines run before.
            KeyCode::Up => edit.older(line),
            KeyCode::Down => edit.newer(line),
            // Tab finishes a command, a sort mode or a repository name.
            KeyCode::Tab | KeyCode::BackTab => {
                let names: Vec<String> = self.repos.iter().map(|r| r.name.clone()).collect();
                edit.complete(line, &names, key.code == KeyCode::BackTab);
            }
            KeyCode::Enter => {
                edit.remember(line);
                let parsed = crate::command::parse(&self.command_line);
                // A line that is not a command says why, not just "unknown".
                let rejected = parsed
                    .is_none()
                    .then(|| crate::command::explain(&self.command_line));
                self.command_line.clear();
                self.mode = Mode::Normal;
                let cmd = self.apply_command(parsed);
                if let Some(why) = rejected {
                    self.status = Some(why);
                }
                return cmd;
            }
            KeyCode::Backspace => edit.backspace(line),
            KeyCode::Char(c) => edit.insert(line, c),
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
            Some(C::Standup) => return self.open_standup(),
            Some(C::Launchpad) => return self.open_launchpad(),
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
            Some(i) => {
                self.selected = i;
                // Put the focus where the cursor just moved, so it is visible.
                self.focus = Card::Repos;
                self.pane = Pane::List;
            }
            // A bare word is tried as a repository name, so a mistyped
            // command lands here too: say both.
            None => self.status = Some(format!("no command or repository matches '{name}'")),
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

    /// Ask before handing the repository under the cursor to the user's
    /// coding agent to write (or update) its launchpad file. One repository at
    /// a time: an agent session is interactive, so it cannot be a bulk action.
    fn request_agent_setup(&mut self) {
        let Some(repo) = self.current_repo() else {
            self.status = Some("no repo selected".to_string());
            return;
        };
        if repo.has_error() || repo.path.is_none() {
            self.status = Some(format!(
                "{} cannot be read, so it cannot be set up",
                repo.name
            ));
            return;
        }
        self.confirm = Some(Pending {
            prompt: format!(
                "Start your coding agent in {} to set it up for the launchpad?",
                repo.name
            ),
            action: ConfirmAction::AgentSetup(repo.id.clone()),
        });
        self.mode = Mode::Confirm;
    }

    /// Open the confirmation modal for stashing the target repos' changes.
    fn request_bulk_stash(&mut self) {
        let ids = self.action_targets();
        self.confirm_stash(ids);
    }

    /// Ask before stashing changes in `ids` (unreadable repositories are left
    /// out: there is nothing to stash in a repository that cannot be read).
    fn confirm_stash(&mut self, ids: Vec<RepoId>) {
        let ids: Vec<RepoId> = ids
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
        let step = if toward_top { -1 } else { 1 };
        match self.mode {
            // The wheel moves whatever the arrow keys would move.
            Mode::Normal => match self.screen {
                Screen::Overview => self.move_cursor(step),
                Screen::Repos => self.move_in_detail(step),
            },
            Mode::Filter => {
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
                    conflicted: 0,
                }
            } else {
                WorktreeStatus::default()
            },
            stash_count: 0,
            stash_latest: None,
            remote_url: None,
            operation: None,
            default_branch: None,
            last_fetch: None,
            latest_tag: None,
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
            branches: Vec::new(),
        }
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn code(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// An app with these repositories and the focus on the repository table
    /// (card 5), which is where the table keys live.
    fn app_with(names: &[(&str, bool)]) -> App {
        let mut app = App::new(vec![], String::new());
        app.set_repos(names.iter().map(|(n, d)| snap(n, *d)).collect());
        app.focus = Card::Repos;
        app
    }

    const NOW: i64 = 1_700_000_000;

    /// The built-in sample fleet with its overview, as `cohors demo` starts.
    fn demo_app() -> App {
        let mut app = App::new(vec![], String::new());
        crate::tui::seed_demo(&mut app, NOW);
        app.now = NOW;
        app
    }

    /// The index of the first "Do next" step of `kind` in the sample fleet.
    fn step(app: &App, kind: ActionKind) -> usize {
        app.actions()
            .iter()
            .position(|action| action.kind == kind)
            .unwrap_or_else(|| panic!("the sample fleet has no {kind:?} step"))
    }

    #[test]
    fn number_keys_focus_cards_and_tab_cycles_both_ways() {
        let mut app = demo_app();
        assert_eq!(app.focus, Card::DoNext); // what cohors opens on
        for (digit, card) in ['1', '2', '3', '4', '5'].into_iter().zip(Card::ALL) {
            app.on_key(key(digit));
            assert_eq!(app.focus, card);
        }
        // Tab wraps from the last card to the first, shift-tab the other way.
        app.on_key(code(KeyCode::Tab));
        assert_eq!(app.focus, Card::DoNext);
        app.on_key(code(KeyCode::BackTab));
        assert_eq!(app.focus, Card::Repos);
        // Digits that are not a card do nothing.
        app.on_key(key('6'));
        app.on_key(key('0'));
        assert_eq!(app.focus, Card::Repos);
    }

    #[test]
    fn arrows_and_jk_move_only_the_focused_card() {
        let mut app = demo_app();
        app.on_key(key('2'));
        app.on_key(key('j'));
        app.on_key(code(KeyCode::Down));
        assert_eq!(app.cursors.risk, 2);
        app.on_key(key('k'));
        assert_eq!(app.cursors.risk, 1);
        // Nothing else moved.
        assert_eq!((app.cursors.next, app.selected), (0, 0));

        // The cursor stops at both ends instead of wrapping or overflowing.
        app.on_key(code(KeyCode::End));
        assert_eq!(app.cursors.risk, app.at_risk().len() - 1);
        app.on_key(code(KeyCode::Down));
        assert_eq!(app.cursors.risk, app.at_risk().len() - 1);
        app.on_key(code(KeyCode::Home));
        app.on_key(code(KeyCode::Up));
        assert_eq!(app.cursors.risk, 0);
    }

    #[test]
    fn moving_in_an_empty_card_is_harmless() {
        // No overview yet and no repositories: every card is empty.
        let mut app = App::new(vec![], String::new());
        for card in ['1', '2', '3', '4', '5'] {
            app.on_key(key(card));
            for movement in [KeyCode::Down, KeyCode::Up, KeyCode::End, KeyCode::PageDown] {
                assert_eq!(app.on_key(code(movement)), Cmd::None);
            }
        }
        // Enter has nothing to open in "Do next", "Work at risk" or the table.
        for card in ['1', '2', '5'] {
            app.on_key(key(card));
            assert_eq!(app.on_key(code(KeyCode::Enter)), Cmd::None);
        }
        assert_eq!(app.cursors, Cursors::default());
        assert_eq!((app.screen, app.selected), (Screen::Overview, 0));
    }

    #[test]
    fn a_step_with_a_verb_runs_over_exactly_its_repositories() {
        let mut app = demo_app();
        let index = step(&app, ActionKind::Pull);
        let targets: Vec<RepoId> = app.actions()[index]
            .repos
            .iter()
            .map(|id| RepoId(id.clone()))
            .collect();
        assert!(!targets.is_empty());

        // Its own key, from "Do next"…
        assert_eq!(
            app.on_key(key('p')),
            Cmd::Act {
                verb: BulkVerb::Pull,
                ids: targets.clone()
            }
        );
        // …and Enter on the highlighted step.
        app.cursors.next = index;
        assert_eq!(
            app.on_key(code(KeyCode::Enter)),
            Cmd::Act {
                verb: BulkVerb::Pull,
                ids: targets
            }
        );
    }

    #[test]
    fn push_and_publish_share_a_key_and_run_together() {
        let step = |kind, repos: &[&str]| NextAction {
            kind,
            severity: cohors_core::Severity::Warn,
            title: String::new(),
            why: String::new(),
            label: String::new(),
            verb: Some("push".to_string()),
            repos: repos.iter().map(|r| r.to_string()).collect(),
        };
        let mut app = demo_app();
        let overview = app.overview.as_mut().unwrap();
        overview.actions = vec![
            step(ActionKind::Push, &["a", "b"]),
            // "b" is listed by both steps and must be pushed once.
            step(ActionKind::Publish, &["b", "c"]),
        ];
        let ids = |names: &[&str]| names.iter().map(|n| RepoId(n.to_string())).collect();
        assert_eq!(
            app.on_key(key('P')),
            Cmd::Act {
                verb: BulkVerb::Push,
                ids: ids(&["a", "b", "c"])
            }
        );
        // Enter acts on the highlighted step alone.
        app.cursors.next = 1;
        assert_eq!(
            app.on_key(code(KeyCode::Enter)),
            Cmd::Act {
                verb: BulkVerb::Push,
                ids: ids(&["b", "c"])
            }
        );
    }

    #[test]
    fn a_step_that_needs_a_look_narrows_the_table_instead_of_acting() {
        let mut app = demo_app();
        let index = step(&app, ActionKind::Commit);
        let wanted = app.actions()[index].repos.len();
        // `c` means "commit", and the rows here are steps: it does nothing
        // but say where it works.
        assert_eq!(app.on_key(key('c')), Cmd::None);
        assert_eq!(app.focus, Card::DoNext);
        assert!(app.status.as_deref().unwrap().contains("commit"));
        // Enter on the step shows its repositories; it commits nothing.
        app.cursors.next = index;
        assert_eq!(app.on_key(code(KeyCode::Enter)), Cmd::None);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.focus, Card::Repos);
        assert_eq!(app.visible_len(), wanted);
        assert!(app.scope.is_some());

        // Esc on the table shows the whole fleet again.
        app.on_key(code(KeyCode::Esc));
        assert!(app.scope.is_none());
        assert_eq!(app.visible_len(), app.repos.len());
    }

    #[test]
    fn a_stash_step_asks_before_it_does_anything() {
        let mut app = demo_app();
        let target = app
            .repos
            .iter()
            .find(|r| !r.has_error())
            .unwrap()
            .id
            .clone();
        app.overview.as_mut().unwrap().actions = vec![NextAction {
            kind: ActionKind::ReviewStash,
            severity: cohors_core::Severity::Warn,
            title: String::new(),
            why: String::new(),
            label: String::new(),
            verb: Some("stash".to_string()),
            repos: vec![target.0.clone()],
        }];
        assert_eq!(app.on_key(code(KeyCode::Enter)), Cmd::None);
        assert_eq!(app.mode, Mode::Confirm);
        assert!(matches!(
            app.confirm.as_ref().unwrap().action,
            ConfirmAction::BulkStash(ref ids) if ids == &[target]
        ));
    }

    #[test]
    fn an_unknown_verb_never_runs_blind() {
        let mut app = demo_app();
        app.overview.as_mut().unwrap().actions = vec![NextAction {
            kind: ActionKind::Fetch,
            severity: cohors_core::Severity::Info,
            title: "Something new".to_string(),
            why: String::new(),
            label: String::new(),
            verb: Some("obliterate".to_string()),
            repos: vec!["demo/payments".to_string()],
        }];
        // A verb this build does not know is shown, not executed.
        assert_eq!(app.on_key(key('f')), Cmd::None);
        assert_eq!(app.scope.as_ref().unwrap().label, "Something new");
    }

    #[test]
    fn step_keys_do_nothing_when_no_step_lists_them() {
        let mut app = demo_app();
        app.overview.as_mut().unwrap().actions.clear();
        for c in ['P', 'p', 'f', 'S', 'c', 'e', 'm', 'd', 'x'] {
            assert_eq!(app.on_key(key(c)), Cmd::None, "{c}");
        }
        assert!(app.scope.is_none() && app.confirm.is_none());
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn the_start_task_step_opens_the_launchpad() {
        let mut app = demo_app();
        assert_eq!(app.on_key(key('t')), Cmd::OpenLaunchpad);
        assert_eq!(app.mode, Mode::Launchpad);
    }

    #[test]
    fn enter_on_work_at_risk_opens_that_repository_in_the_detail_view() {
        let mut app = demo_app();
        app.on_key(key('2'));
        app.on_key(code(KeyCode::Down));
        let wanted = app.at_risk()[1].id.clone();
        // A filter that hides the repository must not stop it being shown.
        app.filter = "zzz-matches-nothing".to_string();
        app.on_key(code(KeyCode::Enter));
        assert_eq!(app.screen, Screen::Repos);
        assert_eq!(app.selected_repo().unwrap().id.0, wanted);
        assert!(app.filter.is_empty());
    }

    #[test]
    fn tasks_card_opens_the_board_or_one_repositorys_tasks() {
        let mut app = demo_app();
        app.on_key(key('3'));
        assert_eq!(app.on_key(key('t')), Cmd::OpenLaunchpad);
        assert_eq!(app.launchpad_jump, None);

        app.mode = Mode::Normal;
        app.on_key(code(KeyCode::Down));
        let name = app.overview.as_ref().unwrap().work[1].name.clone();
        assert_eq!(app.on_key(code(KeyCode::Enter)), Cmd::OpenLaunchpad);
        assert_eq!(app.launchpad_jump, Some(name));
    }

    #[test]
    fn commits_card_opens_the_standup_or_copies_it() {
        let mut app = demo_app();
        app.on_key(key('4'));
        assert_eq!(app.on_key(key('s')), Cmd::OpenStandup);
        assert_eq!(app.mode, Mode::Standup);

        // `Y` copies the standup from any card; `y` stays "copy the path",
        // which means nothing on this card.
        app.mode = Mode::Normal;
        assert_eq!(app.on_key(key('y')), Cmd::None);
        for card in ['1', '2', '3', '4', '5'] {
            app.on_key(key(card));
            app.copy_standup_pending = false;
            assert_eq!(app.on_key(key('Y')), Cmd::CopyStandupMarkdown, "{card}");
            assert!(app.copy_standup_pending);
            assert_eq!(app.mode, Mode::Normal); // copying does not open the view
        }
        app.on_key(key('4'));

        // Enter opens the standup on the highlighted repository.
        let name = app.overview.as_ref().unwrap().commits[0].name.clone();
        assert_eq!(app.on_key(code(KeyCode::Enter)), Cmd::OpenStandup);
        assert_eq!(app.standup_jump, Some(name.clone()));
        let commit = |repo: &str| StandupCommit {
            repo: repo.to_string(),
            short_id: "abc1234".to_string(),
            summary: "work".to_string(),
            timestamp: NOW,
        };
        app.show_standup(vec![commit("zzz"), commit("zzz"), commit(&name)]);
        let view = app.standup.as_ref().unwrap();
        assert_eq!(view.groups[view.focus].0, name);
        assert_eq!(app.standup_jump, None);
    }

    #[test]
    fn enter_on_the_table_opens_the_detail_view_and_esc_returns() {
        let mut app = demo_app();
        app.on_key(key('5'));
        app.on_key(code(KeyCode::Down));
        let selected = app.selected_repo().unwrap().id.clone();
        assert_eq!(app.on_key(code(KeyCode::Enter)), Cmd::None);
        assert_eq!(app.screen, Screen::Repos);
        assert_eq!(app.selected_repo().unwrap().id, selected);

        // Enter (or `d`) there asks for the full drill-in.
        assert_eq!(app.on_key(code(KeyCode::Enter)), Cmd::OpenDetail);
        assert_eq!(app.on_key(key('d')), Cmd::OpenDetail);

        // Tab walks the four cards; arrows only move the list while it has
        // the focus.
        app.on_key(code(KeyCode::Tab));
        assert_eq!(app.pane, Pane::State);
        app.on_key(code(KeyCode::Down));
        assert_eq!(app.selected_repo().unwrap().id, selected);
        app.on_key(code(KeyCode::BackTab));
        app.on_key(code(KeyCode::Down));
        assert_ne!(app.selected_repo().unwrap().id, selected);

        app.on_key(code(KeyCode::Esc));
        assert_eq!(app.screen, Screen::Overview);
        assert_eq!(app.focus, Card::Repos);
        // A number key also leaves the detail view, focusing that card.
        app.on_key(code(KeyCode::Enter));
        app.on_key(key('2'));
        assert_eq!((app.screen, app.focus), (Screen::Overview, Card::AtRisk));
    }

    #[test]
    fn the_table_keys_work_in_the_detail_view_too() {
        let mut app = demo_app();
        app.screen = Screen::Repos;
        assert_eq!(app.on_key(key('f')), Cmd::FetchSelected);
        assert_eq!(app.on_key(key('p')), Cmd::PullSelected);
        assert_eq!(app.on_key(key('P')), Cmd::PushSelected);
        assert_eq!(app.on_key(key('o')), Cmd::OpenWith);
        app.on_key(key('>'));
        assert_eq!(app.sort, SortMode::Recent);
    }

    #[test]
    fn table_action_keys_are_inert_on_the_other_cards() {
        // On "Open tasks" and "Commits" none of the repository keys act.
        let mut app = demo_app();
        for card in ['3', '4'] {
            app.on_key(key(card));
            for c in ['f', 'p', 'P', 'S', 'c', ' ', '*', '>', 'o', 'L'] {
                assert_eq!(app.on_key(key(c)), Cmd::None, "card {card}, key {c}");
            }
        }
        assert!(app.selection.is_empty() && app.confirm.is_none());
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn repository_keys_act_on_the_row_under_the_cursor_on_work_at_risk() {
        let mut app = demo_app();
        // Mark something in the table: marks must not leak into this card.
        app.on_key(key('5'));
        app.on_key(key(' '));
        app.on_key(key('2'));
        // The third row: a readable repository (the second cannot be read).
        app.cursors.risk = 2;
        let row = RepoId(app.at_risk()[2].id.clone());
        assert_ne!(app.selected_repo().map(|r| r.id.clone()), Some(row.clone()));

        // The same keys as the table, about the highlighted repository.
        assert_eq!(app.current_repo().unwrap().id, row);
        assert_eq!(app.action_targets(), vec![row.clone()]);
        assert_eq!(app.on_key(key('f')), Cmd::FetchSelected);
        assert_eq!(app.on_key(key('p')), Cmd::PullSelected);
        assert_eq!(app.on_key(key('P')), Cmd::PushSelected);
        assert_eq!(app.on_key(key('o')), Cmd::OpenWith);
        // `S` is "stash" here as everywhere: it asks about that repository.
        assert_eq!(app.on_key(key('S')), Cmd::None);
        assert!(matches!(
            app.confirm.as_ref().unwrap().action,
            ConfirmAction::BulkStash(ref ids) if ids == &[row]
        ));
        // Back on the table the marks count again.
        app.confirm = None;
        app.mode = Mode::Normal;
        app.on_key(key('5'));
        assert_eq!(app.action_targets().len(), 1);
        assert_eq!(
            app.action_targets(),
            app.selection.iter().cloned().collect::<Vec<_>>()
        );
    }

    #[test]
    fn fetch_all_works_from_every_card() {
        let mut app = demo_app();
        for card in ['1', '2', '3', '4', '5'] {
            app.on_key(key(card));
            assert_eq!(app.on_key(key('F')), Cmd::FetchAll, "{card}");
        }
    }

    #[test]
    fn the_command_line_recalls_completes_and_edits() {
        let mut app = app_with(&[("payments", false), ("web-app", false)]);
        let typed = |app: &mut App, text: &str| {
            for c in text.chars() {
                app.on_key(key(c));
            }
        };
        // Tab finishes a command, then its argument.
        app.on_key(key(':'));
        typed(&mut app, "so");
        app.on_key(code(KeyCode::Tab));
        assert_eq!(app.command_line, "sort ");
        typed(&mut app, "n");
        app.on_key(code(KeyCode::Tab));
        assert_eq!(app.command_line, "sort name");
        app.on_key(code(KeyCode::Enter));
        assert_eq!(app.sort, SortMode::Name);

        // A repository name completes too, and Enter jumps to it.
        app.on_key(key(':'));
        typed(&mut app, "we");
        app.on_key(code(KeyCode::Tab));
        assert_eq!(app.command_line, "web-app");
        app.on_key(code(KeyCode::Enter));
        assert_eq!(app.selected_repo().unwrap().name, "web-app");

        // Up brings back what was run, newest first; it can be edited.
        app.on_key(key(':'));
        app.on_key(code(KeyCode::Up));
        assert_eq!(app.command_line, "web-app");
        app.on_key(code(KeyCode::Up));
        assert_eq!(app.command_line, "sort name");
        for _ in 0..4 {
            app.on_key(code(KeyCode::Backspace));
        }
        typed(&mut app, "recent");
        // The cursor moves: fix a word from the middle of the line.
        app.on_key(code(KeyCode::Home));
        app.on_key(code(KeyCode::Delete));
        typed(&mut app, "s");
        assert_eq!(app.command_line, "sort recent");
        app.on_key(code(KeyCode::Enter));
        assert_eq!(app.sort, SortMode::Recent);

        // Esc cancels, and a cancelled line is not remembered.
        app.on_key(key(':'));
        typed(&mut app, "quit");
        app.on_key(code(KeyCode::Esc));
        app.on_key(key(':'));
        app.on_key(code(KeyCode::Up));
        assert_eq!(app.command_line, "sort recent");
    }

    #[test]
    fn commit_key_asks_for_a_message_then_for_confirmation() {
        let mut app = app_with(&[("a", true)]);
        assert_eq!(app.on_key(key('c')), Cmd::None);
        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.command_line, "commit ");
        for c in "wip".chars() {
            app.on_key(key(c));
        }
        // Enter does not commit: it opens the confirmation.
        assert_eq!(app.on_key(code(KeyCode::Enter)), Cmd::None);
        assert_eq!(app.mode, Mode::Confirm);
        // An empty message is not a commit at all.
        let mut app = app_with(&[("a", true)]);
        app.on_key(key('c'));
        assert_eq!(app.on_key(code(KeyCode::Enter)), Cmd::None);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn slash_filters_the_table_from_any_card() {
        let mut app = demo_app();
        app.on_key(key('3'));
        app.on_key(key('/'));
        assert_eq!((app.mode, app.focus), (Mode::Filter, Card::Repos));
    }

    #[test]
    fn esc_on_the_table_peels_back_one_narrowing_at_a_time() {
        let mut app = demo_app();
        // Narrow to the "commit or discard" step, then mark the first of them.
        app.cursors.next = step(&app, ActionKind::Commit);
        app.on_key(code(KeyCode::Enter));
        app.on_key(key(' '));
        assert!(!app.selection.is_empty() && app.scope.is_some());
        app.on_key(code(KeyCode::Esc));
        assert!(app.selection.is_empty() && app.scope.is_some());
        app.on_key(code(KeyCode::Esc));
        assert!(app.scope.is_none());
        // With nothing else narrowing, Esc clears a kept filter.
        app.filter = "pay".to_string();
        app.on_key(code(KeyCode::Esc));
        assert!(app.filter.is_empty());
    }

    #[test]
    fn the_table_follows_the_overviews_order_until_a_sort_is_chosen() {
        let mut app = demo_app();
        let listed = |app: &App| -> Vec<String> {
            app.view()
                .iter()
                .map(|row| app.repos[row.index].id.0.clone())
                .collect()
        };
        let overview: Vec<String> = app
            .overview
            .as_ref()
            .unwrap()
            .repos
            .iter()
            .map(|line| line.id.clone())
            .collect();
        assert_eq!(listed(&app), overview);

        // Choosing a sort takes over from the overview's order.
        app.on_key(key('5'));
        app.on_key(key('>'));
        app.on_key(key('>'));
        assert_eq!(app.sort, SortMode::Name);
        let mut by_name = listed(&app);
        by_name.sort_by_key(|id| id.to_lowercase());
        assert_eq!(listed(&app), by_name);
    }

    #[test]
    fn a_new_overview_keeps_the_cursor_on_the_same_repository() {
        let mut app = demo_app();
        app.on_key(key('5'));
        app.on_key(code(KeyCode::End));
        let selected = app.selected_repo().unwrap().id.clone();
        // An overview whose order is reversed must not move the cursor off it.
        let mut overview = app.overview.clone().unwrap();
        overview.repos.reverse();
        let entries = app.entries.clone();
        app.set_overview(overview, entries);
        assert_eq!(app.selected_repo().unwrap().id, selected);
    }

    #[test]
    fn a_shorter_overview_pulls_the_card_cursors_back_in_range() {
        let mut app = demo_app();
        app.cursors = Cursors {
            next: 99,
            risk: 99,
            tasks: 99,
            commits: 99,
        };
        let overview = cohors_core::overview(&[], &[], &Default::default(), NOW);
        app.set_overview(overview, Vec::new());
        assert_eq!(app.cursors, Cursors::default());
    }

    #[test]
    fn the_key_a_asks_before_starting_an_agent_on_the_selected_repo() {
        let mut app = demo_app();
        // On the cards whose rows are not repositories, `a` starts nothing.
        for card in ['1', '3', '4'] {
            app.on_key(key(card));
            assert_eq!(app.on_key(key('a')), Cmd::None);
            assert_eq!(app.mode, Mode::Normal, "card {card}");
        }
        // On the repositories card it asks, naming the repository, and starts
        // nothing by itself.
        app.on_key(key('5'));
        let name = app.selected_repo().unwrap().name.clone();
        assert_eq!(app.on_key(key('a')), Cmd::None);
        assert_eq!(app.mode, Mode::Confirm);
        let pending = app.confirm.clone().unwrap();
        assert!(pending.prompt.contains(&name), "{}", pending.prompt);
        assert!(matches!(pending.action, ConfirmAction::AgentSetup(_)));
        // Anything but y cancels and forgets it.
        assert_eq!(app.on_key(key('n')), Cmd::None);
        assert!(app.confirm.is_none() && app.mode == Mode::Normal);
        // y hands the pending action to the event loop.
        app.on_key(key('a'));
        assert_eq!(app.on_key(key('y')), Cmd::ConfirmAccept);
        assert!(matches!(
            app.confirm.as_ref().unwrap().action,
            ConfirmAction::AgentSetup(_)
        ));
        // The detail view offers the same key.
        app.confirm = None;
        app.screen = Screen::Repos;
        app.on_key(key('a'));
        assert_eq!(app.mode, Mode::Confirm);
        assert!(app.selection.is_empty(), "it never marks repositories");
    }

    #[test]
    fn a_step_has_a_letter_only_when_it_is_that_verb() {
        use ActionKind::*;
        // The steps that act carry the letter the repository keys use for
        // the same verb, so a letter is one verb on every card.
        assert_eq!(action_key(Fetch), Some('f'));
        assert_eq!(action_key(Pull), Some('p'));
        assert_eq!(action_key(Push), Some('P'));
        assert_eq!(action_key(Publish), Some('P'));
        assert_eq!(action_key(StartTask), Some('t'));
        for (kind, key) in [(Fetch, 'f'), (Pull, 'p'), (Push, 'P')] {
            let verb = repo_key_verb(key).unwrap();
            assert!(
                format!("{kind:?}").to_lowercase().starts_with(verb),
                "{kind:?}"
            );
        }
        // The steps that only show you repositories have none: `S` and `c`
        // stay "stash" and "commit", and nothing else is borrowed either.
        for kind in [Unreadable, Resolve, Reconcile, ReviewStash, Commit] {
            assert_eq!(action_key(kind), None, "{kind:?}");
        }
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
        app.on_key(key('>'));
        assert_eq!(app.sort, SortMode::Recent);
    }

    #[test]
    fn dirty_only_toggle_filters_view() {
        let mut app = app_with(&[("clean", false), ("dirty", true)]);
        assert_eq!(app.visible_len(), 2);
        // The attention toggle has no bare key any more; it is `:dirty`.
        app.on_key(key(':'));
        for c in "dirty".chars() {
            app.on_key(key(c));
        }
        app.on_key(code(KeyCode::Enter));
        assert!(app.dirty_only);
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
        app.on_key(key('*'));
        assert_eq!(app.selection.len(), 3);
        app.on_key(key('*')); // already all marked → clears
        assert!(app.selection.is_empty());
    }

    #[test]
    fn esc_clears_selection() {
        let mut app = app_with(&[("a", false)]);
        app.on_key(key('*'));
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
        app.dirty_only = true;
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

    /// Live filtering: typing narrows immediately, and backspacing past the
    /// empty query removes the leading `/` and closes the filter.
    #[test]
    fn filter_backspace_past_empty_closes() {
        let mut app = app_with(&[("payments", false), ("auth", false)]);
        app.on_key(key('/'));
        assert_eq!(app.mode, Mode::Filter);

        // Typing narrows live — no Enter needed.
        app.on_key(key('p'));
        assert_eq!(app.filter, "p");
        assert_eq!(app.visible_len(), 1);

        // Backspace deletes the char but stays in filter mode (query now empty).
        app.on_key(code(KeyCode::Backspace));
        assert!(app.filter.is_empty());
        assert_eq!(app.mode, Mode::Filter);
        assert_eq!(app.visible_len(), 2);

        // Backspacing again (past empty) closes the filter.
        app.on_key(code(KeyCode::Backspace));
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.filter.is_empty());
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
    fn push_branch_picker_navigates_accepts_and_cancels() {
        let branch = |name: &str, is_current: bool| cohors_core::BranchInfo {
            name: name.into(),
            ahead: 0,
            behind: 0,
            has_upstream: true,
            is_current,
            last_commit: None,
        };
        let mut app = app_with(&[("a", false)]);
        app.mode = Mode::PushBranch;
        app.push_pick = Some(PushPick {
            id: RepoId("a".into()),
            name: "a".into(),
            branches: vec![branch("main", true), branch("feat", false)],
            cursor: 0,
        });
        // Down moves the highlight; Enter asks the loop to push the chosen branch.
        assert_eq!(app.on_key(code(KeyCode::Down)), Cmd::None);
        assert_eq!(app.push_pick.as_ref().unwrap().cursor, 1);
        assert_eq!(app.on_key(code(KeyCode::Enter)), Cmd::PushBranchAccept);
        // Esc closes the picker.
        app.on_key(code(KeyCode::Esc));
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.push_pick.is_none());
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
        // Enter on the table opens the detail view, where Enter again asks for
        // the full drill-in.
        assert_eq!(app.on_key(code(KeyCode::Enter)), Cmd::None);
        assert_eq!(app.screen, Screen::Repos);
        assert_eq!(app.on_key(code(KeyCode::Enter)), Cmd::OpenDetail);
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
