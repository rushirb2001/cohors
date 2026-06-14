//! Dashboard state and key handling.
//!
//! The state is deliberately small and rendering-free: the ordered, filtered
//! view is *derived* each frame via [`cohors_core::compute_view`], so the
//! "what to show, in what order" logic stays in the shared core. Key handling
//! returns a [`Cmd`] telling the event loop what side effect to run (quit,
//! rescan), keeping I/O out of the state.

use std::collections::HashSet;

use cohors_core::{
    RepoId, RepoSnapshot, SortMode, StandupWindow, ViewParams, ViewRow, compute_view,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Which input mode the dashboard is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Typing into the fuzzy filter.
    Filter,
    /// The help overlay is open.
    Help,
    /// The weekly-standup view is open.
    Standup,
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
    /// Open the weekly-standup view (collect commits).
    OpenStandup,
    /// Cycle the standup window (today ↔ this week) and re-collect.
    StandupNextWindow,
    /// Copy the standup markdown to the clipboard.
    CopyStandup,
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
    /// The rendered standup markdown (`None` while collecting, or not opened).
    pub standup: Option<String>,
    /// The standup time window.
    pub standup_window: StandupWindow,
    /// Vertical scroll offset (in lines) within the standup overlay.
    pub standup_scroll: u16,
    /// Max scroll offset, cached from the last render so key handling can clamp
    /// without knowing the viewport. Interior-mutable: the view writes it each
    /// frame, the controller reads it.
    standup_max_scroll: std::cell::Cell<u16>,
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
            standup_scroll: 0,
            standup_max_scroll: std::cell::Cell::new(0),
        }
    }

    /// Cache the standup's maximum scroll offset (the view calls this each frame
    /// so the controller can clamp scrolling to the available content).
    pub fn set_standup_max_scroll(&self, max: u16) {
        self.standup_max_scroll.set(max);
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
    // Consumed by the command runner and bulk actions in the following chunks.
    #[allow(dead_code)]
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
            Mode::Standup => self.on_key_standup(key),
        }
    }

    fn on_key_normal(&mut self, key: KeyEvent) -> Cmd {
        match key.code {
            KeyCode::Char('q') => return Cmd::Quit,
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
            KeyCode::Tab => {
                self.mode = Mode::Standup;
                self.standup = None;
                self.standup_scroll = 0;
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

    fn on_key_help(&mut self, key: KeyEvent) {
        if matches!(
            key.code,
            KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q')
        ) {
            self.mode = Mode::Normal;
        }
    }

    fn on_key_standup(&mut self, key: KeyEvent) -> Cmd {
        let max = self.standup_max_scroll.get();
        match key.code {
            KeyCode::Tab | KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Normal;
                Cmd::None
            }
            KeyCode::Char('w') => {
                self.standup_window = self.standup_window.next();
                self.standup = None;
                self.standup_scroll = 0;
                Cmd::StandupNextWindow
            }
            KeyCode::Char('y') => Cmd::CopyStandup,
            KeyCode::Down | KeyCode::Char('j') => {
                self.standup_scroll = self.standup_scroll.saturating_add(1).min(max);
                Cmd::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.standup_scroll = self.standup_scroll.saturating_sub(1);
                Cmd::None
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.standup_scroll = self.standup_scroll.saturating_add(10).min(max);
                Cmd::None
            }
            KeyCode::PageUp => {
                self.standup_scroll = self.standup_scroll.saturating_sub(10);
                Cmd::None
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.standup_scroll = 0;
                Cmd::None
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.standup_scroll = max;
                Cmd::None
            }
            _ => Cmd::None,
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
