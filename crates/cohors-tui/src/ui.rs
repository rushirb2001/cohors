//! Rendering. Pure view-model → widgets: every frame derives the visible rows
//! from [`App::view`] (i.e. `cohors-core`) and maps them onto ratatui widgets.
//! No state is mutated here.

use cohors_core::{Branch, CiStatus, RepoSnapshot, Severity, assess, fleet_summary, time};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Cell, Clear, List, ListItem, ListState, Padding, Paragraph, Row, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Table, TableState, Wrap,
};

use crate::app::{App, ConfirmAction, Mode, RunState};

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

    // Top-level layout: the branded header box, the body, and a boxed key-hint
    // footer — one labelled group per row, each wrapping if the terminal is
    // narrow, so no command is ever truncated.
    let footer = footer_lines(app, &theme);
    let footer_inner = area.width.saturating_sub(4).max(1); // 2 border + 2 padding
    let footer_rows: u16 = footer
        .iter()
        .map(|l| (l.width() as u16).max(1).div_ceil(footer_inner))
        .sum::<u16>()
        .clamp(1, 6);
    let [header_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(footer_rows + 2),
    ])
    .areas(area);
    render_header(frame, header_area, app, &theme);
    render_footer(frame, footer_area, footer, &theme);

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

    // Dim the whole frame behind a modal overlay so the background recedes and
    // the overlay stands out. The overlays `Clear` their own area, so they
    // render crisp on top of the dimmed background.
    let overlay_open = matches!(
        app.mode,
        Mode::Help | Mode::Standup | Mode::CommandInput | Mode::CommandRun | Mode::Confirm
    );
    if overlay_open {
        dim_area(frame.buffer_mut(), area);
    }

    if app.mode == Mode::Help {
        render_help(frame, area, app, &theme);
    }
    if app.mode == Mode::Standup {
        render_standup(frame, area, app, &theme);
    }
    if app.mode == Mode::CommandInput {
        render_command_input(frame, area, app, &theme);
    }
    if app.mode == Mode::CommandRun {
        render_command_run(frame, area, app, &theme);
    }
    if app.mode == Mode::Confirm {
        render_confirm(frame, area, app, &theme);
    }
}

/// Dim every cell in `area` (keeps its colours, adds the DIM attribute) — used
/// to fade the background behind a modal overlay.
fn dim_area(buf: &mut Buffer, area: Rect) {
    let dim = Style::new().add_modifier(Modifier::DIM);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(dim);
            }
        }
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
        .title(header_status_line(app, theme).right_aligned())
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

/// The footer: context-sensitive key hints in a box, one labelled group per row,
/// wrapping on a narrow terminal so no command is ever truncated.
fn render_footer(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>, theme: &Theme) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.dim())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner,
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

/// The right-aligned header line: repo count, selection, sort, and the transient
/// status. The status is styled to stand out — green for a confirmation, red for
/// a failure — instead of blending into the dim metadata.
fn header_status_line(app: &App, theme: &Theme) -> Line<'static> {
    let total = app.repos.len();
    let shown = app.visible_len();
    let mut meta = if shown != total {
        format!("{shown}/{total} repos")
    } else {
        format!("{total} repos")
    };
    if !app.selection.is_empty() {
        meta.push_str(&format!(" · {} selected", app.selection.len()));
    }
    meta.push_str(&format!(" · sort: {}", app.sort.label()));
    if app.dirty_only {
        meta.push_str(" · dirty-only");
    }

    let mut spans = vec![Span::styled(meta, theme.dim())];
    if app.scanning {
        spans.push(Span::styled(
            format!(" · {} scanning…", spinner_frame(app.spinner)),
            theme.dim(),
        ));
    } else if let Some(msg) = &app.status {
        // A failure reads red; everything else is a confirmation (green + ✓).
        let failed = msg.contains("fail") || msg.contains("error") || msg.starts_with("no ");
        let (prefix, style) = if failed {
            ("", theme.risk())
        } else {
            ("✓ ", theme.ok().add_modifier(Modifier::BOLD))
        };
        spans.push(Span::styled(" · ", theme.dim()));
        spans.push(Span::styled(format!("{prefix}{msg}"), style));
    }
    spans.push(Span::raw(" ")); // small gap before the border corner
    Line::from(spans)
}

