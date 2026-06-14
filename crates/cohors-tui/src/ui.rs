//! Rendering. Pure view-model → widgets: every frame derives the visible rows
//! from [`App::view`] (i.e. `cohors-core`) and maps them onto ratatui widgets.
//! No state is mutated here.

use cohors_core::{
    Assessment, Branch, CiStatus, RepoSnapshot, Severity, assess, fleet_summary, time,
};
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
    fn warn(&self) -> Style {
        self.fg(Color::Yellow)
    }
    fn risk(&self) -> Style {
        self.fg(Color::Red)
    }
    fn ok(&self) -> Style {
        self.fg(Color::Green)
    }
    /// Color for an attention severity (the per-row reason + the summary).
    fn severity_style(&self, severity: Severity) -> Style {
        match severity {
            Severity::Risk => self.risk(),
            Severity::Warn => self.warn(),
            Severity::Notice => Style::new(),
            Severity::Info | Severity::Ok => self.dim(),
        }
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

    // A 1-line strip above the table: the fuzzy input while filtering, otherwise
    // the fleet triage summary (when there are repos to summarize).
    let body_area = if app.mode == Mode::Filter {
        let [strip, body] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
        render_filter_input(frame, strip, app);
        body
    } else if app.repos.is_empty() {
        inner
    } else {
        let [strip, body] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
        render_summary_strip(frame, strip, app, now, &theme);
        body
    };
    render_body(frame, body_area, app, now, &theme);

    if app.mode == Mode::Help {
        render_help(frame, area, app);
    }
}

/// The fleet triage summary: "N need attention" plus a chip per category.
fn render_summary_strip(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
    let s = fleet_summary(&app.repos, now);
    let mut spans: Vec<Span> = Vec::new();

    if s.needs_attention == 0 {
        spans.push(Span::styled("✓ all clear", theme.ok()));
    } else {
        spans.push(Span::styled(
            format!("⚠ {} need attention", s.needs_attention),
            Style::new().add_modifier(Modifier::BOLD),
        ));

        let mut chips: Vec<Span> = Vec::new();
        if s.unpushed > 0 {
            let mut label = format!("↑{} unpushed", s.unpushed);
            let style = if s.unpushed_aging > 0 {
                label.push_str(&format!(" ({} aging)", s.unpushed_aging));
                theme.risk()
            } else {
                theme.ahead()
            };
            chips.push(Span::styled(label, style));
        }
        if s.behind > 0 {
            chips.push(Span::styled(
                format!("↓{} behind", s.behind),
                theme.behind(),
            ));
        }
        if s.dirty > 0 {
            chips.push(Span::styled(
                format!("✎{} dirty", s.dirty),
                theme.modified(),
            ));
        }
        if s.stash > 0 {
            chips.push(Span::styled(format!("⚑{} stash", s.stash), theme.dim()));
        }
        if s.errors > 0 {
            chips.push(Span::styled(format!("⚠{} error", s.errors), theme.risk()));
        }
        for chip in chips {
            spans.push(Span::styled("  ·  ", theme.dim()));
            spans.push(chip);
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
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
            " ↑/↓ move · / filter · d dirty · s sort · ⏎ open · F fetch · p pull · L lazygit · ? help · q quit ".to_string()
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
    let header = Row::new([
        "Repo",
        "Branch",
        "↑/↓",
        "Dirty",
        "Stash",
        "Remote",
        "Last commit",
    ])
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
        Constraint::Min(14),    // Repo
        Constraint::Length(16), // Branch
        Constraint::Length(7),  // ↑/↓
        Constraint::Length(11), // Dirty
        Constraint::Length(5),  // Stash
        Constraint::Length(8),  // Remote (CI + PRs)
        Constraint::Min(16),    // Last commit
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
            Cell::default(),
        ]);
    }

    let assessment = assess(snap, now);
    let attention = assessment.needs_attention();
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
        remote_cell(snap, theme),
        status_cell(snap, &assessment, now, theme),
    ])
}

/// The Remote column: CI glyph (✓/✗/●/·) + open-PR count, or "—" when the repo
/// isn't on GitHub or hasn't been fetched yet.
fn remote_cell<'a>(snap: &RepoSnapshot, theme: &Theme) -> Cell<'a> {
    match &snap.remote {
        None => Cell::from(Span::styled("—", theme.dim())),
        Some(r) => {
            let (glyph, style) = match r.ci {
                CiStatus::Passing => ("✓", theme.ok()),
                CiStatus::Failing => ("✗", theme.risk()),
                CiStatus::Pending => ("●", theme.warn()),
                CiStatus::None => ("·", theme.dim()),
            };
            let mut spans = vec![Span::styled(glyph, style)];
            if r.open_prs > 0 {
                spans.push(Span::styled(format!(" {}pr", r.open_prs), theme.dim()));
            }
            Cell::from(Line::from(spans))
        }
    }
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

/// The rightmost column: for a repo needing attention, its primary reason
/// (colored by severity); otherwise the last commit's age + subject.
fn status_cell<'a>(
    snap: &RepoSnapshot,
    assessment: &Assessment,
    now: i64,
    theme: &Theme,
) -> Cell<'a> {
    let age = snap
        .last_commit
        .as_ref()
        .map(|c| time::relative(c.timestamp, now));
    let age = age.as_deref().unwrap_or("—");

    if let Some(primary) = &assessment.primary {
        Cell::from(Line::from(vec![
            Span::styled(format!("{age}  "), theme.dim()),
            Span::styled(primary.label(), theme.severity_style(assessment.severity)),
        ]))
    } else if let Some(commit) = &snap.last_commit {
        Cell::from(Span::styled(
            format!("{age}  {}", commit.summary),
            theme.dim(),
        ))
    } else {
        Cell::from(Span::styled("—", theme.dim()))
    }
}

fn render_help(frame: &mut Frame, full: Rect, app: &App) {
    let area = centered_rect(62, 80, full);
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from("Navigation").bold(),
        Line::from("  ↑ / ↓           move selection"),
        Line::from("  Home / End      top / bottom"),
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
            stash_latest: None,
            remote_url: None,
            remote: None,
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
