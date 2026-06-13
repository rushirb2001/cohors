//! Rendering. Pure view-model → widgets: every frame derives the visible rows
//! from [`App::view`] (i.e. `cohors-core`) and maps them onto ratatui widgets.
//! No state is mutated here.

use cohors_core::{Branch, RepoSnapshot, time};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};

use crate::app::{App, Mode};

/// Spinner frames (braille) for the scan indicator.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn spinner_frame(tick: usize) -> &'static str {
    SPINNER[tick % SPINNER.len()]
}

/// Color policy. Colors are dropped when `NO_COLOR` is set; structural
/// modifiers (dim/bold/reversed) are kept so the layout still reads.
struct Theme {
    color: bool,
}

impl Theme {
    fn from_env() -> Self {
        Self {
            color: std::env::var_os("NO_COLOR").is_none(),
        }
    }

    fn fg(&self, c: Color) -> Style {
        if self.color {
            Style::new().fg(c)
        } else {
            Style::new()
        }
    }

    fn dim(&self) -> Style {
        Style::new().add_modifier(Modifier::DIM)
    }
    fn staged(&self) -> Style {
        self.fg(Color::Green)
    }
    fn modified(&self) -> Style {
        self.fg(Color::Yellow)
    }
    fn untracked(&self) -> Style {
        self.dim()
    }
    fn ahead(&self) -> Style {
        self.fg(Color::Cyan)
    }
    fn behind(&self) -> Style {
        self.fg(Color::Yellow)
    }
    fn detached(&self) -> Style {
        self.fg(Color::Magenta)
    }
    fn error(&self) -> Style {
        self.fg(Color::Red).add_modifier(Modifier::BOLD)
    }
    fn highlight(&self) -> Style {
        self.fg(Color::Yellow).add_modifier(Modifier::BOLD)
    }
}