/// The footer key hints for the current mode, in labelled groups. Each group is
/// `(group label, [(key, what it does)])` and renders on its own row, so the
/// controls read like a small legend: the group label says what the keys are
/// for (e.g. the "act" keys act on the marked repos).
fn footer_groups(app: &App) -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    match app.mode {
        Mode::Filter => vec![(
            "filter",
            vec![("type", "to filter"), ("⏎", "apply"), ("Esc", "clear")],
        )],
        Mode::Help => vec![("", vec![("? / Esc", "close")])],
        Mode::Standup => {
            if app.standup.as_ref().is_some_and(|s| s.commits_focused) {
                vec![(
                    "",
                    vec![
                        ("↑/↓", "scroll commits"),
                        ("←", "back to repos"),
                        ("w", "window"),
                        ("y", "copy"),
                        ("q", "close"),
                    ],
                )]
            } else {
                vec![(
                    "",
                    vec![
                        ("↑/↓", "repo"),
                        ("→/⏎", "view commits"),
                        ("w", "window"),
                        ("y", "copy"),
                        ("q", "close"),
                    ],
                )]
            }
        }
        Mode::CommandInput => vec![(
            "",
            vec![("type", "a command"), ("⏎", "run it"), ("Esc", "cancel")],
        )],
        Mode::CommandRun => vec![(
            "",
            vec![
                ("↑/↓", "repo"),
                ("PgUp/PgDn", "scroll output"),
                ("y", "copy"),
                ("Esc", "close"),
            ],
        )],
        Mode::Confirm => vec![("", vec![("y", "confirm"), ("N / Esc", "cancel")])],
        Mode::Normal => vec![
            (
                "select",
                vec![
                    ("Space", "mark repo"),
                    ("a", "mark all"),
                    ("/", "filter"),
                    ("Esc", "clear"),
                ],
            ),
            (
                "act",
                vec![
                    ("⏎", "open"),
                    ("f", "fetch"),
                    ("p", "pull"),
                    ("!", "run command"),
                    ("S", "stash"),
                ],
            ),
            (
                "view",
                vec![
                    ("↑/↓", "move"),
                    ("s", "sort"),
                    ("d", "dirty-only"),
                    ("Tab", "standup"),
                    ("?", "help"),
                    ("q", "quit"),
                ],
            ),
        ],
    }
}

/// Build one styled footer row per group: a dim group label, then each key in an
/// accent colour with its dimmed description, `·`-separated.
fn footer_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let key_style = theme.ahead().add_modifier(Modifier::BOLD);
    footer_groups(app)
        .into_iter()
        .map(|(label, items)| {
            let mut spans: Vec<Span> = Vec::new();
            if !label.is_empty() {
                spans.push(Span::styled(format!("{label:<7}"), theme.dim()));
            }
            for (i, (key, desc)) in items.into_iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled(" · ", theme.dim()));
                }
                spans.push(Span::styled(key, key_style));
                spans.push(Span::styled(format!(" {desc}"), theme.dim()));
            }
            // On the "act" row, spell out *what* an action will hit — the marked
            // set, or the repo under the cursor when nothing is marked — so the
            // selection model isn't a hidden rule the user has to infer.
            if label == "act"
                && let Some((arrow, target)) = action_target_hint(app)
            {
                spans.push(Span::styled(arrow, theme.dim()));
                spans.push(Span::styled(target, theme.highlight()));
            }
            Line::from(spans)
        })
        .collect()
}

