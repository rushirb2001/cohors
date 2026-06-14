//! Rendering. Pure view-model → widgets: every frame derives the visible rows
//! from [`App::view`] (i.e. `cohors-core`) and maps them onto ratatui widgets.
//! No state is mutated here.

use cohors_core::{Branch, CiStatus, RepoSnapshot, Severity, assess, fleet_summary, time};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Cell, Clear, Padding, Paragraph, Row, Table, TableState, Wrap,
};

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
}

/// Render the whole dashboard for one frame. `now` (Unix seconds) is injected
/// so relative commit ages are deterministic in tests.
pub fn render(frame: &mut Frame, app: &App, now: i64) {
    let theme = Theme::from_env();
    let area = frame.area();

    // Top-level layout: a title line, the body, and a footer hint line. There's
    // no outer frame, so the inner panels read as the app's "windows" and we
    // don't waste columns on nested borders.
    let [header_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);
    render_header(frame, header_area, app, &theme);
    render_footer(frame, footer_area, app, &theme);

    if app.repos.is_empty() {
        if app.scanning {
            render_loading(frame, body_area, app);
        } else {
            render_empty(frame, body_area, app);
        }
    } else {
        // A strip on top — the fuzzy input while filtering, otherwise the
        // Attention panel — then the Repositories panel fills the rest.
        let strip_height = if app.mode == Mode::Filter { 1 } else { 4 };
        let [strip, list] =
            Layout::vertical([Constraint::Length(strip_height), Constraint::Min(0)])
                .areas(body_area);
        if app.mode == Mode::Filter {
            render_filter_input(frame, strip, app);
        } else {
            render_attention_panel(frame, strip, app, now, &theme);
        }
        render_repos_panel(frame, list, app, now, &theme);
    }

    if app.mode == Mode::Help {
        render_help(frame, area, app);
    }
    if app.mode == Mode::Standup {
        render_standup(frame, area, app);
    }
}

/// The branded header: a rounded box with the tool name + version on the left
/// of the top border, the live repo count / sort / status on the right, and a
/// one-line description inside — cohors's "business card".
fn render_header(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let name = Span::styled("cohors", theme.ahead().add_modifier(Modifier::BOLD));
    let version = Span::styled(format!(" v{} ", env!("CARGO_PKG_VERSION")), theme.dim());
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.dim())
        .title(Line::from(vec![Span::raw(" "), name, version]))
        .title(Line::from(header_status(app)).right_aligned())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(Span::styled(
            "All your git repositories at a glance — status, fetch, pull & weekly standup.",
            theme.dim(),
        )),
        inner,
    );
}

/// The bottom line: context-sensitive key hints, dimmed.
fn render_footer(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    frame.render_widget(
        Paragraph::new(Span::styled(footer_hints(app), theme.dim())),
        area,
    );
}