/// Render the whole dashboard for one frame. `now` (Unix seconds) is injected
/// so relative commit ages are deterministic in tests.
pub fn render(frame: &mut Frame, app: &App, now: i64) {
    let theme = Theme::from_env();
    let area = frame.area();

    let block = Block::bordered()
        .title_top(Line::from(" cohors ").bold())
        .title_top(Line::from(header_status(app)).right_aligned())
        .title_bottom(Line::from(footer_hints(app)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.mode == Mode::Filter {
        let [filter_area, body_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
        render_filter_input(frame, filter_area, app);
        render_body(frame, body_area, app, now, &theme);
    } else {
        render_body(frame, inner, app, now, &theme);
    }

    if app.mode == Mode::Help {
        render_help(frame, area, app);
    }
}

fn render_body(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
    if app.repos.is_empty() {
        if app.scanning {
            render_loading(frame, area, app);
        } else {
            render_empty(frame, area, app);
        }
    } else {
        render_table(frame, area, app, now, theme);
    }
}

fn header_status(app: &App) -> String {
    let total = app.repos.len();
    let shown = app.visible_len();
    let mut status = if shown != total {
        format!("{shown}/{total} repos")
    } else {
        format!("{total} repos")
    };
    status.push_str(&format!(" · sort: {}", app.sort.label()));
    if app.dirty_only {
        status.push_str(" · dirty-only");
    }
    if app.scanning {
        status.push_str(&format!(" · {} scanning…", spinner_frame(app.spinner)));
    } else if let Some(msg) = &app.status {
        status.push_str(&format!(" · {msg}"));
    }
    status.push(' '); // small gap before the border corner
    status
}

fn footer_hints(app: &App) -> String {
    match app.mode {
        Mode::Filter => " type to filter · ↑/↓ move · ⏎ apply · Esc clear ".to_string(),
        Mode::Help => " ? / Esc close ".to_string(),
        Mode::Normal => {
            " j/k move · / filter · d dirty · s sort · ⏎ open · F fetch · p pull · L lazygit · ? help · q quit ".to_string()
        }
    }
}

fn render_filter_input(frame: &mut Frame, area: Rect, app: &App) {
    let line = Line::from(vec![
        Span::raw("/ "),
        Span::raw(app.filter.clone()),
        Span::raw("▏"),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_loading(frame: &mut Frame, area: Rect, app: &App) {
    let text = format!(
        "{} Scanning {}…",
        spinner_frame(app.spinner),
        roots_label(app)
    );
    let para = Paragraph::new(text).alignment(Alignment::Center);
    frame.render_widget(para, center_v(area, 1));
}

fn render_empty(frame: &mut Frame, area: Rect, app: &App) {
    let text = Text::from(vec![
        Line::from(format!("No git repos found under: {}", roots_label(app))),
        Line::from(""),
        Line::from("Run `cohors init` to create a config, or pass --root <dir>."),
    ]);
    let para = Paragraph::new(text).alignment(Alignment::Center);
    frame.render_widget(para, center_v(area, 3));
}

fn roots_label(app: &App) -> String {
    if app.roots.is_empty() {
        "(no roots configured)".to_string()
    } else {
        app.roots.join(", ")
    }
}

fn render_table(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
    let header = Row::new(["Repo", "Branch", "↑/↓", "Dirty", "Stash", "Last commit"])
        .style(Style::new().add_modifier(Modifier::BOLD));

    let view = app.view();
    let spin = spinner_frame(app.spinner);
    let rows: Vec<Row> = view
        .iter()
        .map(|vr| {
            let snap = &app.repos[vr.index];
            let busy = app.busy.contains(&snap.id).then_some(spin);
            repo_row(snap, &vr.name_highlights, now, theme, busy)
        })
        .collect();

    let widths = [
        Constraint::Min(16),    // Repo
        Constraint::Length(18), // Branch
        Constraint::Length(8),  // ↑/↓
        Constraint::Length(12), // Dirty
        Constraint::Length(5),  // Stash
        Constraint::Min(20),    // Last commit
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▌ ");

    let mut state = TableState::default();
    if !view.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn repo_row<'a>(
    snap: &'a RepoSnapshot,
    highlights: &[u32],
    now: i64,
    theme: &Theme,
    busy: Option<&str>,
) -> Row<'a> {
    if let Some(reason) = &snap.error {
        // Error repos show the reason in place of the normal columns.
        return Row::new(vec![
            name_cell(&snap.name, &[], false, theme),
            Cell::from(Span::styled(format!("⚠ {reason}"), theme.error())),
            Cell::default(),
            Cell::default(),
            Cell::default(),
            Cell::default(),
        ]);
    }

    let attention = snap.needs_attention();
    // While an action runs, the ↑/↓ cell shows a spinner instead.
    let arrows = match busy {
        Some(spin) => Cell::from(Span::styled(spin.to_string(), theme.ahead())),
        None => arrows_cell(snap, theme),
    };
    Row::new(vec![
        name_cell(&snap.name, highlights, attention, theme),
        branch_cell(snap, attention, theme),
        arrows,
        dirty_cell(snap, theme),
        stash_cell(snap, theme),
        commit_cell(snap, attention, now, theme),
    ])
}

fn name_cell<'a>(name: &str, highlights: &[u32], attention: bool, theme: &Theme) -> Cell<'a> {
    let base = if attention { Style::new() } else { theme.dim() };
    if highlights.is_empty() {
        return Cell::from(Span::styled(name.to_string(), base));
    }
    // Bold the fuzzy-matched characters.
    let hl = base.patch(theme.highlight());
    let spans: Vec<Span> = name
        .chars()
        .enumerate()
        .map(|(i, ch)| {
            let style = if highlights.contains(&(i as u32)) {
                hl
            } else {
                base
            };
            Span::styled(ch.to_string(), style)
        })
        .collect();
    Cell::from(Line::from(spans))
}

fn branch_cell<'a>(snap: &RepoSnapshot, attention: bool, theme: &Theme) -> Cell<'a> {
    let style = match snap.branch {
        Branch::Detached(_) => theme.detached(),
        _ if attention => Style::new(),
        _ => theme.dim(),
    };
    Cell::from(Span::styled(snap.branch.label(), style))
}

fn arrows_cell<'a>(snap: &RepoSnapshot, theme: &Theme) -> Cell<'a> {
    match &snap.upstream {
        None => Cell::from(Span::styled("—", theme.dim())),
        Some(up) if up.ahead == 0 && up.behind == 0 => Cell::from(Span::styled("·", theme.dim())),
        Some(up) => {
            let mut spans = Vec::new();
            if up.ahead > 0 {
                spans.push(Span::styled(format!("↑{}", up.ahead), theme.ahead()));
            }
            if up.ahead > 0 && up.behind > 0 {
                spans.push(Span::raw("·"));
            }
            if up.behind > 0 {
                spans.push(Span::styled(format!("↓{}", up.behind), theme.behind()));
            }
            Cell::from(Line::from(spans))
        }
    }
}

fn dirty_cell<'a>(snap: &RepoSnapshot, theme: &Theme) -> Cell<'a> {
    let w = &snap.worktree;
    if !w.is_dirty() {
        return Cell::from(Span::styled("·", theme.dim()));
    }
    let mut spans = Vec::new();
    if w.staged > 0 {
        spans.push(Span::styled(format!("●{}", w.staged), theme.staged()));
    }
    if w.modified > 0 {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(format!("+{}", w.modified), theme.modified()));
    }
    if w.untracked > 0 {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(format!("?{}", w.untracked), theme.untracked()));
    }
    Cell::from(Line::from(spans))
}

fn stash_cell<'a>(snap: &RepoSnapshot, theme: &Theme) -> Cell<'a> {
    if snap.stash_count > 0 {
        Cell::from(snap.stash_count.to_string())
    } else {
        Cell::from(Span::styled("·", theme.dim()))
    }
}

fn commit_cell<'a>(snap: &RepoSnapshot, attention: bool, now: i64, theme: &Theme) -> Cell<'a> {
    match &snap.last_commit {
        None => Cell::from(Span::styled("—", theme.dim())),
        Some(commit) => {
            let age = time::relative(commit.timestamp, now);
            let text = format!("{age}  {}", commit.summary);
            let style = if attention { Style::new() } else { theme.dim() };
            Cell::from(Span::styled(text, style))
        }
    }
}

fn render_help(frame: &mut Frame, full: Rect, app: &App) {
    let area = centered_rect(62, 80, full);
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from("Navigation").bold(),
        Line::from("  j / k, ↓ / ↑    move selection"),
        Line::from("  g / G           top / bottom"),
        Line::from(""),
        Line::from("View").bold(),
        Line::from("  /               fuzzy filter (Esc clears)"),
        Line::from("  d               toggle dirty-only"),
        Line::from("  s               cycle sort mode"),
        Line::from(""),
        Line::from("Actions").bold(),
        Line::from("  ⏎               open in editor"),
        Line::from("  o               reveal in file manager"),
        Line::from("  f / F           fetch selected / all"),
        Line::from("  p               pull (fast-forward only)"),
        Line::from("  L               open in lazygit"),
        Line::from("  y               copy path to clipboard"),
        Line::from(""),
        Line::from("App").bold(),
        Line::from("  r               refresh (re-scan)"),
        Line::from("  ?               toggle this help"),
        Line::from("  q / Ctrl-C      quit"),
        Line::from(""),
        Line::from(format!("cohors v{}", env!("CARGO_PKG_VERSION"))),
        Line::from(format!("config: {}", app.config_path)),
    ];
    let para = Paragraph::new(Text::from(lines))
        .block(Block::bordered().title(" Help "))
        .wrap(Wrap { trim: false });
    frame.render_widget(para, area);
}

/// A rect centered within `area`, sized as a percentage of it.
fn centered_rect(pct_x: u16, pct_y: u16, area: Rect) -> Rect {
    let [_, vmid, _] = Layout::vertical([
        Constraint::Percentage((100 - pct_y) / 2),
        Constraint::Percentage(pct_y),
        Constraint::Percentage((100 - pct_y) / 2),
    ])
    .areas(area);
    let [_, hmid, _] = Layout::horizontal([
        Constraint::Percentage((100 - pct_x) / 2),
        Constraint::Percentage(pct_x),
        Constraint::Percentage((100 - pct_x) / 2),
    ])
    .areas(vmid);
    hmid
}

/// A fixed-height rect centered vertically within `area`.
fn center_v(area: Rect, height: u16) -> Rect {
    let [_, mid, _] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(height),
        Constraint::Min(0),
    ])
    .areas(area);
    mid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Mode};
    use cohors_core::{Branch, CommitMeta, RepoId, RepoSnapshot, Upstream, WorktreeStatus};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Fixed "now" so relative commit ages render deterministically.
    const NOW: i64 = 1_700_000_000;

    #[allow(clippy::too_many_arguments)]
    fn snap(
        name: &str,
        branch: Branch,
        upstream: Option<(&str, u32, u32)>,
        worktree: (u32, u32, u32),
        stash: u32,
        commit: Option<(i64, &str)>,
        error: Option<&str>,
    ) -> RepoSnapshot {
        RepoSnapshot {
            id: RepoId(name.to_string()),
            name: name.to_string(),
            path: Some(camino::Utf8PathBuf::from(format!("/repos/{name}"))),
            branch,
            upstream: upstream.map(|(n, a, b)| Upstream {
                name: n.to_string(),
                ahead: a,
                behind: b,
            }),
            worktree: WorktreeStatus {
                staged: worktree.0,
                modified: worktree.1,
                untracked: worktree.2,
            },
            stash_count: stash,
            last_commit: commit.map(|(ts, summary)| CommitMeta {
                short_id: "a1b2c3d".to_string(),
                author: "Dev".to_string(),
                timestamp: ts,
                summary: summary.to_string(),
            }),
            error: error.map(str::to_string),
        }
    }

    fn demo_app() -> App {
        let mut app = App::new(
            vec!["~/projects".to_string(), "~/work".to_string()],
            "~/.config/cohors/config.toml".to_string(),
        );
        app.set_repos(vec![
            snap(
                "payments",
                Branch::Named("main".into()),
                Some(("origin/main", 2, 0)),
                (0, 3, 1),
                1,
                Some((NOW - 7200, "fix: retry on 5xx")),
                None,
            ),
            snap(
                "web-app",
                Branch::Named("feat/checkout".into()),
                Some(("origin/feat", 0, 5)),
                (0, 7, 0),
                0,
                Some((NOW - 1200, "wip: cart drawer")),
                None,
            ),
            snap(
                "auth-service",
                Branch::Named("main".into()),
                None,
                (0, 0, 0),
                0,
                Some((NOW - 259_200, "chore: bump deps")),
                None,
            ),
            snap(
                "infra",
                Branch::Detached("a1b2c3d".into()),
                None,
                (0, 0, 4),
                0,
                Some((NOW - 604_800, "build: pin ci image")),
                None,
            ),
            snap(
                "legacy-billing",
                Branch::Unborn,
                None,
                (0, 0, 0),
                0,
                None,
                Some("could not read .git (permission denied)"),
            ),
        ]);
        app
    }

    fn render_to_string(app: &App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, app, NOW)).unwrap();
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in buf.area.top()..buf.area.bottom() {
            for x in buf.area.left()..buf.area.right() {
                if let Some(cell) = buf.cell((x, y)) {
                    out.push_str(cell.symbol());
                }
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn snapshot_list() {
        let app = demo_app();
        insta::assert_snapshot!(render_to_string(&app, 92, 12));
    }

    #[test]
    fn snapshot_dirty_only() {
        let mut app = demo_app();
        app.dirty_only = true;
        insta::assert_snapshot!(render_to_string(&app, 92, 12));
    }

    #[test]
    fn snapshot_filtered() {
        let mut app = demo_app();
        app.mode = Mode::Filter;
        app.filter = "pay".to_string();
        insta::assert_snapshot!(render_to_string(&app, 92, 12));
    }

    #[test]
    fn snapshot_help() {
        let mut app = demo_app();
        app.mode = Mode::Help;
        insta::assert_snapshot!(render_to_string(&app, 92, 28));
    }

    #[test]
    fn snapshot_empty() {
        let app = App::new(vec!["~/projects".to_string()], "cfg".to_string());
        insta::assert_snapshot!(render_to_string(&app, 92, 10));
    }

    #[test]
    fn snapshot_loading() {
        let mut app = App::new(vec!["~/projects".to_string()], "cfg".to_string());
        app.scanning = true;
        insta::assert_snapshot!(render_to_string(&app, 92, 10));
    }
}