/// The "what will an action affect" hint for the footer: the count of marked
/// repos, or the current repo's name when nothing is marked. Returns the dim
/// lead-in and the highlighted target separately. `None` when there's nothing.
fn action_target_hint(app: &App) -> Option<(String, String)> {
    let n = app.selection.len();
    if n > 0 {
        Some(("   → acts on ".to_string(), format!("{n} selected")))
    } else {
        app.selected_repo()
            .map(|r| ("   → acts on ".to_string(), r.name.clone()))
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

    // "Repo" is padded by 2 so it lines up with the names, which carry a 2-col
    // selection gutter (`● ` / `  `) inside their cell.
    let header = Row::new(["  Repo", "Branch", "Sync", "Changes", "Last commit"])
        .style(Style::new().add_modifier(Modifier::BOLD))
        .bottom_margin(1);

    let view = app.view();
    let spin = spinner_frame(app.spinner);
    let rows: Vec<Row> = view
        .iter()
        .map(|vr| {
            let snap = &app.repos[vr.index];
            let busy = app.busy.contains(&snap.id).then_some(spin);
            let marked = app.selection.contains(&snap.id);
            repo_row(snap, &vr.name_highlights, now, theme, busy, marked)
        })
        .collect();

    // Size the Sync and Changes columns to their actual content so they stay
    // tight (a fleet with no PRs gets a narrow Sync, etc.) rather than always
    // reserving room for the widest possible case. The floor is the header label
    // width ("Sync" = 4, "Changes" = 7); the ceiling guards against one outlier
    // repo stretching the whole column.
    let sync_w = view
        .iter()
        .map(|vr| line_width(&sync_spans(&app.repos[vr.index], theme)))
        .max()
        .unwrap_or(0)
        .clamp(4, 12);
    let changes_w = view
        .iter()
        .map(|vr| line_width(&changes_spans(&app.repos[vr.index], theme)))
        .max()
        .unwrap_or(0)
        .clamp(7, 12);

    let widths = [
        Constraint::Length(18),        // Repo (incl. 2-col selection gutter)
        Constraint::Length(13),        // Branch
        Constraint::Length(sync_w),    // Sync (ahead/behind + remote CI/PRs)
        Constraint::Length(changes_w), // Changes (working tree + stash)
        Constraint::Fill(1),           // Last commit takes the remaining width
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
    marked: bool,
) -> Row<'a> {
    if let Some(reason) = &snap.error {
        // A broken repo: a red name + an "error" marker and the reason in the
        // wide last column. The data columns get a dim "·" (no data to report);
        // they must be non-empty or the table collapses them and misaligns.
        // The leading "  " keeps the name aligned with the selection gutter.
        let dot = || Cell::from(Span::styled("·", theme.dim()));
        return Row::new(vec![
            Cell::from(Line::from(vec![
                Span::raw("  "),
                Span::styled(snap.name.clone(), theme.error()),
            ])),
            Cell::from(Span::styled("error", theme.risk())),
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
        name_cell(&snap.name, highlights, severity, marked, theme),
        branch_cell(snap, severity, theme),
        sync,
        changes_cell(snap, theme),
        last_commit_cell(snap, now, theme),
    ])
}

/// The remote sub-part of the Sync column: a status dot colored by CI health —
/// green passing, red failing, yellow pending, dim when there's no CI signal —
/// plus the open-PR count. Empty when the repo isn't on a remote.
///
/// We use `●` (a basic geometric glyph present in every monospace font, colored
/// via ANSI like the rest of the UI) rather than a cloud emoji: emoji are
/// double-width, can't be themed/`NO_COLOR`'d, and — as with `☁` (U+2601) — may
/// have no text glyph in the user's font and render invisibly.
fn remote_spans(snap: &RepoSnapshot, theme: &Theme) -> Vec<Span<'static>> {
    match &snap.remote {
        None => Vec::new(),
        Some(r) => {
            let style = match r.ci {
                CiStatus::Passing => theme.ok(),
                CiStatus::Failing => theme.risk(),
                CiStatus::Pending => theme.warn(),
                CiStatus::None => theme.dim(),
            };
            let mut spans = vec![Span::styled("●", style)];
            if r.open_prs > 0 {
                spans.push(Span::styled(format!(" {}pr", r.open_prs), theme.dim()));
            }
            spans
        }
    }
}

/// The repo name, preceded by a selection gutter: a cyan `●` when the repo is
/// marked for a bulk action, two spaces otherwise (so names stay aligned). The
/// name is dimmed when clean, red in a risk state, default otherwise;
/// fuzzy-matched characters are bold-highlighted.
fn name_cell<'a>(
    name: &str,
    highlights: &[u32],
    severity: Severity,
    marked: bool,
    theme: &Theme,
) -> Cell<'a> {
    let base = match severity {
        Severity::Ok | Severity::Info => theme.dim(),
        Severity::Risk => theme.risk(),
        _ => Style::new(),
    };
    let gutter = if marked {
        Span::styled("● ", theme.ahead())
    } else {
        Span::raw("  ")
    };
    let mut spans = vec![gutter];
    if highlights.is_empty() {
        spans.push(Span::styled(name.to_string(), base));
    } else {
        // Bold the fuzzy-matched characters.
        let hl = base.patch(theme.highlight());
        spans.extend(name.chars().enumerate().map(|(i, ch)| {
            let style = if highlights.contains(&(i as u32)) {
                hl
            } else {
                base
            };
            Span::styled(ch.to_string(), style)
        }));
    }
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

/// The Sync column: the local-vs-upstream state *and* the remote health, fused.
/// First the upstream delta (`↑2`, `↓5`, `↑2 ↓5`, or `·` when even), then the
/// remote dot + open-PR count (`●`, `● 2pr`). Examples: `↑2 ● 2pr`, `↓5 ●`,
/// `· ●`. "—" when the repo has neither an upstream nor a remote (purely local).
fn sync_cell<'a>(snap: &RepoSnapshot, theme: &Theme) -> Cell<'a> {
    Cell::from(Line::from(sync_spans(snap, theme)))
}