/// The "Attention" panel: a titled box summarizing what needs the user, in
/// plain words ("3 dirty · 1 behind") rather than terse glyphs.
fn render_attention_panel(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
    let s = fleet_summary(&app.repos, now);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(" Attention ").bold())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    if s.needs_attention == 0 {
        lines.push(Line::from(Span::styled(
            format!("All {} repositories are up to date.", s.total),
            theme.ok(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!(
                "{} of {} repositories need attention:",
                s.needs_attention, s.total
            ),
            Style::new().add_modifier(Modifier::BOLD),
        )));

        // Readable, word-labeled chips; skip any category with a zero count.
        let mut items: Vec<(String, Style)> = Vec::new();
        if s.unpushed > 0 {
            if s.unpushed_aging > 0 {
                items.push((
                    format!("{} unpushed ({} aging)", s.unpushed, s.unpushed_aging),
                    theme.risk(),
                ));
            } else {
                items.push((format!("{} unpushed", s.unpushed), theme.ahead()));
            }
        }
        if s.behind > 0 {
            items.push((format!("{} behind", s.behind), theme.behind()));
        }
        if s.dirty > 0 {
            items.push((format!("{} dirty", s.dirty), theme.modified()));
        }
        if s.stash > 0 {
            items.push((format!("{} stashed", s.stash), Style::new()));
        }
        if s.errors > 0 {
            items.push((format!("{} unreadable", s.errors), theme.risk()));
        }

        let mut chips: Vec<Span> = Vec::new();
        for (i, (text, style)) in items.into_iter().enumerate() {
            if i > 0 {
                chips.push(Span::styled(" · ", theme.dim()));
            }
            chips.push(Span::styled(text, style));
        }
        lines.push(Line::from(chips));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
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
        Mode::Standup => " w window · y copy · Tab/Esc close ".to_string(),
        Mode::Normal => {
            " ↑/↓ move · / filter · d dirty · s sort · Tab standup · ⏎ open · F fetch · p pull · ? help · q quit ".to_string()
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

/// The "Repositories" panel: the repo table wrapped in a titled box. The column
/// headers act as the legend, so the bare numbers in each row read clearly.
fn render_repos_panel(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
    let title = format!(" Repositories ({}) ", app.visible_len());
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(title).bold());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let header = Row::new([
        "Repo",
        "Branch",
        "Sync",
        "Changes",
        "Stash",
        "Remote",
        "Last commit",
    ])
    .style(Style::new().add_modifier(Modifier::BOLD))
    .bottom_margin(1);

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
        Constraint::Length(18), // Repo
        Constraint::Length(13), // Branch
        Constraint::Length(7),  // Sync (ahead/behind)
        Constraint::Length(7),  // Changes (file count)
        Constraint::Length(5),  // Stash
        Constraint::Length(6),  // Remote (CI + PRs)
        Constraint::Fill(1),    // Last commit takes the remaining width
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(2)
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▌ ");

    let mut state = TableState::default();
    if !view.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(table, inner, &mut state);
}

fn repo_row<'a>(
    snap: &'a RepoSnapshot,
    highlights: &[u32],
    now: i64,
    theme: &Theme,
    busy: Option<&str>,
) -> Row<'a> {
    if let Some(reason) = &snap.error {
        // A broken repo: a red name + an "error" marker and the reason in the
        // wide last column. The data columns get a dim "·" (no data to report);
        // they must be non-empty or the table collapses them and misaligns.
        let dot = || Cell::from(Span::styled("·", theme.dim()));
        return Row::new(vec![
            Cell::from(Span::styled(snap.name.clone(), theme.error())),
            Cell::from(Span::styled("error", theme.risk())),
            dot(),
            dot(),
            dot(),
            dot(),
            Cell::from(Span::styled(reason.clone(), theme.dim())),
        ]);
    }

    let severity = assess(snap, now).severity;
    // While an action runs, the Sync cell shows a spinner instead.
    let sync = match busy {
        Some(spin) => Cell::from(Span::styled(spin.to_string(), theme.ahead())),
        None => sync_cell(snap, theme),
    };
    Row::new(vec![
        name_cell(&snap.name, highlights, severity, theme),
        branch_cell(snap, severity, theme),
        sync,
        changes_cell(snap, theme),
        stash_cell(snap, theme),
        remote_cell(snap, theme),
        last_commit_cell(snap, now, theme),
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

/// The repo name. Dimmed when clean, red when in a risk state, default
/// otherwise; fuzzy-matched characters are bold-highlighted.
fn name_cell<'a>(name: &str, highlights: &[u32], severity: Severity, theme: &Theme) -> Cell<'a> {
    let base = match severity {
        Severity::Ok | Severity::Info => theme.dim(),
        Severity::Risk => theme.risk(),
        _ => Style::new(),
    };
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

/// The branch — or a compact `@sha` for detached HEAD / "unborn" for a fresh
/// repo — ellipsized to the column width.
fn branch_cell<'a>(snap: &RepoSnapshot, severity: Severity, theme: &Theme) -> Cell<'a> {
    match &snap.branch {
        Branch::Detached(id) => {
            let short: String = id.chars().take(7).collect();
            Cell::from(Span::styled(format!("@{short}"), theme.detached()))
        }
        Branch::Unborn => Cell::from(Span::styled("unborn", theme.dim())),
        Branch::Named(name) => {
            let style = match severity {
                Severity::Ok | Severity::Info => theme.dim(),
                _ => Style::new(),
            };
            Cell::from(Span::styled(ellipsize(name, 13), style))
        }
    }
}

/// The Sync column: how far ahead/behind upstream, e.g. "↑2", "↓5", "↑2 ↓5".
/// "·" means even with upstream; "—" means no upstream is configured.
fn sync_cell<'a>(snap: &RepoSnapshot, theme: &Theme) -> Cell<'a> {
    match &snap.upstream {
        None => Cell::from(Span::styled("—", theme.dim())),
        Some(up) if up.ahead == 0 && up.behind == 0 => Cell::from(Span::styled("·", theme.dim())),
        Some(up) => {
            let mut spans = Vec::new();
            if up.ahead > 0 {
                spans.push(Span::styled(format!("↑{}", up.ahead), theme.ahead()));
            }
            if up.ahead > 0 && up.behind > 0 {
                spans.push(Span::raw(" "));
            }
            if up.behind > 0 {
                spans.push(Span::styled(format!("↓{}", up.behind), theme.behind()));
            }
            Cell::from(Line::from(spans))
        }
    }
}

/// The Changes column: a count of changed files, green when everything is
/// staged and yellow when there's still unstaged work. "·" when clean.
fn changes_cell<'a>(snap: &RepoSnapshot, theme: &Theme) -> Cell<'a> {
    let w = &snap.worktree;
    let total = w.staged + w.modified + w.untracked;
    if total == 0 {
        return Cell::from(Span::styled("·", theme.dim()));
    }
    let style = if w.modified > 0 || w.untracked > 0 {
        theme.modified()
    } else {
        theme.staged()
    };
    Cell::from(Span::styled(total.to_string(), style))
}

/// The Stash column: how many stashed entries, or "·" when there are none.
fn stash_cell<'a>(snap: &RepoSnapshot, theme: &Theme) -> Cell<'a> {
    if snap.stash_count > 0 {
        Cell::from(snap.stash_count.to_string())
    } else {
        Cell::from(Span::styled("·", theme.dim()))
    }
}

/// The Last commit column: the commit's age and subject. Why a repo needs the
/// user is carried by the row's colors, not repeated here as text.
fn last_commit_cell<'a>(snap: &'a RepoSnapshot, now: i64, theme: &Theme) -> Cell<'a> {
    match &snap.last_commit {
        Some(commit) => Cell::from(Line::from(vec![
            Span::styled(
                format!("{:>3}  ", time::relative(commit.timestamp, now)),
                theme.dim(),
            ),
            Span::raw(commit.summary.clone()),
        ])),
        None => Cell::from(Span::styled("—", theme.dim())),
    }
}

/// Truncate `s` to at most `max` characters, adding an ellipsis when cut.
fn ellipsize(s: &str, max: usize) -> String {
    let len = s.chars().count();
    if len <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
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
        Line::from("  Tab             weekly standup"),
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
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Help ")
                .padding(Padding::horizontal(1)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(para, area);
}

/// The standup overlay: the rendered markdown digest for the current window,
/// or a "collecting" placeholder while the worker thread walks the commits.
fn render_standup(frame: &mut Frame, full: Rect, app: &App) {
    let area = centered_rect(70, 80, full);
    frame.render_widget(Clear, area);
    let text = match &app.standup {
        Some(md) => Text::from(standup_lines(md)),
        None => Text::from(Line::from(Span::styled(
            format!("Collecting commits for {}…", app.standup_window.label()),
            Style::new().add_modifier(Modifier::DIM),
        ))),
    };
    let para = Paragraph::new(text)
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Standup ")
                .padding(Padding::horizontal(1)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(para, area);
}

/// Lightly style the standup markdown for the terminal: headings bold, the
/// summary line dim, commit bullets as-is.
fn standup_lines(md: &str) -> Vec<Line<'static>> {
    md.lines()
        .map(|line| {
            if let Some(rest) = line.strip_prefix("### ") {
                Line::from(Span::styled(
                    rest.to_string(),
                    Style::new().add_modifier(Modifier::BOLD),
                ))
            } else if let Some(rest) = line.strip_prefix("## ") {
                Line::from(Span::styled(
                    rest.to_string(),
                    Style::new().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                ))
            } else if line.len() > 1 && line.starts_with('_') && line.ends_with('_') {
                Line::from(Span::styled(
                    line.trim_matches('_').to_string(),
                    Style::new().add_modifier(Modifier::DIM),
                ))
            } else {
                Line::from(line.to_string())
            }
        })
        .collect()
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
        insta::assert_snapshot!(render_to_string(&app, 100, 20));
    }

    #[test]
    fn snapshot_dirty_only() {
        let mut app = demo_app();
        app.dirty_only = true;
        insta::assert_snapshot!(render_to_string(&app, 100, 20));
    }

    #[test]
    fn snapshot_filtered() {
        let mut app = demo_app();
        app.mode = Mode::Filter;
        app.filter = "pay".to_string();
        insta::assert_snapshot!(render_to_string(&app, 100, 20));
    }

    #[test]
    fn snapshot_help() {
        let mut app = demo_app();
        app.mode = Mode::Help;
        insta::assert_snapshot!(render_to_string(&app, 100, 30));
    }

    #[test]
    fn snapshot_standup() {
        use cohors_core::{StandupCommit, StandupWindow, to_markdown};
        let mut app = demo_app();
        app.mode = Mode::Standup;
        app.standup_window = StandupWindow::Week;
        let commits = vec![
            StandupCommit {
                repo: "payments".into(),
                short_id: "a1b2c3d".into(),
                summary: "fix: retry on 5xx".into(),
                timestamp: NOW - 7200,
            },
            StandupCommit {
                repo: "web-app".into(),
                short_id: "e4f5a6b".into(),
                summary: "wip: cart drawer".into(),
                timestamp: NOW - 1200,
            },
            StandupCommit {
                repo: "payments".into(),
                short_id: "c7d8e9f".into(),
                summary: "test: add retry coverage".into(),
                timestamp: NOW - 90_000,
            },
        ];
        app.standup = Some(to_markdown(&commits, StandupWindow::Week));
        insta::assert_snapshot!(render_to_string(&app, 100, 22));
    }

    #[test]
    fn snapshot_standup_collecting() {
        let mut app = demo_app();
        app.mode = Mode::Standup;
        app.standup = None;
        insta::assert_snapshot!(render_to_string(&app, 100, 18));
    }

    #[test]
    fn snapshot_empty() {
        let app = App::new(vec!["~/projects".to_string()], "cfg".to_string());
        insta::assert_snapshot!(render_to_string(&app, 100, 12));
    }

    #[test]
    fn snapshot_loading() {
        let mut app = App::new(vec!["~/projects".to_string()], "cfg".to_string());
        app.scanning = true;
        insta::assert_snapshot!(render_to_string(&app, 100, 12));
    }
}
