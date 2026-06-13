//! Dashboard state and key handling.
//!
//! The state is deliberately small and rendering-free: the ordered, filtered
//! view is *derived* each frame via [`cohors_core::compute_view`], so the
//! "what to show, in what order" logic stays in the shared core. Key handling
//! returns a [`Cmd`] telling the event loop what side effect to run (quit,
//! rescan), keeping I/O out of the state.

use std::collections::HashSet;

use cohors_core::{RepoId, RepoSnapshot, SortMode, ViewParams, ViewRow, compute_view};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Which input mode the dashboard is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Typing into the fuzzy filter.
    Filter,
    /// The help overlay is open.
    Help,
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
    /// Open the selected repo in the editor.
    OpenEditor,
    /// Reveal the selected repo in the file manager.
    RevealFileManager,
    /// Open the selected repo in lazygit.
    Lazygit,
    /// Copy the selected repo's path to the clipboard.
    CopyPath,
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
    /// Configured roots, for the empty/loading states.
    pub roots: Vec<String>,
    /// Config file path, for the help overlay.
    pub config_path: String,
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
            roots,
            config_path,
        }
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

    /// The repo under the selection, if any. (Used by the action keys —
    /// fetch/pull/open — landing in the next milestone; exercised by tests now.)
    #[allow(dead_code)]
    pub fn selected_repo(&self) -> Option<&RepoSnapshot> {
        self.view()
            .get(self.selected)
            .map(|row| &self.repos[row.index])
    }

    /// Replace the repo set (e.g. after a scan) and keep the selection in range.
    pub fn set_repos(&mut self, repos: Vec<RepoSnapshot>) {
        self.repos = repos;
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
            Mode::Help => {
                self.on_key_help(key);
                Cmd::None
            }
        }
    }

    fn on_key_normal(&mut self, key: KeyEvent) -> Cmd {
        match key.code {
            KeyCode::Char('q') => return Cmd::Quit,
            KeyCode::Char('j') | KeyCode::Down => self.move_down(),
            KeyCode::Char('k') | KeyCode::Up => self.move_up(),
            KeyCode::Char('g') => self.selected = 0,
            KeyCode::Char('G') => self.select_last(),
            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Char('d') => {
                self.dirty_only = !self.dirty_only;
                self.clamp_selection();
            }
            KeyCode::Char('s') => {
                self.sort = self.sort.next();
                self.clamp_selection();
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('r') => return Cmd::Refresh,
            // Actions (operate on the selection / all repos).
            KeyCode::Char('f') => return Cmd::FetchSelected,
            KeyCode::Char('F') => return Cmd::FetchAll,
            KeyCode::Char('p') => return Cmd::PullSelected,
            KeyCode::Enter => return Cmd::OpenEditor,
            KeyCode::Char('o') => return Cmd::RevealFileManager,
            KeyCode::Char('L') => return Cmd::Lazygit,
            KeyCode::Char('y') => return Cmd::CopyPath,
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

    fn on_key_help(&mut self, key: KeyEvent) {
        if matches!(
            key.code,
            KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q')
        ) {
            self.mode = Mode::Normal;
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
            last_commit: Some(CommitMeta {
                short_id: "abc1234".to_string(),
                author: "Dev".to_string(),
                timestamp: 1_700_000_000,
                summary: "msg".to_string(),
            }),
            error: None,
        }
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn app_with(names: &[(&str, bool)]) -> App {
        let mut app = App::new(vec![], String::new());
        app.set_repos(names.iter().map(|(n, d)| snap(n, *d)).collect());
        app
    }

    #[test]
    fn navigation_moves_and_clamps() {
        let mut app = app_with(&[("a", false), ("b", false), ("c", false)]);
        assert_eq!(app.selected, 0);
        app.on_key(key('j'));
        assert_eq!(app.selected, 1);
        app.on_key(key('G'));
        assert_eq!(app.selected, 2);
        app.on_key(key('j')); // clamp at bottom
        assert_eq!(app.selected, 2);
        app.on_key(key('g'));
        assert_eq!(app.selected, 0);
        app.on_key(key('k')); // clamp at top
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
        assert_eq!(app.on_key(key('o')), Cmd::RevealFileManager);
        assert_eq!(app.on_key(key('L')), Cmd::Lazygit);
        assert_eq!(app.on_key(key('y')), Cmd::CopyPath);
        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Cmd::OpenEditor
        );
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