/// The styled segments of the Sync cell. Exposed (rather than built inline) so
/// the column can be sized to its widest content — see [`line_width`].
fn sync_spans(snap: &RepoSnapshot, theme: &Theme) -> Vec<Span<'static>> {
    let remote = remote_spans(snap, theme);

    // The upstream sub-part: ahead/behind arrows when the branch has diverged.
    // When it's even with upstream we'd normally show "·", but that's redundant
    // next to the remote dot (which already says "this is tracked") — so we only
    // emit the "·" when there's no remote dot to stand in for it.
    let upstream: Vec<Span> = match &snap.upstream {
        None => Vec::new(),
        Some(up) if up.ahead == 0 && up.behind == 0 => {
            if remote.is_empty() {
                vec![Span::styled("·", theme.dim())]
            } else {
                Vec::new()
            }
        }
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
            spans
        }
    };

    // Join the two sub-parts with a space; fall back to "—" when both are empty.
    let mut spans = upstream;
    if !spans.is_empty() && !remote.is_empty() {
        spans.push(Span::raw(" "));
    }
    spans.extend(remote);
    if spans.is_empty() {
        spans.push(Span::styled("—", theme.dim()));
    }
    spans
}

/// The Changes column: changed-file count (green when all staged, yellow when
/// there's unstaged work), plus the stash folded in as a dim `s{n}` when there
/// are stashes. "·" when the tree is clean and nothing is stashed.
fn changes_cell<'a>(snap: &RepoSnapshot, theme: &Theme) -> Cell<'a> {
    Cell::from(Line::from(changes_spans(snap, theme)))
}

/// The styled segments of the Changes cell. Exposed (rather than built inline)
/// so the column can be sized to its widest content — see [`line_width`].
fn changes_spans(snap: &RepoSnapshot, theme: &Theme) -> Vec<Span<'static>> {
    let w = &snap.worktree;
    let total = w.staged + w.modified + w.untracked;
    let mut spans: Vec<Span> = Vec::new();
    if total == 0 {
        spans.push(Span::styled("·", theme.dim()));
    } else {
        let style = if w.modified > 0 || w.untracked > 0 {
            theme.modified()
        } else {
            theme.staged()
        };
        spans.push(Span::styled(total.to_string(), style));
    }
    if snap.stash_count > 0 {
        spans.push(Span::styled(format!(" s{}", snap.stash_count), theme.dim()));
    }
    spans
}

/// The display width (in terminal columns) of a run of spans — all of cohors's
/// glyphs are single-width, so a `char` count is exact. Used to size data
/// columns to their actual content rather than a fixed guess.
fn line_width(spans: &[Span]) -> u16 {
    spans.iter().map(|s| s.content.chars().count() as u16).sum()
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

fn render_help(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    let area = centered_rect(66, 88, full);
    frame.render_widget(Clear, area);

    // A small helper: a styled span, so the legend can show the *real* colored
    // glyphs next to their meaning rather than describing them in words.
    let s = |t: &str, st: Style| Span::styled(t.to_string(), st);
    let dim = |t: &str| s(t, theme.dim());

    let lines = vec![
        // The legend goes first: it answers "what am I looking at?", which is the
        // question a new user has before they care about keybindings.
        Line::from("Legend  —  what the columns show").bold(),
        Line::from(vec![
            dim("  Sync     "),
            s("↑2", theme.ahead()),
            dim(" ahead  "),
            s("↓5", theme.behind()),
            dim(" behind  "),
            dim("· even  — local"),
        ]),
        Line::from(vec![
            dim("           "),
            s("●", theme.ok()),
            s("●", theme.risk()),
            s("●", theme.warn()),
            dim(" CI passing/failing/pending  "),
            dim("2pr"),
            dim(" open PRs"),
        ]),
        Line::from(vec![
            dim("  Changes  "),
            s("3", theme.staged()),
            dim(" all staged  "),
            s("3", theme.modified()),
            dim(" unstaged work  "),
            dim("s1"),
            dim(" one stash"),
        ]),
        Line::from(vec![
            dim("  Rows     "),
            s("name", theme.dim()),
            dim(" clean  "),
            s("name", theme.risk()),
            dim(" needs attention  "),
            s("●", theme.ahead()),
            dim(" marked for an action"),
        ]),
        Line::from(""),
        Line::from("Navigation").bold(),
        Line::from("  ↑ / ↓           move cursor"),
        Line::from("  Home / End      top / bottom"),
        Line::from(""),
        Line::from("Select  &  filter").bold(),
        Line::from("  Space           mark / unmark the repo"),
        Line::from("  a               mark all (again to clear)"),
        Line::from("  Esc             clear the selection"),
        Line::from("  /               fuzzy filter (Esc clears)"),
        Line::from("  d               toggle dirty-only"),
        Line::from("  s               cycle sort mode"),
        Line::from("  Tab             weekly standup"),
        Line::from(""),
        Line::from("Actions  (on the marked repos, or the current one)").bold(),
        Line::from("  ⏎               open in editor"),
        Line::from("  o               reveal in file manager"),
        Line::from("  f / F           fetch selection / all"),
        Line::from("  p               pull (fast-forward only)"),
        Line::from("  !               run a command across them"),
        Line::from("  S               stash (asks to confirm)"),
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

/// The standup overlay: a "Repos" box (per-repo commit counts) beside a box of
/// the focused repo's commits, scrollable. The active pane's title is bold; the
/// other is dim. `None` shows a "collecting" placeholder.
/// Below this inner width a two-pane overlay can't show both panes side-by-side
/// without squeezing one of them, so it stacks the list on top instead.
const TWO_PANE_MIN_WIDTH: u16 = 64;

/// Lay out a two-pane overlay (a list + a detail view). On a wide terminal the
/// panes sit side-by-side (`list_w` wide list, gap, detail fills the rest); on a
/// narrow one the list stacks on top (`stacked_list_h` tall) so neither pane is
/// squeezed. Returns `(list_area, detail_area)`.
fn two_pane(inner: Rect, list_w: u16, stacked_list_h: u16) -> (Rect, Rect) {
    if inner.width >= TWO_PANE_MIN_WIDTH {
        let [list, detail] = Layout::horizontal([Constraint::Length(list_w), Constraint::Min(0)])
            .spacing(2)
            .areas(inner);
        (list, detail)
    } else {
        let [list, detail] =
            Layout::vertical([Constraint::Length(stacked_list_h), Constraint::Min(0)])
                .spacing(1)
                .areas(inner);
        (list, detail)
    }
}

fn render_standup(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    let area = centered_rect(84, 86, full);
    frame.render_widget(Clear, area);

    let window = app.standup_window.label();
    // The outer frame, padded so the inner boxes breathe.
    let outer = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(format!(" Standup · {window} ")).bold())
        .padding(Padding::new(1, 1, 1, 0));

    // Still walking the commits: a placeholder.
    let Some(view) = &app.standup else {
        let inner = outer.inner(area);
        frame.render_widget(outer, area);
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!("Collecting your commits for {window}…"),
                theme.dim(),
            )),
            inner,
        );
        return;
    };

    let total = view.commits.len();
    let repos = view.groups.len();
    let summary = format!(
        " {total} commit{} · {repos} repo{} ",
        if total == 1 { "" } else { "s" },
        if repos == 1 { "" } else { "s" },
    );
    let outer = outer.title_bottom(Line::from(summary).right_aligned());
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if view.groups.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(format!("No commits {window}."), theme.dim())),
            inner,
        );
        return;
    }

    // A "Repos" box and a "{repo}" commits box: side-by-side when there's room,
    // stacked (repos on top) on a narrow terminal.
    let (left_area, right_area) = two_pane(inner, 26, 8);

    // Highlight the active pane's title (bold) and dim the inactive one.
    let active = |on: bool| {
        if on {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            theme.dim()
        }
    };

    // Left: the repos with commit counts.
    let repos_block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(" Repos ").style(active(!view.commits_focused)))
        .padding(Padding::horizontal(1));
    let repos_inner = repos_block.inner(left_area);
    frame.render_widget(repos_block, left_area);
    let items: Vec<ListItem> = view
        .groups
        .iter()
        .map(|(repo, commits)| {
            ListItem::new(Line::from(vec![
                Span::raw(ellipsize(repo, 16)),
                Span::styled(format!("  {}", commits.len()), theme.dim()),
            ]))
        })
        .collect();
    let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut list_state = ListState::default();
    list_state.select(Some(view.focus));
    frame.render_stateful_widget(list, repos_inner, &mut list_state);

    // Right: the focused repo's commits, scrollable.
    let repo_name = view
        .groups
        .get(view.focus)
        .map(|(r, _)| r.as_str())
        .unwrap_or("");
    let commits_block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(format!(" {repo_name} ")).style(active(view.commits_focused)))
        .padding(Padding::new(1, 0, 0, 0));
    let commits_inner = commits_block.inner(right_area);
    frame.render_widget(commits_block, right_area);

    let lines: Vec<Line> = view
        .groups
        .get(view.focus)
        .map(|(_, commits)| {
            commits
                .iter()
                .map(|c| {
                    Line::from(vec![
                        Span::styled(c.short_id.clone(), theme.ahead()),
                        Span::raw("  "),
                        Span::raw(c.summary.clone()),
                    ])
                })
                .collect()
        })
        .unwrap_or_default();
    let total_lines = lines.len() as u16;
    let viewport = commits_inner.height;
    let max_scroll = total_lines.saturating_sub(viewport);
    view.set_max_scroll(max_scroll);
    let offset = view.scroll.min(max_scroll);

    let [text_area, bar_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).areas(commits_inner);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).scroll((offset, 0)),
        text_area,
    );
    if max_scroll > 0 {
        let mut sb = ScrollbarState::new(total_lines as usize)
            .position(offset as usize)
            .viewport_content_length(viewport as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            bar_area,
            &mut sb,
        );
    }
}

/// The command-input overlay: a small box to type the command to run, showing
/// how many repos it will run across.
fn render_command_input(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    let area = centered_rect(70, 20, full);
    frame.render_widget(Clear, area);
    let n = app.action_targets().len();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(format!(" Run command · {n} repos ")).bold())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let line = Line::from(vec![
        Span::styled("$ ", theme.dim()),
        Span::raw(app.command_input.clone()),
        Span::raw("▏"),
    ]);
    frame.render_widget(Paragraph::new(line), inner);
}

/// The command-run overlay: a left list of repos (status glyph + name, the
/// focused one highlighted) and the focused repo's scrollable output on the
/// right, with a live combined pass/fail summary in the bottom border.
fn render_command_run(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    let area = centered_rect(88, 85, full);
    frame.render_widget(Clear, area);
    let Some(run) = &app.run else {
        return;
    };
    let (ok, fail, running) = run.summary();
    let summary = if running > 0 {
        format!(" {ok} ✓ · {fail} ✗ · {running} running ")
    } else {
        format!(" {ok} ✓ · {fail} ✗ ")
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(format!(" Run · {} ", run.command)).bold())
        .title_bottom(Line::from(summary).right_aligned())
        .padding(Padding::new(1, 0, 0, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Left: the repo list (with focus highlight). Right: the focused output.
    // Stacks (list on top) on a narrow terminal so neither pane is squeezed.
    let (list_area, out_area) = two_pane(inner, 26, 6);

    let spin = spinner_frame(app.spinner);
    let items: Vec<ListItem> = run
        .results
        .iter()
        .map(|r| {
            let (glyph, style, note) = match &r.state {
                RunState::Running => (spin.to_string(), theme.ahead(), String::new()),
                RunState::Done { code: 0, .. } => ("✓".to_string(), theme.ok(), String::new()),
                RunState::Done { code, .. } => ("✗".to_string(), theme.risk(), format!(" {code}")),
            };
            ListItem::new(Line::from(vec![
                Span::styled(glyph, style),
                Span::raw(" "),
                Span::raw(ellipsize(&r.name, 18)),
                Span::styled(note, theme.dim()),
            ]))
        })
        .collect();
    let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut list_state = ListState::default();
    if !run.results.is_empty() {
        list_state.select(Some(run.focus));
    }
    frame.render_stateful_widget(list, list_area, &mut list_state);

    // Right: the focused repo's output (stdout, then a dim stderr divider).
    let out_lines = run_output_lines(run, theme);
    let total = out_lines.len() as u16;
    let viewport = out_area.height;
    let max_scroll = total.saturating_sub(viewport);
    run.set_max_scroll(max_scroll);
    let offset = run.scroll.min(max_scroll);

    let [text_area, bar_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).areas(out_area);
    frame.render_widget(
        Paragraph::new(Text::from(out_lines)).scroll((offset, 0)),
        text_area,
    );
    if max_scroll > 0 {
        let mut sb = ScrollbarState::new(total as usize)
            .position(offset as usize)
            .viewport_content_length(viewport as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            bar_area,
            &mut sb,
        );
    }
}

/// The lines for the focused repo's output pane.
fn run_output_lines(run: &crate::app::CommandRun, theme: &Theme) -> Vec<Line<'static>> {
    match run.results.get(run.focus).map(|r| &r.state) {
        Some(RunState::Done {
            code,
            stdout,
            stderr,
        }) => {
            let exit_style = if *code == 0 { theme.ok() } else { theme.risk() };
            let mut lines = vec![
                Line::from(Span::styled(format!("exit {code}"), exit_style)),
                Line::from(""),
            ];
            for l in stdout.lines() {
                lines.push(Line::from(l.to_string()));
            }
            if !stderr.is_empty() {
                lines.push(Line::from(Span::styled("── stderr ──", theme.dim())));
                for l in stderr.lines() {
                    lines.push(Line::from(Span::styled(l.to_string(), theme.warn())));
                }
            }
            lines
        }
        Some(RunState::Running) => vec![Line::from(Span::styled("running…", theme.dim()))],
        None => Vec::new(),
    }
}

/// The confirmation modal: the prompt, the affected repos, and y/N choices with
/// No as the default. Reached only via a deliberate action key (ADR-008/021).
fn render_confirm(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    let Some(pending) = &app.confirm else {
        return;
    };
    let names: Vec<&str> = match &pending.action {
        ConfirmAction::BulkStash(ids) => ids
            .iter()
            .filter_map(|id| {
                app.repos
                    .iter()
                    .find(|r| &r.id == id)
                    .map(|r| r.name.as_str())
            })
            .collect(),
    };
    let area = centered_rect(54, 32, full);
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(" Confirm ").bold())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = vec![
        Line::from(Span::styled(
            pending.prompt.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(names.join(", "), theme.dim())),
        Line::from(""),
        Line::from(vec![
            Span::styled("[y]", theme.ok()),
            Span::raw(" yes    "),
            Span::styled("[N]", theme.warn()),
            Span::raw(" no    "),
            Span::styled("(Esc cancels)", theme.dim()),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner,
    );
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
    use cohors_core::{
        Branch, CiStatus, CommitMeta, RemoteInfo, RepoId, RepoSnapshot, Upstream, WorktreeStatus,
    };
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
        // A few repos carry GitHub remote info so the Remote cloud is exercised
        // in passing/failing/pending states; the rest stay local ("—").
        let remote = |open_prs: u32, ci: CiStatus| {
            Some(RemoteInfo {
                host: "github.com".to_string(),
                owner: "demo".to_string(),
                repo: "demo".to_string(),
                default_branch: "main".to_string(),
                open_prs,
                prs_awaiting_review: 0,
                ci,
            })
        };
        let mut payments = snap(
            "payments",
            Branch::Named("main".into()),
            Some(("origin/main", 2, 0)),
            (0, 3, 1),
            1,
            Some((NOW - 7200, "fix: retry on 5xx")),
            None,
        );
        payments.remote = remote(2, CiStatus::Passing);
        let mut web_app = snap(
            "web-app",
            Branch::Named("feat/checkout".into()),
            Some(("origin/feat", 0, 5)),
            (0, 7, 0),
            0,
            Some((NOW - 1200, "wip: cart drawer")),
            None,
        );
        web_app.remote = remote(0, CiStatus::Failing);
        let mut auth_service = snap(
            "auth-service",
            Branch::Named("main".into()),
            None,
            (0, 0, 0),
            0,
            Some((NOW - 259_200, "chore: bump deps")),
            None,
        );
        auth_service.remote = remote(1, CiStatus::Pending);
        app.set_repos(vec![
            payments,
            web_app,
            auth_service,
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
        // Normalize the version so a bump doesn't churn every header snapshot —
        // these tests assert layout, not the version number.
        out.replace(&format!("v{}", env!("CARGO_PKG_VERSION")), "vX.Y.Z")
    }

    #[test]
    fn snapshot_list() {
        let app = demo_app();
        insta::assert_snapshot!(render_to_string(&app, 100, 20));
    }

    /// The `cohors demo` fleet renders end-to-end (validates the demo data path).
    #[test]
    fn snapshot_demo_fleet() {
        let mut app = App::new(vec!["(demo)".to_string()], "(demo)".to_string());
        app.set_repos(cohors_core::demo::fleet(NOW));
        insta::assert_snapshot!(render_to_string(&app, 100, 26));
    }

    #[test]
    fn snapshot_multiselect() {
        let mut app = demo_app();
        app.selection.insert(RepoId("payments".to_string()));
        app.selection.insert(RepoId("web-app".to_string()));
        insta::assert_snapshot!(render_to_string(&app, 100, 20));
    }

    #[test]
    fn snapshot_confirm_stash() {
        use crate::app::{ConfirmAction, Pending};
        let mut app = demo_app();
        app.selection.insert(RepoId("payments".to_string()));
        app.selection.insert(RepoId("web-app".to_string()));
        app.mode = Mode::Confirm;
        app.confirm = Some(Pending {
            prompt: "Stash changes in 2 repos?".to_string(),
            action: ConfirmAction::BulkStash(vec![
                RepoId("payments".to_string()),
                RepoId("web-app".to_string()),
            ]),
        });
        insta::assert_snapshot!(render_to_string(&app, 100, 20));
    }

    #[test]
    fn snapshot_command_input() {
        let mut app = demo_app();
        app.mode = Mode::CommandInput;
        app.command_input = "git fetch --all".to_string();
        insta::assert_snapshot!(render_to_string(&app, 100, 20));
    }

    #[test]
    fn snapshot_command_run_running() {
        use crate::app::{CommandRun, RunResult};
        let mut app = demo_app();
        app.mode = Mode::CommandRun;
        app.run = Some(CommandRun::new(
            1,
            "git status -s".to_string(),
            vec![
                RunResult {
                    id: RepoId("payments".to_string()),
                    name: "payments".to_string(),
                    state: RunState::Done {
                        code: 0,
                        stdout: " M src/lib.rs\n?? notes.txt\n".to_string(),
                        stderr: String::new(),
                    },
                },
                RunResult {
                    id: RepoId("web-app".to_string()),
                    name: "web-app".to_string(),
                    state: RunState::Running,
                },
                RunResult {
                    id: RepoId("auth-service".to_string()),
                    name: "auth-service".to_string(),
                    state: RunState::Running,
                },
            ],
        ));
        insta::assert_snapshot!(render_to_string(&app, 100, 22));
    }

    #[test]
    fn snapshot_command_run_finished() {
        use crate::app::{CommandRun, RunResult};
        let mut app = demo_app();
        app.mode = Mode::CommandRun;
        let mut run = CommandRun::new(
            2,
            "cargo test".to_string(),
            vec![
                RunResult {
                    id: RepoId("payments".to_string()),
                    name: "payments".to_string(),
                    state: RunState::Done {
                        code: 0,
                        stdout: "test result: ok. 12 passed\n".to_string(),
                        stderr: String::new(),
                    },
                },
                RunResult {
                    id: RepoId("web-app".to_string()),
                    name: "web-app".to_string(),
                    state: RunState::Done {
                        code: 1,
                        stdout: "running 3 tests\n".to_string(),
                        stderr: "test cart::total failed\nerror: 1 test failed".to_string(),
                    },
                },
            ],
        );
        run.focus = 1; // the failing repo, so its stderr shows
        app.run = Some(run);
        insta::assert_snapshot!(render_to_string(&app, 100, 22));
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
        insta::assert_snapshot!(render_to_string(&app, 100, 40));
    }

    #[test]
    fn snapshot_standup() {
        use crate::app::StandupView;
        use cohors_core::{StandupCommit, StandupWindow};
        let mut app = demo_app();
        app.mode = Mode::Standup;
        app.standup_window = StandupWindow::Week;
        // payments (most active) sorts first; its commits overflow the pane,
        // so the scrollbar renders.
        let mut commits = Vec::new();
        for i in 0..24 {
            commits.push(StandupCommit {
                repo: "payments".into(),
                short_id: format!("aa{i:05}"),
                summary: format!("feat: payments work item {i}"),
                timestamp: NOW - (i as i64) * 3600,
            });
        }
        for i in 0..5 {
            commits.push(StandupCommit {
                repo: "web-app".into(),
                short_id: format!("bb{i:05}"),
                summary: format!("fix: web-app bug {i}"),
                timestamp: NOW - (i as i64) * 7200,
            });
        }
        app.standup = Some(StandupView::new(commits));
        insta::assert_snapshot!(render_to_string(&app, 100, 22));
    }

    /// On a narrow terminal the standup's two panes stack (Repos box on top,
    /// commits box below) instead of sitting side-by-side.
    #[test]
    fn snapshot_standup_narrow_stacked() {
        use crate::app::StandupView;
        use cohors_core::{StandupCommit, StandupWindow};
        let mut app = demo_app();
        app.mode = Mode::Standup;
        app.standup_window = StandupWindow::Week;
        let mut commits = Vec::new();
        for i in 0..8 {
            commits.push(StandupCommit {
                repo: "payments".into(),
                short_id: format!("aa{i:05}"),
                summary: format!("feat: payments work item {i}"),
                timestamp: NOW - (i as i64) * 3600,
            });
        }
        for i in 0..3 {
            commits.push(StandupCommit {
                repo: "web-app".into(),
                short_id: format!("bb{i:05}"),
                summary: format!("fix: web-app bug {i}"),
                timestamp: NOW - (i as i64) * 7200,
            });
        }
        app.standup = Some(StandupView::new(commits));
        insta::assert_snapshot!(render_to_string(&app, 50, 24));
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
