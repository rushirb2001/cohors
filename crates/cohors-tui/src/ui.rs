//! Rendering. Pure view-model → widgets: every frame derives the visible rows
//! from [`App::view`] (i.e. `cohors-core`) and maps them onto ratatui widgets.
//! No state is mutated here.

use cohors_core::{
    AttentionReason, Branch, CiStatus, RepoSnapshot, Severity, StandupCommit, assess,
    fleet_summary, time,
};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, List, ListItem, ListState, Padding, Paragraph, Row,
    Scrollbar, ScrollbarOrientation, ScrollbarState, Table, TableState, Wrap,
};

use crate::app::{App, ConfirmAction, Mode, Opener, RunState};
use crate::glyphs::Glyphs;

/// Spinner frames (braille) for the scan indicator.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn spinner_frame(tick: usize) -> &'static str {
    SPINNER[tick % SPINNER.len()]
}

/// Color policy. Colors are dropped when `NO_COLOR` is set; structural
/// modifiers (dim/bold/reversed) are kept so the layout still reads. Also carries
/// the resolved [`Glyphs`] so every cell renders glyphs through one source of
/// truth (with ASCII fallback under `NO_COLOR`).
struct Theme {
    color: bool,
    glyphs: Glyphs,
}

impl Theme {
    fn from_env(icons: cohors_config::IconMode) -> Self {
        let color = std::env::var_os("NO_COLOR").is_none();
        Self {
            color,
            glyphs: Glyphs::resolve(icons, !color),
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
    let theme = Theme::from_env(app.icons);
    let area = frame.area();

    // Top-level layout: the branded header box, the body, and a boxed key-hint
    // footer — one labelled group per row, each wrapping if the terminal is
    // narrow, so no command is ever truncated.
    let footer_h = footer_height(app, area.width, &theme);
    let [header_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Min(0),
        Constraint::Length(footer_h),
    ])
    .areas(area);
    render_header(frame, header_area, app, now, &theme);
    render_footer(frame, footer_area, app, &theme);

    if app.repos.is_empty() {
        if app.scanning {
            render_loading(frame, body_area, app);
        } else {
            render_empty(frame, body_area, app);
        }
    } else {
        // A strip on top — the fuzzy input while filtering, otherwise the
        // Attention panel — then the Repositories panel, which hosts the context
        // pane inside its own box (bottom rows) when the terminal is tall enough.
        let strip_height = if app.mode == Mode::Filter { 1 } else { 3 };
        let [strip, rest] =
            Layout::vertical([Constraint::Length(strip_height), Constraint::Min(0)])
                .areas(body_area);
        if app.mode == Mode::Filter {
            render_filter_input(frame, strip, app);
        } else {
            render_attention_panel(frame, strip, app, now, &theme);
        }
        render_repos_panel(frame, rest, app, now, &theme);
    }

    // Dim the whole frame behind a modal overlay so the background recedes and
    // the overlay stands out. The overlays `Clear` their own area, so they
    // render crisp on top of the dimmed background.
    let overlay_open = matches!(
        app.mode,
        Mode::Help
            | Mode::Standup
            | Mode::Command
            | Mode::CommandRun
            | Mode::Confirm
            | Mode::OpenWith
            | Mode::Detail
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
    if app.mode == Mode::Command {
        render_command_box(frame, area, app, &theme);
    }
    if app.mode == Mode::CommandRun {
        render_command_run(frame, area, app, &theme);
    }
    if app.mode == Mode::Confirm {
        render_confirm(frame, area, app, &theme);
    }
    if app.mode == Mode::OpenWith {
        render_open_with(frame, area, app, &theme);
    }
    if app.mode == Mode::Detail {
        render_detail(frame, area, app, now, &theme);
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

/// The branded header: a logo lockup — a shield mark built from block glyphs
/// beside the wordmark, version, and tagline. cohors's "business card."
/// Transient feedback lives in a self-dismissing toast, not here.
/// The brand purple for the spider mark (a true violet, not the terminal's
/// pinkish ANSI magenta). Dropped under `NO_COLOR` via [`Theme::fg`].
const SPIDER_PURPLE: Color = Color::Rgb(0xA8, 0x55, 0xF7);

fn render_header(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.dim())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // The header mark, drawn as 3 rows of block "pixels". The purple is dropped
    // under NO_COLOR, but the silhouette still reads. A space ` ` is a
    // transparent gap (use it for eyes / holes).
    //
    // ── Glyph palette ───────────────────────────────────────────────────────
    // Full / half blocks (solid fills):   █  ▀  ▄  ▌  ▐
    // Shades (lighter fills):             ░  ▒  ▓
    // Quadrants — one corner:             ▖(BL) ▗(BR) ▘(TL) ▝(TR)
    // Quadrants — three corners (notch):  ▙(no TR) ▟(no TL) ▛(no BR) ▜(no BL)
    // Quadrants — diagonals:              ▚(TL+BR) ▞(TR+BL)
    // Geometric (centers / eyes):         ● ○ ◉ ◆ ■ ▮ ▬ ◐ ◑
    //
    // ── Designs to try ──────────────────────────────────────────────────────
    // Swap the three rows of `spider` below for a set here, and set `ICON_W`
    // (just below) to the noted width. Keep all three rows the same length (pad
    // with spaces) so the columns line up.
    //
    //   V0 · droid — width 9
    //       "█ ▟███▙ █"   /   "███ █ ███"   /   "█ ▜███▛ █"
    //
    //   V1 · diagonal legs — width 9
    //       "▀▄     ▄▀"   /   "   ▟█▙   "   /   "▄▀     ▀▄"
    //
    //   V2 · eight quadrant legs — width 9
    //       "▖▗     ▖▗"   /   "   ▝█▘   "   /   "▘▝     ▘▝"
    //
    //   V3 · corner-leg crab — width 9
    //       "▟▙     ▟▙"   /   "  █████  "   /   "▜▛     ▜▛"
    //
    //   V4 · wide diagonal splay — width 11
    //       "▀▄       ▄▀" /   "    ▟█▙    " /   "▄▀       ▀▄"
    //
    //   V5 · octopus: two eyes + feet — width 7
    //       "▟█████▙"     /   "██ █ ██"     /   " ▘▘ ▘▘ "
    //
    //   V6 · bristled body, angled legs — width 9
    //       "▝▙     ▟▘"   /   "   ███   "   /   "▗▛     ▜▖"
    //
    //   V7 · shaded body (soft fill) — width 9
    //       "▀▄     ▄▀"   /   "   ▒█▒   "   /   "▄▀     ▀▄"
    // ────────────────────────────────────────────────────────────────────────
    let mark = theme.fg(SPIDER_PURPLE).add_modifier(Modifier::BOLD);
    const ICON_W: u16 = 9;
    let spider = Text::from(vec![
        Line::from(Span::styled("▜▒▟███▙▒▛", mark)),
        Line::from(Span::styled("▟██▌█▐██▙", mark)),
        Line::from(Span::styled("▀▐▖▀█▀▗▌▀", mark)),
    ]);

    // The wordmark + version — the lede, shared by the full and compact layouts.
    let lede = Line::from(vec![
        Span::styled(
            "cohors",
            theme.fg(SPIDER_PURPLE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  v{}", env!("CARGO_PKG_VERSION")), theme.dim()),
    ]);

    // Full brand block (lede + taglines) and the info column, both sized to
    // their content so the bar packs left instead of leaving a gap mid-line.
    let brand = Text::from(vec![
        lede.clone(),
        Line::from(Span::styled(
            "All your git repositories at a glance",
            theme.dim(),
        )),
        Line::from(Span::styled(
            "status · fetch · pull · weekly standup",
            theme.dim(),
        )),
    ]);
    let info = header_info(app, now, 34, theme);
    let text_w = brand.width() as u16;
    let info_w = info.width() as u16;
    let full_need = ICON_W + 2 + text_w + 2 + 1 + 2 + info_w;

    // Compact fallback for narrow terminals: spider + lede, then just the
    // directory — no taglines, divider, or info column.
    if inner.width < full_need {
        let [icon_area, text_area] =
            Layout::horizontal([Constraint::Length(ICON_W), Constraint::Min(0)])
                .spacing(2)
                .areas(inner);
        frame.render_widget(Paragraph::new(spider), icon_area);
        let dir = truncate_tail(&header_dir(app), text_area.width as usize);
        frame.render_widget(
            Paragraph::new(Text::from(vec![
                lede,
                Line::from(Span::styled(dir, theme.dim())),
            ])),
            text_area,
        );
        return;
    }

    // Full layout, packed left: spider · brand · divider · info · (spacer).
    let [icon_area, text_area, div_area, info_area, _rest] = Layout::horizontal([
        Constraint::Length(ICON_W),
        Constraint::Length(text_w),
        Constraint::Length(1),
        Constraint::Length(info_w),
        Constraint::Min(0),
    ])
    .spacing(2)
    .areas(inner);

    frame.render_widget(Paragraph::new(spider), icon_area);
    frame.render_widget(Paragraph::new(brand), text_area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(Span::styled("│", theme.dim())),
            Line::from(Span::styled("│", theme.dim())),
            Line::from(Span::styled("│", theme.dim())),
        ])),
        div_area,
    );
    frame.render_widget(Paragraph::new(info), info_area);
}

/// The configured root(s) as a compact, home-abbreviated string.
fn header_dir(app: &App) -> String {
    match app.roots.as_slice() {
        [] => "(no roots)".to_string(),
        [one] => abbrev_home(one),
        [first, rest @ ..] => format!("{} (+{})", abbrev_home(first), rest.len()),
    }
}

/// The header's right-hand info column: session orientation that isn't shown
/// elsewhere — where we're watching, the config in effect, and the fleet's most
/// recent activity. Three aligned `label value` rows trimmed to `width`.
fn header_info(app: &App, now: i64, width: u16, theme: &Theme) -> Text<'static> {
    let val = width.saturating_sub(7) as usize;

    // Where — the configured root(s), home-abbreviated and tail-trimmed.
    let dir = truncate_tail(&header_dir(app), val);

    // Which config is in effect.
    let config = truncate_tail(&abbrev_home(&app.config_path), val);

    // The fleet's pulse — the single most recent commit across every repo.
    let latest = app.repos.iter().filter_map(|r| r.last_commit_time()).max();
    let active = match latest {
        Some(ts) => time::relative(ts, now),
        None => "—".to_string(),
    };

    let label = |t: &'static str| Span::styled(format!("{t:<6} "), theme.dim());
    Text::from(vec![
        Line::from(vec![label("dir"), Span::styled(dir, Style::new())]),
        Line::from(vec![label("config"), Span::styled(config, theme.dim())]),
        Line::from(vec![label("active"), Span::styled(active, Style::new())]),
    ])
}

/// Replace a leading `$HOME` with `~` for a compact path display.
fn abbrev_home(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

/// Trim `s` to at most `max` columns, keeping the tail (most specific part of a
/// path) and prefixing an ellipsis when truncated.
fn truncate_tail(s: &str, max: usize) -> String {
    let len = s.chars().count();
    if max == 0 || len <= max {
        return s.to_string();
    }
    let tail: String = s.chars().skip(len - max.saturating_sub(1)).collect();
    format!("…{tail}")
}

/// A hint's rendered width (`key` + space + `desc`).
fn footer_item_w(key: &str, desc: &str) -> usize {
    key.chars().count() + 1 + desc.chars().count()
}

/// Hints at or below this width pack into the two-column grid; wider ones
/// ("run command", "mark repo"…) drop to full-width rows below the grid, so the
/// columns stay narrow enough to always fit two-up — even on a compact terminal.
const FOOTER_GRID_MAX: usize = 10;

/// One footer hint: `(key, description)`.
type Hint<'a> = &'a (&'a str, &'a str);

/// Split a group's hints into the two-column "short" set and the full-width
/// "long" set, preserving order.
fn footer_partition<'a>(items: &'a [(&'a str, &'a str)]) -> (Vec<Hint<'a>>, Vec<Hint<'a>>) {
    items
        .iter()
        .partition(|(k, d)| footer_item_w(k, d) <= FOOTER_GRID_MAX)
}

/// A group box's content height: the in-box title row + its divider rule, the
/// two-column grid rows, the full-width long rows, and a 1-row horizontal divider
/// between them when both are present.
fn footer_group_rows(items: &[(&str, &str)]) -> u16 {
    let (short, long) = footer_partition(items);
    let hr = if !short.is_empty() && !long.is_empty() {
        1
    } else {
        0
    };
    (2 + short.len().div_ceil(2) + hr + long.len()) as u16
}

/// The footer's total height (including borders). In Normal mode it's the three
/// group boxes, sized to the tallest after responsive column collapse; in the
/// overlay modes it's a single box of wrapped hint rows.
fn footer_height(app: &App, width: u16, theme: &Theme) -> u16 {
    if app.mode == Mode::Normal {
        // The hint bar is collapsed to just its toggle divider.
        if app.hints_hidden {
            return 1;
        }
        let rows = footer_groups(app)
            .iter()
            .map(|(_, items)| footer_group_rows(items))
            .max()
            .unwrap_or(1);
        rows + 2 + 1 // group-box border + the toggle divider row
    } else {
        let lines = footer_lines(app, theme);
        let inner = width.saturating_sub(4).max(1);
        let rows: u16 = lines
            .iter()
            .map(|l| (l.width() as u16).max(1).div_ceil(inner))
            .sum::<u16>()
            .clamp(1, 6);
        rows + 2
    }
}

/// The footer: context-sensitive key hints. In Normal mode it's three titled
/// group boxes (`select`/`act`/`view`) side by side — no outer box — that adapt
/// to the width; in the overlay modes it's a single box of hint rows.
fn render_footer(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if app.mode == Mode::Normal {
        // A divider that doubles as the show/hide affordance, then the boxes
        // (unless the user has collapsed them with `h`).
        let [divider, boxes] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
        render_hint_divider(frame, divider, app.hints_hidden, theme);
        if !app.hints_hidden {
            render_footer_boxed(frame, boxes, app, theme);
        }
    } else {
        render_footer_simple(frame, area, footer_lines(app, theme), theme);
    }
}

/// A full-width divider with centered text advertising the `h` toggle:
/// "── press h to hide hints ──" (or "unhide" when the boxes are collapsed).
fn render_hint_divider(frame: &mut Frame, area: Rect, hidden: bool, theme: &Theme) {
    let verb = if hidden { "unhide" } else { "hide" };
    let mid = vec![
        Span::styled("press ", theme.dim()),
        Span::styled("h", theme.ahead().add_modifier(Modifier::BOLD)),
        Span::styled(format!(" to {verb} hints"), theme.dim()),
    ];
    let text_w: usize = mid.iter().map(|s| s.content.chars().count()).sum();
    let total = area.width as usize;
    let dashes = total.saturating_sub(text_w + 2); // a space on each side of the text
    let left = dashes / 2;
    let right = dashes - left;

    let mut spans = vec![Span::styled("─".repeat(left), theme.dim()), Span::raw(" ")];
    spans.extend(mid);
    spans.push(Span::raw(" "));
    spans.push(Span::styled("─".repeat(right), theme.dim()));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_footer_simple(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>, theme: &Theme) {
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

/// Three titled group boxes side by side, directly in the footer area (no outer
/// box, to save space).
fn render_footer_boxed(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let groups = footer_groups(app);
    let cols = Layout::horizontal(vec![Constraint::Fill(1); groups.len()])
        .spacing(1)
        .split(area);
    for (i, (label, items)) in groups.iter().enumerate() {
        render_group_box(frame, cols[i], label, items, app, theme);
    }
}

/// A single titled group box. Short hints fill a two-column grid split by a thin
/// `│` divider; multi-word hints stack full-width below it. No inner padding, so
/// the two columns stay two-up even on a compact terminal. The `act` box carries
/// the live action-target on its bottom edge.
fn render_group_box(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    items: &[(&'static str, &'static str)],
    app: &App,
    theme: &Theme,
) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.dim())
        .padding(Padding::horizontal(1));
    let outer_inner = block.inner(area);
    frame.render_widget(block, area);

    // The group title lives *inside* the box now: a centered header row with a
    // divider rule beneath it, not on the border.
    let [title_area, rule_area, inner] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(outer_inner);
    // The "act" box shows its live action-target beside the title — the whole
    // "act  → <repo>" unit is centered (not on the bottom border).
    let mut title_spans = vec![Span::styled(
        label.to_string(),
        theme.dim().add_modifier(Modifier::BOLD),
    )];
    if label == "act"
        && let Some((_, target)) = action_target_hint(app)
    {
        title_spans.push(Span::raw("  "));
        title_spans.push(Span::styled(format!("→ {target}"), theme.highlight()));
    }
    frame.render_widget(
        Paragraph::new(Line::from(title_spans)).alignment(Alignment::Center),
        title_area,
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(rule_area.width as usize),
            theme.dim(),
        ))),
        rule_area,
    );

    let key_style = theme.ahead().add_modifier(Modifier::BOLD);
    let hint = |key: &str, desc: &str| {
        Line::from(vec![
            Span::styled(key.to_string(), key_style),
            Span::styled(format!(" {desc}"), theme.dim()),
        ])
    };

    let (short, long) = footer_partition(items);
    let grid_rows = short.len().div_ceil(2) as u16;
    // A horizontal rule separates the grid from the full-width rows when both
    // are present.
    let need_hr = !short.is_empty() && !long.is_empty();
    let chunks = if need_hr {
        Layout::vertical([
            Constraint::Length(grid_rows),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner)
    } else {
        Layout::vertical([Constraint::Length(grid_rows), Constraint::Min(0)]).split(inner)
    };
    let grid_area = chunks[0];
    let (hr_area, long_area) = if need_hr {
        (Some(chunks[1]), chunks[2])
    } else {
        (None, chunks[1])
    };

    // Two columns with a 1-col `│` divider between them.
    if !short.is_empty() {
        let [c1, divider, c2] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .spacing(1)
        .areas(grid_area);
        let half = short.len().div_ceil(2);
        let col = |chunk: &[&(&str, &str)]| {
            Text::from(chunk.iter().map(|(k, d)| hint(k, d)).collect::<Vec<_>>())
        };
        frame.render_widget(Paragraph::new(col(&short[..half])), c1);
        frame.render_widget(Paragraph::new(col(&short[half..])), c2);
        let bar: Vec<Line> = (0..grid_rows)
            .map(|_| Line::from(Span::styled("│", theme.dim())))
            .collect();
        frame.render_widget(Paragraph::new(Text::from(bar)), divider);
    }

    // The horizontal rule, edge to edge (touches both borders since there's no
    // inner padding).
    if let Some(hr) = hr_area {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(hr.width as usize),
                theme.dim(),
            ))),
            hr,
        );
    }

    // Multi-word hints, full-width below the grid.
    if !long.is_empty() {
        let lines: Vec<Line> = long.iter().map(|(k, d)| hint(k, d)).collect();
        frame.render_widget(Paragraph::new(Text::from(lines)), long_area);
    }
}

/// The "Attention" panel: a titled box summarizing what needs the user, in
/// plain words ("3 dirty · 1 behind") rather than terse glyphs.
fn render_attention_panel(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
    let s = fleet_summary(&app.repos, now);

    // The "N of M repositories" count lives in the box title; the body is just
    // the category pills (or an all-clear message).
    let mut title = vec![Span::styled(
        " Attention ",
        Style::new().add_modifier(Modifier::BOLD),
    )];
    if s.needs_attention > 0 {
        title.push(Span::styled(
            format!("({} of {} repositories) ", s.needs_attention, s.total),
            theme.dim(),
        ));
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(title))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let line = if s.needs_attention == 0 {
        Line::from(Span::styled(
            format!("All {} repositories are up to date.", s.total),
            theme.ok(),
        ))
    } else {
        let mut spans: Vec<Span> = Vec::new();

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

        for (i, (text, style)) in items.into_iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" · ", theme.dim()));
            }
            spans.push(Span::styled(text, style));
        }
        Line::from(spans)
    };

    frame.render_widget(Paragraph::new(line), inner);
}

/// The self-dismissing toast line — scan progress or the latest action result
/// (fetch/pull/push/stash/copy…) — shown on the **top-right of the Repositories
/// box border**, not crowding the header. `None` when there's nothing to show.
/// In-progress messages stay until the work finishes; results clear after a few
/// seconds (the event loop owns the timer).
fn toast_line(app: &App, theme: &Theme) -> Option<Line<'static>> {
    if app.scanning {
        let msg = app
            .status
            .clone()
            .unwrap_or_else(|| "scanning…".to_string());
        return Some(Line::from(Span::styled(
            format!(" {} {msg} ", spinner_frame(app.spinner)),
            theme.dim(),
        )));
    }
    let msg = app.status.as_ref()?;
    // A failure reads red; everything else is a confirmation (green + ✓).
    let failed = msg.contains("fail")
        || msg.contains("error")
        || msg.contains("reject")
        || msg.starts_with("no ");
    let (icon, style) = if failed {
        ("✗", theme.risk())
    } else {
        ("✓", theme.ok().add_modifier(Modifier::BOLD))
    };
    Some(Line::from(Span::styled(format!(" {icon} {msg} "), style)))
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
        Mode::Command => vec![("", vec![("⏎", "run"), ("Esc", "cancel")])],
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
        Mode::Detail => vec![(
            "",
            vec![("↑/↓", "scroll"), ("g/G", "top / bottom"), ("Esc", "close")],
        )],
        Mode::OpenWith => vec![(
            "",
            vec![
                ("↑/↓", "choose"),
                ("⏎", "open"),
                ("d", "set default"),
                ("Esc", "cancel"),
            ],
        )],
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
                    ("P", "push"),
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

/// A centered modal of fixed width (% of the frame) and height, for the command
/// overlays.
fn centered_modal(full: Rect, pct_w: u16, h: u16) -> Rect {
    let w = ((full.width as u32 * pct_w as u32 / 100) as u16).clamp(24, full.width);
    let h = h.min(full.height.saturating_sub(2)).max(3);
    Rect {
        x: full.x + full.width.saturating_sub(w) / 2,
        y: full.y + full.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

/// The `:`-command palette: a centered boxed overlay (mirroring the `!` runner)
/// with the `:` prompt and a structured, colour-coded cheat sheet of the verbs.
fn render_command_box(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    // Shell mode (opened with `!`): the accent shifts to a warning colour and the
    // sheet collapses to just the highlighted shell row.
    let shell = app.command_line.starts_with('!');
    let accent = if shell { theme.warn() } else { theme.ahead() };
    let cmd = |t: &str| Span::styled(t.to_string(), accent.add_modifier(Modifier::BOLD));
    let dim = |t: &str| Span::styled(t.to_string(), theme.dim());

    // (command spans, description) — a structured two-column cheat sheet.
    let rows: Vec<(Vec<Span<'static>>, &str)> = if shell {
        vec![(
            vec![
                Span::styled("▌ ", accent.add_modifier(Modifier::BOLD)),
                cmd(":!"),
                Span::styled("<cmd>", accent),
            ],
            "run any shell command across the targets",
        )]
    } else {
        vec![
            (
                vec![
                    cmd(":fetch"),
                    dim(" "),
                    cmd(":pull"),
                    dim(" "),
                    cmd(":push"),
                ],
                "sync the targets with their remotes",
            ),
            (
                vec![cmd(":!"), dim("<cmd>")],
                "run any shell command across the targets",
            ),
            (
                vec![cmd(":sort"), dim(" <name·dirty·recent>")],
                "change the sort order",
            ),
            (
                vec![cmd(":dirty"), dim("   "), cmd("/<text>")],
                "dirty-only · fuzzy filter",
            ),
            (
                vec![cmd(":jump"), dim(" <repo>")],
                "move the cursor to a repo",
            ),
            (
                vec![
                    cmd(":standup"),
                    dim(" "),
                    cmd(":refresh"),
                    dim(" "),
                    cmd(":quit"),
                ],
                "standup · rescan · exit",
            ),
        ]
    };
    let desc_style = if shell { accent } else { theme.dim() };

    let area = centered_modal(full, 74, rows.len() as u16 + 4);
    frame.render_widget(Clear, area);
    let title = if shell {
        " Shell command "
    } else {
        " Command "
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if shell { accent } else { Style::new() })
        .title(Line::from(title).bold())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Prompt · horizontal divider · the two-column grid.
    let [prompt_area, hr_area, grid_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(": ", accent.add_modifier(Modifier::BOLD)),
            Span::styled(
                app.command_line.clone(),
                if shell { accent } else { Style::new() },
            ),
            Span::raw("▏"),
        ])),
        prompt_area,
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(hr_area.width as usize),
            theme.dim(),
        ))),
        hr_area,
    );

    // Commands │ descriptions, with a vertical divider between the columns.
    let [cmd_col, div_col, desc_col] = Layout::horizontal([
        Constraint::Length(26),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .spacing(1)
    .areas(grid_area);
    let cmd_lines: Vec<Line> = rows.iter().map(|(s, _)| Line::from(s.clone())).collect();
    let div_lines: Vec<Line> = rows
        .iter()
        .map(|_| Line::from(Span::styled("│", theme.dim())))
        .collect();
    let desc_lines: Vec<Line> = rows
        .iter()
        .map(|(_, d)| Line::from(Span::styled(d.to_string(), desc_style)))
        .collect();
    frame.render_widget(Paragraph::new(Text::from(cmd_lines)), cmd_col);
    frame.render_widget(Paragraph::new(Text::from(div_lines)), div_col);
    frame.render_widget(Paragraph::new(Text::from(desc_lines)), desc_col);
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
    // First-run rescue: we found repos elsewhere — offer to adopt them in one key.
    let text = if app.empty_picker_active() {
        Text::from(vec![
            Line::from(format!("No git repos under: {}", roots_label(app))),
            Line::from(""),
            Line::from(format!(
                "Found repos under: {}",
                app.suggested_roots.join(", ")
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("[u]", Style::new().add_modifier(Modifier::BOLD)),
                Span::raw(" use these  ·  "),
                Span::styled("[q]", Style::new().add_modifier(Modifier::BOLD)),
                Span::raw(" quit"),
            ]),
        ])
    } else {
        Text::from(vec![
            Line::from(format!("No git repos found under: {}", roots_label(app))),
            Line::from(""),
            Line::from("Run `cohors init` — it auto-detects your repos — or pass --root <dir>."),
        ])
    };
    let height = text.lines.len() as u16;
    let para = Paragraph::new(text).alignment(Alignment::Center);
    frame.render_widget(para, center_v(area, height));
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
/// The scroll offset that keeps `selected` visible in a `viewport`-tall window,
/// nudging from `prev` only as far as needed, then clamping to the end.
fn keep_visible(prev: usize, selected: usize, viewport: usize, total: usize) -> usize {
    let mut off = prev;
    if selected < off {
        off = selected;
    } else if viewport > 0 && selected >= off + viewport {
        off = selected + 1 - viewport;
    }
    off.min(total.saturating_sub(viewport))
}

fn render_repos_panel(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
    // The list's view state lives in its own title: the (visible) count in bold,
    // then the sort mode (and the dirty-only filter, when on) in dim.
    let mut title = vec![Span::styled(
        format!(" Repositories ({}) ", app.visible_len()),
        Style::new().add_modifier(Modifier::BOLD),
    )];
    title.push(Span::styled(
        format!("· sort: {} ", app.sort.label()),
        theme.dim(),
    ));
    if app.dirty_only {
        title.push(Span::styled("· dirty-only ", theme.dim()));
    }
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(title));
    // Transient feedback (scan progress, action results) rides the top-right of
    // this box's border — a self-dismissing toast that doesn't crowd the header.
    if let Some(toast) = toast_line(app, theme) {
        block = block.title(toast.right_aligned());
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // The context pane lives *inside* this box. When the panel is tall enough,
    // reserve its bottom rows (a divider rule + a blank pad + the content) for the
    // selected repo's facts and lay the table out in what's left; on short panels
    // it doesn't appear and the table uses the whole inner area. `dock_visible`
    // also drives the table's expanded-column layout.
    let dock = build_dock(app, now, theme, inner.width.saturating_sub(2) as usize);
    let dock_reserve = match &dock {
        // divider(1) + pad(1) + content, only if the table keeps ≥6 rows.
        Some(d) if inner.height >= d.lines.len() as u16 + 2 + 6 => d.lines.len() as u16 + 2,
        _ => 0,
    };
    let dock_visible = dock_reserve > 0;
    let [table_inner, dock_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(dock_reserve)]).areas(inner);

    let view = app.view();
    let total = view.len();
    // The header is its own row (no blank margin); the data fills the rest. When
    // the fleet overflows, a scroll hint is reserved above and/or below the data —
    // both at once when repos are hidden in both directions.
    let avail = table_inner.height.saturating_sub(1) as usize; // data rows with header only
    let overflow = total > avail;

    // First assume one hint row; if that still hides repos both above and below,
    // two hint rows are needed (shrinking the viewport by one more).
    let one_vp = avail.saturating_sub(usize::from(overflow)).max(1);
    let one_off = keep_visible(app.repos_scroll.get(), app.selected, one_vp, total);
    let two_hints = overflow && one_off > 0 && total.saturating_sub(one_off + one_vp) > 0;
    let viewport = if two_hints {
        avail.saturating_sub(2).max(1)
    } else {
        one_vp
    };
    let offset = if two_hints {
        keep_visible(app.repos_scroll.get(), app.selected, viewport, total)
    } else {
        one_off
    };
    app.repos_scroll.set(offset);
    let above = offset;
    let below = total.saturating_sub(offset + viewport);

    // header row, then the data, with `↑ N more` below the header when repos are
    // hidden above and `… N more ↓` at the bottom when hidden below — either or
    // both.
    let len1 = || Constraint::Length(1);
    let (header_area, data_area, top_hint, bot_hint) = if !overflow {
        let [hd, dt] = Layout::vertical([len1(), Constraint::Min(0)]).areas(table_inner);
        (hd, dt, None, None)
    } else if above == 0 {
        let [hd, dt, bn] =
            Layout::vertical([len1(), Constraint::Min(0), len1()]).areas(table_inner);
        (hd, dt, None, Some(bn))
    } else if below == 0 {
        let [hd, tn, dt] =
            Layout::vertical([len1(), len1(), Constraint::Min(0)]).areas(table_inner);
        (hd, dt, Some(tn), None)
    } else {
        let [hd, tn, dt, bn] =
            Layout::vertical([len1(), len1(), Constraint::Min(0), len1()]).areas(table_inner);
        (hd, dt, Some(tn), Some(bn))
    };

    // Size each data column to its actual content so it stays tight (a fleet with
    // no PRs gets a narrow PRs column, etc.). The floor is the header-label width;
    // the ceiling guards against one outlier stretching a column. `line_width`
    // measures the same spans the cell will render.
    let col_w = |build: fn(&RepoSnapshot, &Theme) -> Vec<Span<'static>>, floor: u16, ceil: u16| {
        view.iter()
            .map(|vr| line_width(&build(&app.repos[vr.index], theme)))
            .max()
            .unwrap_or(0)
            .clamp(floor, ceil)
    };

    // The commit age column, present only in the dock layout.
    let age_w: u16 = if dock_visible {
        view.iter()
            .map(|vr| match &app.repos[vr.index].last_commit {
                Some(c) => time::relative(c.timestamp, now).chars().count() as u16,
                None => 1,
            })
            .max()
            .unwrap_or(0)
            .clamp(4, 6)
    } else {
        0
    };

    // Two layouts. With the dock up it carries the selected repo's commit
    // message, so the table drops the message and spends that width on legible,
    // expanded columns: the old fused Sync (`↑2 ● 2pr`) and Changes (`4 s1`)
    // split into Sync / PRs / CI and Changes / Stash, plus Last (age) and a
    // Status column (the primary attention reason, which makes the dirty-first
    // ordering self-explaining per row). With the dock hidden (short terminal)
    // the table keeps the compact, fused columns and the full "Last commit", so
    // nothing is lost. The trailing column is Fill so the table spans edge to
    // edge; its text is ellipsized to that width (computed by mirroring the
    // table's own arithmetic: inner width − the 2-col highlight reserve − the
    // fixed columns − the 2-col gaps between them).
    // Repo and Branch are fused into one `name @branch` column, sized to its
    // widest content (gutter + name + branch) within a cap so a long branch can't
    // run away. This mirrors the web and reclaims a whole column for Status.
    let repo_w = col_w(repo_spans, 12, 30);
    let (widths, reason_max, summary_max): (Vec<Constraint>, usize, usize) = if dock_visible {
        let sync_w = col_w(ahead_behind_spans, 4, 9);
        let chg_w = col_w(changed_count_spans, 7, 9);
        let stash_w = col_w(stash_spans, 5, 6);
        let prs_w = col_w(prs_spans, 3, 5);
        let ci_w = col_w(ci_spans, 2, 8);
        let fixed = 2 + repo_w + sync_w + chg_w + stash_w + prs_w + ci_w + age_w;
        let status_w = (inner.width as usize).saturating_sub(fixed as usize + 2 * 7); // 8 cols ⇒ 7 gaps
        let widths = vec![
            Constraint::Length(repo_w), // Repo @branch (incl. 2-col gutter)
            Constraint::Length(sync_w),
            Constraint::Length(chg_w),
            Constraint::Length(stash_w),
            Constraint::Length(prs_w),
            Constraint::Length(ci_w),
            Constraint::Length(age_w),
            Constraint::Fill(1), // Status (primary attention reason)
        ];
        (widths, status_w, 0)
    } else {
        let sync_w = col_w(sync_spans, 4, 12);
        let changes_w = col_w(changes_spans, 7, 12);
        let fixed = 2 + repo_w as usize + sync_w as usize + changes_w as usize;
        let last_w = (inner.width as usize).saturating_sub(fixed + 2 * 3); // 4 cols ⇒ 3 gaps
        let widths = vec![
            Constraint::Length(repo_w),
            Constraint::Length(sync_w),
            Constraint::Length(changes_w),
            Constraint::Fill(1), // Last commit (age + message)
        ];
        (widths, 0, last_w.saturating_sub(5)) // age prefix `{:>3}  ` = 5 cols
    };

    let fmt = RowFmt {
        dock: dock_visible,
        summary_max,
        reason_max,
        age_w,
        // ~400ms on / ~400ms off at the 100ms tick cadence.
        blink_on: (app.spinner / 4).is_multiple_of(2),
    };
    let spin = spinner_frame(app.spinner);
    let rows: Vec<Row> = view
        .iter()
        .map(|vr| {
            let snap = &app.repos[vr.index];
            let busy = app.busy.contains(&snap.id).then_some(spin);
            let marked = app.selection.contains(&snap.id);
            repo_row(snap, &vr.name_highlights, now, theme, busy, marked, &fmt)
        })
        .collect();

    // The header, rendered manually (so the hint can sit between it and the data)
    // but aligned to the data table: a 2-col reserve for the highlight symbol,
    // then the same column widths/spacing. "Repo" is padded by 2 to line up with
    // the names' selection gutter.
    let labels: &[&str] = if dock_visible {
        &[
            "  Repo", "Sync", "Changes", "Stash", "PRs", "CI", "Last", "Status",
        ]
    } else {
        &["  Repo", "Sync", "Changes", "Last commit"]
    };
    let header_style = Style::new().add_modifier(Modifier::BOLD);
    let [_sym, header_cols] =
        Layout::horizontal([Constraint::Length(2), Constraint::Min(0)]).areas(header_area);
    let col_rects = Layout::horizontal(widths.clone())
        .spacing(2)
        .split(header_cols);
    for (rect, label) in col_rects.iter().zip(labels.iter()) {
        frame.render_widget(Paragraph::new(Span::styled(*label, header_style)), *rect);
    }

    let table = Table::new(rows, widths)
        .column_spacing(2)
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▌ ");

    let mut state = TableState::default();
    if !view.is_empty() {
        state.select(Some(app.selected));
    }
    // Use the offset we computed above so the hint and the visible rows agree
    // (and the table doesn't re-scroll past our clamp).
    *state.offset_mut() = offset;
    frame.render_stateful_widget(table, data_area, &mut state);

    // The scroll hints, centered on their own rows inside the box: repos hidden
    // above the header and/or below the fold.
    let hint_style = theme.ahead().add_modifier(Modifier::BOLD);
    if let Some(tn) = top_hint {
        frame.render_widget(
            Paragraph::new(
                Line::from(Span::styled(format!("↑ {above} more"), hint_style)).centered(),
            ),
            tn,
        );
    }
    if let Some(bn) = bot_hint {
        frame.render_widget(
            Paragraph::new(
                Line::from(Span::styled(format!("↓ {below} more"), hint_style)).centered(),
            ),
            bn,
        );
    }

    // The context pane, in the reserved bottom rows of this same box.
    if let (true, Some(d)) = (dock_visible, dock.as_ref()) {
        render_inbox_dock(frame, dock_area, d, theme);
    }
}

/// A pre-built docked context pane, laid out before the pane is sized so the box
/// can collapse to its content (no trailing blank rows).
struct DockBox {
    /// Left title, e.g. ` payments  ·  main ` or ` Working — 2 repo(s) `.
    title: String,
    /// Right-aligned title hint (the idle pane's `Enter: full detail`).
    right: Option<&'static str>,
    /// Border colour — dim when idle, accent while an action runs.
    border: Style,
    /// Content rows, already laid out for the given inner width.
    lines: Vec<Line<'static>>,
}

/// The most message lines the dock shows before truncating with `…`, so one huge
/// commit can't make the collapsing pane dominate the screen.
const DOCK_MSG_MAX: usize = 3;

/// Build the docked context pane's content for inner width `inner_w`: the
/// selected repo at a glance when idle, or live progress while a bulk action
/// runs. `None` when there's nothing to show. It fills the space a short fleet
/// would otherwise leave empty and makes the dashboard a cockpit — move the
/// cursor and the pane follows. Cheap by design: it reads the snapshot we already
/// have (no background fetches); the full remote-enriched view lives behind
/// `Enter`.
fn build_dock(app: &App, now: i64, theme: &Theme, inner_w: usize) -> Option<DockBox> {
    // While a bulk fetch/pull/push/stash runs, the dock shows what's in flight
    // rather than the cursor's repo.
    if !app.busy.is_empty() {
        let spin = spinner_frame(app.spinner);
        let lines: Vec<Line> = app
            .repos
            .iter()
            .filter(|r| app.busy.contains(&r.id))
            .map(|r| {
                Line::from(vec![
                    Span::styled(format!("{spin} "), theme.ahead()),
                    Span::raw(r.name.clone()),
                ])
            })
            .collect();
        return Some(DockBox {
            title: format!(" Working — {} repo(s) ", app.busy.len()),
            right: None,
            border: theme.ahead(),
            lines,
        });
    }

    let snap = app.selected_repo()?;
    let branch = match &snap.branch {
        Branch::Named(b) => b.clone(),
        Branch::Detached(id) => format!("@{}", id.chars().take(7).collect::<String>()),
        Branch::Unborn => "unborn".to_string(),
    };
    let title = format!(" {}  ·  {branch} ", snap.name);

    // A broken repo: just the error, in red — none of the facts below apply.
    if let Some(reason) = &snap.error {
        return Some(DockBox {
            title,
            right: Some(" Enter: full detail "),
            border: theme.dim(),
            lines: vec![Line::from(Span::styled(
                format!("⚠ {reason}"),
                theme.error(),
            ))],
        });
    }

    // Labelled facts — each on its own row with a purple, fixed-width label so the
    // pane reads as a small form, not a cryptic glyph soup. Everything is spelled
    // out (no `s1`/`●`); colour carries urgency.
    let mut lines: Vec<Line> = Vec::new();

    // Working tree — the staged/modified/untracked breakdown the row's count hides.
    let w = &snap.worktree;
    let changes_val = if w.staged + w.modified + w.untracked == 0 {
        vec![Span::styled("clean", theme.dim())]
    } else {
        let mut parts = Vec::new();
        if w.staged > 0 {
            parts.push(format!("{} staged", w.staged));
        }
        if w.modified > 0 {
            parts.push(format!("{} modified", w.modified));
        }
        if w.untracked > 0 {
            parts.push(format!("{} untracked", w.untracked));
        }
        let style = if w.modified > 0 || w.untracked > 0 {
            theme.modified()
        } else {
            theme.staged()
        };
        vec![Span::styled(parts.join(" · "), style)]
    };
    dock_field(&mut lines, "Changes", changes_val, theme);

    // Stash — count, and whether the newest is stale (a forgotten stash).
    let stash_stale = assess(snap, now)
        .reasons
        .iter()
        .any(|r| matches!(r, AttentionReason::Stash { stale: true, .. }));
    let stash_val = if snap.stash_count == 0 {
        vec![Span::styled("none", theme.dim())]
    } else if stash_stale {
        vec![Span::styled(
            format!("{} · stale", snap.stash_count),
            theme.warn(),
        )]
    } else {
        vec![Span::raw(snap.stash_count.to_string())]
    };
    dock_field(&mut lines, "Stash", stash_val, theme);

    // Upstream — even / ahead / behind / no upstream, naming the tracked branch.
    let upstream_val = match &snap.upstream {
        None => vec![Span::styled("no upstream", theme.dim())],
        Some(up) if up.ahead == 0 && up.behind == 0 => {
            vec![Span::styled(format!("even with {}", up.name), theme.dim())]
        }
        Some(up) => {
            let mut spans = Vec::new();
            if up.ahead > 0 {
                spans.push(Span::styled(format!("{} ahead", up.ahead), theme.ahead()));
            }
            if up.ahead > 0 && up.behind > 0 {
                spans.push(Span::styled(" · ", theme.dim()));
            }
            if up.behind > 0 {
                spans.push(Span::styled(
                    format!("{} behind", up.behind),
                    theme.behind(),
                ));
            }
            spans.push(Span::styled(format!("  ({})", up.name), theme.dim()));
            spans
        }
    };
    dock_field(&mut lines, "Upstream", upstream_val, theme);

    // Remote — CI health, open PRs, and any awaiting the user's review.
    let remote_val = match &snap.remote {
        None => vec![Span::styled("no GitHub remote", theme.dim())],
        Some(r) => {
            let (ci_label, ci_style) = match r.ci {
                CiStatus::Passing => ("CI passing", theme.ok()),
                CiStatus::Failing => ("CI failing", theme.risk()),
                CiStatus::Pending => ("CI pending", theme.warn()),
                CiStatus::None => ("no CI", theme.dim()),
            };
            let prs = if r.open_prs == 0 {
                "no open PRs".to_string()
            } else {
                format!(
                    "{} open PR{}",
                    r.open_prs,
                    if r.open_prs == 1 { "" } else { "s" }
                )
            };
            let mut spans = vec![
                Span::styled(ci_label, ci_style),
                Span::styled(format!("  ·  {prs}"), theme.dim()),
            ];
            if r.prs_awaiting_review > 0 {
                spans.push(Span::styled(
                    format!("  ·  {} awaiting your review", r.prs_awaiting_review),
                    theme.warn(),
                ));
            }
            spans
        }
    };
    dock_field(&mut lines, "Remote", remote_val, theme);

    // Last commit — age + the full message (the row no longer shows it),
    // word-wrapped across the lines left in the pane rather than truncated.
    match &snap.last_commit {
        None => dock_field(
            &mut lines,
            "Last commit",
            vec![Span::styled("no commits", theme.dim())],
            theme,
        ),
        Some(c) => {
            let head = format!("{} ago — ", time::relative(c.timestamp, now));
            // First message line shares its row with the label + age; wrapped
            // continuation lines sit under the value column.
            let first_w = inner_w
                .saturating_sub(DOCK_LABEL_W + head.chars().count())
                .max(1);
            let rest_w = inner_w.saturating_sub(DOCK_LABEL_W).max(1);
            let mut wrapped = word_wrap(&c.summary, first_w, rest_w);

            // Cap to a few lines; mark the cut with an ellipsis.
            if wrapped.len() > DOCK_MSG_MAX {
                wrapped.truncate(DOCK_MSG_MAX);
                let w = if DOCK_MSG_MAX == 1 { first_w } else { rest_w };
                if let Some(last) = wrapped.last_mut() {
                    let mut s: String = last.chars().take(w.saturating_sub(1)).collect();
                    s.push('…');
                    *last = s;
                }
            }

            dock_field(
                &mut lines,
                "Last commit",
                vec![
                    Span::styled(head, theme.dim()),
                    Span::raw(wrapped.first().cloned().unwrap_or_default()),
                ],
                theme,
            );
            let indent = " ".repeat(DOCK_LABEL_W);
            for chunk in wrapped.iter().skip(1) {
                lines.push(Line::from(vec![
                    Span::raw(indent.clone()),
                    Span::raw(chunk.clone()),
                ]));
            }
        }
    }

    Some(DockBox {
        title,
        right: Some(" Enter: full detail "),
        border: theme.dim(),
        lines,
    })
}

/// Greedy word-wrap: split `text` on whitespace into lines that fit `first`
/// columns for the first line and `rest` for the others. A single word wider
/// than the limit is left long (the cell clips it) rather than broken mid-word.
fn word_wrap(text: &str, first: usize, rest: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut width = first;
    for word in text.split_whitespace() {
        if cur.is_empty() {
            cur.push_str(word);
        } else if cur.chars().count() + 1 + word.chars().count() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            width = rest;
            cur.push_str(word);
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Width of the dim label column in the dock's labelled facts.
const DOCK_LABEL_W: usize = 13;

/// Push one labelled fact row into the dock: a dim, fixed-width label then the
/// value spans.
fn dock_field(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    value: Vec<Span<'static>>,
    theme: &Theme,
) {
    let mut spans = vec![Span::styled(
        format!("{label:<DOCK_LABEL_W$}"),
        theme.fg(SPIDER_PURPLE),
    )];
    spans.extend(value);
    lines.push(Line::from(spans));
}

/// Render a pre-built [`DockBox`] inside the Repositories box, in the reserved
/// bottom rows: a titled divider rule (the repo name·branch on the left, the
/// `Enter` hint on the right), a blank pad row, then the laid-out facts.
fn render_inbox_dock(frame: &mut Frame, area: Rect, dock: &DockBox, theme: &Theme) {
    let [divider, _pad, content] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(area);

    // A horizontal rule, drawn as a top border so it spans the box width and
    // butts against the side borders; the titles ride on it.
    let mut rule = Block::new()
        .borders(Borders::TOP)
        .border_style(dock.border)
        .title(Line::from(dock.title.clone()).bold());
    if let Some(right) = dock.right {
        rule = rule.title(Line::from(Span::styled(right, theme.dim())).right_aligned());
    }
    frame.render_widget(rule, divider);

    // The facts, indented one column off the border.
    let body = Rect {
        x: content.x + 1,
        width: content.width.saturating_sub(1),
        ..content
    };
    frame.render_widget(Paragraph::new(dock.lines.clone()), body);
}

/// Map an attention [`Severity`] to its accent style for the dock's reason chips.
fn severity_style(sev: Severity, theme: &Theme) -> Style {
    match sev {
        Severity::Ok => theme.ok(),
        Severity::Info => theme.dim(),
        Severity::Notice => theme.ahead(),
        Severity::Warn => theme.warn(),
        Severity::Risk => theme.risk(),
    }
}

/// How to render a row's columns, which depend on whether the dock is up (and so
/// carrying the commit message, freeing the table to expand its stat columns).
struct RowFmt {
    /// Dock up ⇒ expanded columns (Sync / Changes / Stash / PRs / CI / Last /
    /// Status); else the compact, fused columns and one "Last commit".
    dock: bool,
    /// Max chars for the commit summary (the non-dock "Last commit" column).
    summary_max: usize,
    /// Max chars for the reason label (the dock "Status" column).
    reason_max: usize,
    /// Width of the age column, for right-aligning the age (the dock layout).
    age_w: u16,
    /// Blink phase for the "synced" dot: on this frame the green `●` shows; off,
    /// it's blank. Toggled from the animation tick so the dot blinks (terminals
    /// ignore the ANSI blink attribute, so we animate it ourselves).
    blink_on: bool,
}

fn repo_row<'a>(
    snap: &'a RepoSnapshot,
    highlights: &[u32],
    now: i64,
    theme: &Theme,
    busy: Option<&str>,
    marked: bool,
    fmt: &RowFmt,
) -> Row<'a> {
    if let Some(reason) = &snap.error {
        // A broken repo: a red name, the reason in the wide trailing Status column.
        // The data columns are genuinely unknowable, so they're left blank (a
        // single space keeps the cell non-empty so the table doesn't collapse). The
        // leading "  " keeps the name aligned with the selection gutter.
        let blank = || Cell::from(Span::raw(" "));
        let name = Cell::from(Line::from(vec![
            Span::raw("  "),
            Span::styled(snap.name.clone(), theme.error()),
        ]));
        let status = Cell::from(Span::styled(
            ellipsize(reason, fmt.reason_max),
            theme.risk(),
        ));
        return if fmt.dock {
            Row::new(vec![
                name,
                blank(), // Sync
                blank(), // Changes
                blank(), // Stash
                blank(), // PRs
                blank(), // CI
                blank(), // Last
                status,
            ])
        } else {
            Row::new(vec![name, blank(), blank(), status])
        };
    }

    let assessment = assess(snap, now);
    let severity = assessment.severity;
    let name = name_cell(
        &snap.name,
        highlights,
        severity,
        marked,
        &snap.branch,
        theme,
    );

    // The "synced" state (even with upstream): in the glyph tiers it's a blinking
    // green dot (blank on the off phase); under ASCII it's a steady "ok" word that
    // never blinks (so the state never depends on an animation). `blink_synced()`
    // gates the blanking accordingly.
    let synced = matches!(&snap.upstream, Some(u) if u.ahead == 0 && u.behind == 0);
    let blink_synced = theme.glyphs.blink_synced();

    if fmt.dock {
        // While an action runs, the Sync cell shows a spinner instead.
        let sync = match busy {
            Some(spin) => Cell::from(Span::styled(spin.to_string(), theme.ahead())),
            None if synced && blink_synced && !fmt.blink_on => Cell::from(Span::raw(" ")),
            None => Cell::from(Line::from(ahead_behind_spans(snap, theme))),
        };
        Row::new(vec![
            name,
            sync,
            Cell::from(Line::from(changed_count_spans(snap, theme))),
            Cell::from(Line::from(stash_spans(snap, theme))),
            Cell::from(Line::from(prs_spans(snap, theme))),
            Cell::from(Line::from(ci_spans(snap, theme))),
            age_cell(snap, now, fmt.age_w as usize, theme),
            reason_cell(assessment.primary.as_ref(), fmt.reason_max, theme),
        ])
    } else {
        let sync = match busy {
            Some(spin) => Cell::from(Span::styled(spin.to_string(), theme.ahead())),
            None if synced && snap.remote.is_none() && blink_synced && !fmt.blink_on => {
                Cell::from(Span::raw(" "))
            }
            None => sync_cell(snap, theme),
        };
        Row::new(vec![
            name,
            sync,
            changes_cell(snap, theme),
            last_commit_cell(snap, now, fmt.summary_max, theme),
        ])
    }
}

/// The "Last" column: the commit age alone, right-aligned (the message lives in
/// the dock when this layout is active).
fn age_cell<'a>(snap: &RepoSnapshot, now: i64, age_w: usize, theme: &Theme) -> Cell<'a> {
    let age = match &snap.last_commit {
        Some(c) => time::relative(c.timestamp, now),
        None => "—".to_string(),
    };
    Cell::from(Span::styled(format!("{age:>age_w$}"), theme.dim()))
}

/// The "Status" column: the repo's primary attention reason, colored by severity
/// — the same signal that drives the dirty-first sort, made visible per row. A
/// dim "up to date" when the repo wants nothing (rather than a bare dot).
fn reason_cell<'a>(primary: Option<&AttentionReason>, max: usize, theme: &Theme) -> Cell<'a> {
    match primary {
        Some(r) => Cell::from(Span::styled(
            ellipsize(&r.label(), max),
            severity_style(r.severity(), theme),
        )),
        None => Cell::from(Span::styled(ellipsize("up to date", max), theme.dim())),
    }
}

/// The "Sync" column (dock layout): ahead/behind arrows — `↑2 ↓5`, `↑2`, `↓5` — a
/// blinking green `●` when in sync with upstream, "local" when there's no
/// upstream. The remote PR/CI signal moved to its own columns.
fn ahead_behind_spans(snap: &RepoSnapshot, theme: &Theme) -> Vec<Span<'static>> {
    match &snap.upstream {
        None => vec![Span::styled("local", theme.dim())],
        Some(up) if up.ahead == 0 && up.behind == 0 => {
            vec![Span::styled(theme.glyphs.synced(), theme.ok())]
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
    }
}

/// The "Changes" column (dock layout): the changed-file count, colored (green
/// when fully staged, yellow when there's unstaged work); a dim "0" when clean.
/// The stash moved to its own column.
fn changed_count_spans(snap: &RepoSnapshot, theme: &Theme) -> Vec<Span<'static>> {
    let w = &snap.worktree;
    let total = w.staged + w.modified + w.untracked;
    if total == 0 {
        vec![Span::styled("0", theme.dim())]
    } else {
        let style = if w.modified > 0 || w.untracked > 0 {
            theme.modified()
        } else {
            theme.staged()
        };
        vec![Span::styled(total.to_string(), style)]
    }
}

/// The "Stash" column (dock layout): the stash count, or a dim "0" when there are
/// none.
fn stash_spans(snap: &RepoSnapshot, theme: &Theme) -> Vec<Span<'static>> {
    if snap.stash_count > 0 {
        vec![Span::styled(snap.stash_count.to_string(), theme.warn())]
    } else {
        vec![Span::styled("0", theme.dim())]
    }
}

/// The "PRs" column (dock layout): open pull-request count — a dim "0" on a remote
/// with none, "local" when the repo isn't on a remote.
fn prs_spans(snap: &RepoSnapshot, theme: &Theme) -> Vec<Span<'static>> {
    match &snap.remote {
        None => vec![Span::styled("local", theme.dim())],
        Some(r) if r.open_prs == 0 => vec![Span::styled("0", theme.dim())],
        Some(r) => vec![Span::styled(r.open_prs.to_string(), theme.ahead())],
    }
}

/// The "CI" column (dock layout): the check status spelled out and colored;
/// "local" when the repo isn't on a remote, "no CI" when it is but has no checks.
fn ci_spans(snap: &RepoSnapshot, theme: &Theme) -> Vec<Span<'static>> {
    match &snap.remote {
        None => vec![Span::styled("local", theme.dim())],
        Some(r) => {
            let (label, style) = match r.ci {
                CiStatus::Passing => ("passing", theme.ok()),
                CiStatus::Failing => ("failing", theme.risk()),
                CiStatus::Pending => ("pending", theme.warn()),
                CiStatus::None => ("no CI", theme.dim()),
            };
            vec![Span::styled(label, style)]
        }
    }
}

/// The remote sub-part of the Sync column: a status dot colored by CI health —
/// green passing, red failing, yellow pending, dim when there's no CI signal —
/// plus the open-PR count. Empty when the repo isn't on a remote.
///
/// In the glyph tiers it's `●` (a basic geometric glyph present in every monospace
/// font, colored via ANSI) rather than a cloud emoji — emoji are double-width,
/// can't be themed/`NO_COLOR`'d, and may render invisibly. Under Ascii the colour
/// can't carry pass/fail, so the dot becomes a short letter (`ok`/`x`/`~`) via
/// [`Glyphs::ci_dot`].
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
            let dot = theme.glyphs.ci_dot(r.ci);
            let mut spans = Vec::new();
            if !dot.is_empty() {
                spans.push(Span::styled(dot, style));
            }
            if r.open_prs > 0 {
                let sep = if spans.is_empty() { "" } else { " " };
                spans.push(Span::styled(format!("{sep}{}pr", r.open_prs), theme.dim()));
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
    branch: &Branch,
    theme: &Theme,
) -> Cell<'a> {
    // Red is reserved for genuinely broken repos (the `snap.error` row); a repo
    // that merely wants attention (aging unpushed, behind, dirty) is conveyed by
    // weight here and by the coloured Sync/Changes cells — never an alarming red
    // name (which, on the reversed selected row, became a red block).
    let base = match severity {
        Severity::Ok | Severity::Info => theme.dim(),
        Severity::Warn | Severity::Risk => Style::new().add_modifier(Modifier::BOLD),
        Severity::Notice => Style::new(),
    };
    let gutter = if marked {
        Span::styled(format!("{} ", theme.glyphs.marked()), theme.ahead())
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
    // The branch is fused onto the name (`name @branch`), mirroring the web — a
    // dim suffix (or magenta for a detached `@sha`), so Repo + Branch share one
    // tight column.
    let bstyle = match branch {
        Branch::Detached(_) => theme.detached(),
        _ => theme.dim(),
    };
    spans.push(Span::styled(branch_suffix(branch), bstyle));
    Cell::from(Line::from(spans))
}

/// The `@branch` suffix appended to the repo name: `@main`, `@a1b2c3d` for a
/// detached HEAD, or `unborn` for a fresh repo. The branch name is ellipsized so
/// one long branch can't blow out the fused Repo column.
fn branch_suffix(branch: &Branch) -> String {
    match branch {
        Branch::Named(b) => format!(" @{}", ellipsize(b, 18)),
        Branch::Detached(id) => format!(" @{}", id.chars().take(7).collect::<String>()),
        Branch::Unborn => " unborn".to_string(),
    }
}

/// The styled spans of the fused Repo cell, *without* fuzzy highlights — used only
/// to size the column to its widest content (see [`col_w`]).
fn repo_spans(snap: &RepoSnapshot, _theme: &Theme) -> Vec<Span<'static>> {
    vec![
        Span::raw("  "),
        Span::raw(snap.name.clone()),
        Span::raw(branch_suffix(&snap.branch)),
    ]
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
                vec![Span::styled(theme.glyphs.synced(), theme.ok())]
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

    // Join the two sub-parts with a space; fall back to "local" (no upstream and
    // no remote, i.e. a purely local repo) when both are empty.
    let mut spans = upstream;
    if !spans.is_empty() && !remote.is_empty() {
        spans.push(Span::raw(" "));
    }
    spans.extend(remote);
    if spans.is_empty() {
        spans.push(Span::styled("local", theme.dim()));
    }
    spans
}

/// The Changes column: changed-file count (green when all staged, yellow when
/// there's unstaged work), plus the stash folded in as a dim `s{n}` when there
/// are stashes. A dim "0" when the tree is clean and nothing is stashed.
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
        spans.push(Span::styled("0", theme.dim()));
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
fn last_commit_cell<'a>(
    snap: &'a RepoSnapshot,
    now: i64,
    summary_max: usize,
    theme: &Theme,
) -> Cell<'a> {
    match &snap.last_commit {
        Some(commit) => Cell::from(Line::from(vec![
            Span::styled(
                format!("{:>3}  ", time::relative(commit.timestamp, now)),
                theme.dim(),
            ),
            Span::raw(ellipsize(&commit.summary, summary_max)),
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
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let s = |t: &str, st: Style| Span::styled(t.to_string(), st);

    // One shared two-column grid for the whole overlay: a 2-space-indented left
    // cell (a colored glyph example, or a key) is padded so *every* description
    // starts in the same column — so the legend and the keymap read as one
    // aligned table instead of a wrapping paragraph.
    let desc_col = 16usize;
    let row = |left: Vec<Span<'static>>, desc: &str| -> Line<'static> {
        let used: usize = left.iter().map(|sp| sp.content.chars().count()).sum();
        let pad = desc_col.saturating_sub(used).max(2);
        let mut spans = vec![Span::raw("  ")];
        spans.extend(left);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(desc.to_string(), theme.dim()));
        Line::from(spans)
    };
    let head = |t: &str| Line::from(Span::styled(t.to_string(), bold));
    let key = |t: &str| vec![Span::styled(t.to_string(), bold)];

    let lines = vec![
        // The legend first: it answers "what am I looking at?", which is the
        // question a new user has before they care about keybindings.
        head("Legend — what each column shows"),
        row(vec![s("↑2", theme.ahead())], "commits ahead of upstream"),
        row(vec![s("↓5", theme.behind())], "commits behind upstream"),
        row(
            vec![s(theme.glyphs.synced(), theme.ok())],
            "in sync with upstream",
        ),
        row(vec![s("local", theme.dim())], "no upstream (local only)"),
        row(
            vec![
                s(theme.glyphs.ci_dot(CiStatus::Passing), theme.ok()),
                Span::raw(" "),
                s(theme.glyphs.ci_dot(CiStatus::Failing), theme.risk()),
                Span::raw(" "),
                s(theme.glyphs.ci_dot(CiStatus::Pending), theme.warn()),
            ],
            "CI: pass / fail / pending",
        ),
        row(vec![s("2pr", theme.dim())], "open pull requests"),
        row(vec![s("3", theme.staged())], "changed files, all staged"),
        row(
            vec![s("3", theme.modified())],
            "changed files, some unstaged",
        ),
        row(vec![s("s1", theme.dim())], "stash entries"),
        row(vec![s("name", theme.dim())], "row: clean"),
        row(vec![s("name", bold)], "row: needs attention"),
        row(vec![s("name", theme.error())], "row: unreadable (error)"),
        row(
            vec![s(theme.glyphs.marked(), theme.ahead())],
            "row: marked for a bulk action",
        ),
        Line::from(""),
        head("Navigation"),
        row(key("↑ / ↓"), "move cursor"),
        row(key("Home / End"), "top / bottom"),
        Line::from(""),
        head("Select & filter"),
        row(key("Space"), "mark / unmark the repo"),
        row(key("a"), "mark all (again to clear)"),
        row(key("Esc"), "clear the selection"),
        row(key("/"), "fuzzy filter (Esc clears)"),
        row(
            key(":"),
            "command mode (:fetch, :!cmd shell, :sort, :jump …)",
        ),
        row(key("d"), "toggle dirty-only"),
        row(key("s"), "cycle sort mode"),
        row(key("Tab"), "weekly standup"),
        Line::from(""),
        head("Act — on the marked repos, else the current one"),
        row(key("⏎"), "inspect repo — commits, changes, branches"),
        row(key("o"), "open with… (editors, reveal, lazygit)"),
        row(key("f / F"), "fetch selection / all"),
        row(key("p"), "pull (fast-forward only)"),
        row(key("P"), "push (current branch upstream)"),
        row(key("!"), "shell command (opens the : palette)"),
        row(key("S"), "stash (asks to confirm)"),
        row(key("L"), "open in lazygit"),
        row(key("y"), "copy path to clipboard"),
        Line::from(""),
        head("App"),
        row(key("r"), "refresh (re-scan)"),
        row(key("h"), "show / hide the hint bar"),
        row(key("?"), "toggle this help"),
        row(key("q / Ctrl-C"), "quit"),
        Line::from(""),
        Line::from(Span::styled(
            format!("cohors v{}", env!("CARGO_PKG_VERSION")),
            theme.dim(),
        )),
        Line::from(Span::styled(
            format!("config: {}", app.config_path),
            theme.dim(),
        )),
    ];

    // Collapse the box to its content (capped at 90% of the screen height) so the
    // help doesn't sit in a tall, mostly-empty modal.
    let want_h = lines.len() as u16 + 4; // top/bottom padding (2) + top/bottom border (2)
    let h = want_h.min((full.height * 90 / 100).max(10));
    let w = (full.width * 70 / 100).max(40);
    let area = Rect {
        x: full.x + full.width.saturating_sub(w) / 2,
        y: full.y + full.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, area);

    let para = Paragraph::new(Text::from(lines))
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Help ")
                .padding(Padding::new(2, 2, 1, 1)),
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

/// Count commits by their conventional-commit type (the word before the first
/// `:`, scope stripped: `feat(ui): …` → `feat`), most-common first. Powers the
/// standup's at-a-glance "what you did" description.
fn commit_type_counts(commits: &[StandupCommit]) -> Vec<(String, usize)> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for c in commits {
        if let Some(kind) = commit_kind(&c.summary) {
            *counts.entry(kind).or_default() += 1;
        }
    }
    let mut v: Vec<(String, usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v
}

/// A stable colour per commit type, so the standup reads at a glance. Unknown
/// types fall back to dim. Respects `NO_COLOR` via `Theme::fg`.
fn commit_type_style(kind: &str, theme: &Theme) -> Style {
    let color = match kind {
        "feat" => Color::Green,
        "fix" => Color::Red,
        "design" | "style" => Color::Magenta,
        "chore" | "build" | "ci" | "deps" => Color::Blue,
        "docs" | "content" => Color::Cyan,
        "refactor" | "perf" => Color::Yellow,
        "test" => Color::LightGreen,
        _ => return theme.dim(),
    };
    theme.fg(color)
}

/// The commit type (`feat`, `fix`, scope stripped) at the start of a summary, if
/// it reads like a conventional-commit prefix.
fn commit_kind(summary: &str) -> Option<String> {
    let (prefix, _) = summary.split_once(':')?;
    let kind = prefix.split('(').next().unwrap_or(prefix).trim();
    (!kind.is_empty() && kind.len() <= 12 && !kind.contains(' ')).then(|| kind.to_lowercase())
}

/// A natural-language clause for a commit type, as `(lead, noun)` — e.g.
/// `("shipping 68 ", "features")` — so the standup can read like a sentence
/// ("…, shipping 68 features, fixing 3 bugs, …"). The noun is coloured by kind
/// by the caller; the lead stays plain. Unknown kinds get a generic phrasing.
fn commit_phrase(kind: &str, n: usize) -> (String, String) {
    let known = match kind {
        "feat" => Some(("shipping", "feature", "features")),
        "fix" => Some(("fixing", "bug", "bugs")),
        "design" => Some(("polishing", "design change", "design changes")),
        "style" => Some(("tidying", "style change", "style changes")),
        "chore" => Some(("clearing", "chore", "chores")),
        "docs" => Some(("writing", "doc update", "doc updates")),
        "content" => Some(("writing", "content update", "content updates")),
        "refactor" => Some(("refactoring in", "commit", "commits")),
        "perf" => Some(("tuning", "perf change", "perf changes")),
        "test" => Some(("adding", "test", "tests")),
        "build" => Some(("updating", "build change", "build changes")),
        "ci" => Some(("tweaking", "CI change", "CI changes")),
        _ => None,
    };
    match known {
        Some((verb, singular, plural)) => {
            let noun = if n == 1 { singular } else { plural };
            (format!("{verb} {n} "), noun.to_string())
        }
        // Unknown type → keep the literal kind: "3 wip commits".
        None => (
            format!("{n} "),
            format!("{kind} commit{}", if n == 1 { "" } else { "s" }),
        ),
    }
}

/// A commit summary as styled spans: the type prefix coloured by kind, the rest
/// plain.
fn commit_summary_spans(summary: &str, theme: &Theme) -> Vec<Span<'static>> {
    if let Some(kind) = commit_kind(summary)
        && let Some((prefix, rest)) = summary.split_once(':')
    {
        return vec![
            Span::styled(format!("{prefix}:"), commit_type_style(&kind, theme)),
            Span::raw(rest.to_string()),
        ];
    }
    vec![Span::raw(summary.to_string())]
}

fn render_standup(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    // Size the modal to its content (capped) so it doesn't dominate the screen:
    // the panes are as tall as the busiest repo's commit list, up to a cap, and
    // anything past that scrolls.
    let body = app
        .standup
        .as_ref()
        .map(|v| {
            let max_c = v.groups.iter().map(|(_, c)| c.len()).max().unwrap_or(1);
            v.groups.len().max(max_c)
        })
        .unwrap_or(3)
        .clamp(6, 16) as u16;
    // body + description(3) + gap(1) + pane border(2) + outer border(2) + pad(1).
    let h = (body + 9).min(full.height.saturating_sub(2)).max(12);
    let w = (full.width as u32 * 84 / 100) as u16;
    let area = Rect {
        x: full.x + full.width.saturating_sub(w) / 2,
        y: full.y + full.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
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

    // A dynamic, one-line description above the panes so the view explains
    // itself: which window, and what each column is.
    let repo_name = view
        .groups
        .get(view.focus)
        .map(|(r, _)| r.as_str())
        .unwrap_or("");
    // The description (up to 3 wrapped lines), then a blank gap, then the panes.
    let [desc_area, _gap, panes_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);

    // A glance of *what you did*, as one flowing, wrapping sentence:
    // "You authored 130 commits this week across 5 repos, shipping 68 features,
    //  fixing 14 bugs, polishing 16 design changes, and writing 8 doc updates."
    // (the type nouns are coloured by kind; the rest is prose.)
    let mut spans = vec![
        Span::styled("You authored ", theme.dim()),
        Span::styled(
            total.to_string(),
            theme.ahead().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " commit{} {window} across {repos} repo{}",
                if total == 1 { "" } else { "s" },
                if repos == 1 { "" } else { "s" }
            ),
            theme.dim(),
        ),
    ];
    // Each commit type becomes a clause ("shipping 68 features"); join them with
    // commas and a final "and".
    let counts = commit_type_counts(&view.commits);
    let mut clauses: Vec<Vec<Span>> = counts
        .iter()
        .take(5)
        .map(|(kind, n)| {
            let (lead, noun) = commit_phrase(kind, *n);
            vec![
                Span::styled(lead, theme.dim()),
                Span::styled(noun, commit_type_style(kind, theme)),
            ]
        })
        .collect();
    if counts.len() > 5 {
        let rest: usize = counts.iter().skip(5).map(|(_, n)| *n).sum();
        clauses.push(vec![Span::styled(
            format!("and {rest} more across other kinds"),
            theme.dim(),
        )]);
    }
    if !clauses.is_empty() {
        spans.push(Span::styled(", ", theme.dim()));
        let last = clauses.len() - 1;
        let has_more = counts.len() > 5; // the final clause already starts with "and"
        for (i, clause) in clauses.into_iter().enumerate() {
            if i > 0 {
                let sep = if i == last && !has_more {
                    if last > 1 { ", and " } else { " and " }
                } else {
                    ", "
                };
                spans.push(Span::styled(sep, theme.dim()));
            }
            spans.extend(clause);
        }
    }
    spans.push(Span::styled(".", theme.dim()));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).wrap(Wrap { trim: true }),
        desc_area,
    );

    // A "Repos" box and a "{repo}" commits box: side-by-side when there's room,
    // stacked (repos on top) on a narrow terminal.
    let (left_area, right_area) = two_pane(panes_area, 26, 8);

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

    // Right: the focused repo's commits, with the commit count in the title and
    // a highlighted cursor (so scrolling is contextual).
    let n = view.focused_len();
    let commits_block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(
            Line::from(format!(
                " {repo_name}  ·  {n} commit{} ",
                if n == 1 { "" } else { "s" }
            ))
            .style(active(view.commits_focused)),
        )
        .padding(Padding::new(1, 0, 0, 0));
    let commits_inner = commits_block.inner(right_area);
    frame.render_widget(commits_block, right_area);

    let items: Vec<ListItem> = view
        .groups
        .get(view.focus)
        .map(|(_, commits)| {
            commits
                .iter()
                .map(|c| {
                    let mut spans = vec![
                        Span::styled(c.short_id.clone(), theme.ahead()),
                        Span::raw("  "),
                    ];
                    spans.extend(commit_summary_spans(&c.summary, theme));
                    ListItem::new(Line::from(spans))
                })
                .collect()
        })
        .unwrap_or_default();

    let [text_area, bar_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).areas(commits_inner);

    // Keep the highlighted commit visible: nudge the scroll offset only as far as
    // needed, then clamp. This drives both the list and the scrollbar.
    let total = items.len() as u16;
    let viewport = text_area.height;
    let cursor = view.commit_cursor as u16;
    let mut offset = view.offset();
    if cursor < offset {
        offset = cursor;
    } else if viewport > 0 && cursor >= offset + viewport {
        offset = cursor + 1 - viewport;
    }
    offset = offset.min(total.saturating_sub(viewport));
    view.set_offset(offset);

    let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut list_state = ListState::default();
    // Only show the cursor when the commits pane has focus.
    if view.commits_focused {
        list_state.select(Some(view.commit_cursor));
    }
    *list_state.offset_mut() = offset as usize;
    frame.render_stateful_widget(list, text_area, &mut list_state);

    if total > viewport {
        let mut sb = ScrollbarState::new(total as usize)
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

/// A section header line inside the detail pane (blank spacer + bold title).
fn detail_section(lines: &mut Vec<Line<'static>>, title: String) {
    if !lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        title,
        Style::new().add_modifier(Modifier::BOLD),
    )));
}

/// Colour for a porcelain status: untracked dim, staged green, unstaged yellow.
fn changed_file_style(status: &str, theme: &Theme) -> Style {
    if status.starts_with("??") {
        theme.dim()
    } else if !status.starts_with(' ') {
        theme.staged()
    } else {
        theme.modified()
    }
}

/// The per-repo drill-in detail pane: recent commits, working-tree changes,
/// branches, and stashes for the current repo, scrollable. Shows a "loading"
/// placeholder until the background read finishes.
fn render_detail(frame: &mut Frame, full: Rect, app: &App, now: i64, theme: &Theme) {
    let Some(dv) = &app.detail else {
        return;
    };
    let branch = dv.detail.as_ref().and_then(|d| d.current_branch.clone());
    let title = match &branch {
        Some(b) => format!(" {}  ·  {b} ", dv.repo_name),
        None => format!(" {} ", dv.repo_name),
    };

    // Still loading: a small centred box, not a full-size empty one.
    let Some(d) = &dv.detail else {
        let area = detail_rect(full, 1);
        frame.render_widget(Clear, area);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Line::from(title).bold())
            .padding(Padding::new(1, 0, 0, 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(Span::styled("Reading repo…", theme.dim())),
            inner,
        );
        return;
    };

    // Build both panes' content up front so the modal can size to it (and so the
    // panes can scroll). Right pane: recent commits.
    let mut clines: Vec<Line> = Vec::new();
    for c in &d.recent_commits {
        let mut spans = vec![
            Span::styled(format!("{}  ", c.short_id), theme.ahead()),
            Span::styled(
                format!("{:>4}  ", time::relative(c.timestamp, now)),
                theme.dim(),
            ),
        ];
        spans.extend(commit_summary_spans(&c.summary, theme));
        clines.push(Line::from(spans));
    }

    // Left pane: changes · branches · stashes · PRs · contributors.
    let mut lines: Vec<Line> = Vec::new();
    detail_section(&mut lines, format!("Changes ({})", d.changed_files.len()));
    if d.changed_files.is_empty() {
        lines.push(Line::from(Span::styled("clean", theme.dim())));
    }
    for f in &d.changed_files {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", f.status),
                changed_file_style(&f.status, theme),
            ),
            Span::raw(f.path.clone()),
        ]));
    }
    detail_section(&mut lines, format!("Branches ({})", d.branches.len()));
    for (i, b) in d.branches.iter().enumerate() {
        let current = i == 0 && branch.as_deref() == Some(b.as_str());
        let style = if current {
            theme.ahead().add_modifier(Modifier::BOLD)
        } else {
            Style::new()
        };
        lines.push(Line::from(vec![
            Span::styled(if current { "● " } else { "  " }, style),
            Span::styled(b.clone(), style),
        ]));
    }
    if !d.stashes.is_empty() {
        detail_section(&mut lines, format!("Stashes ({})", d.stashes.len()));
        for s in &d.stashes {
            lines.push(Line::from(Span::raw(s.clone())));
        }
    }
    render_remote_sections(&mut lines, dv, theme);

    // Collapse the modal to the taller pane's content (its rows + the pane's own
    // border) rather than a fixed 84% box, so short repos don't show acres of
    // whitespace. `detail_rect` caps it at 84% of the screen.
    let content_rows = clines.len().max(lines.len()) as u16;
    let area = detail_rect(full, content_rows);
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(title).bold())
        .padding(Padding::new(1, 0, 0, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Two panes (like the standup): repo state on the left, commits on the right.
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .spacing(1)
            .areas(inner);

    // Both panes share `dv.scroll`, each clamped to its own overflow — so a short
    // pane stops while the longer one keeps scrolling, and either shows a
    // scrollbar only when it actually overflows.
    let commits_inner = render_pane_block(
        frame,
        right_area,
        format!(" Commits ({}) ", d.recent_commits.len()),
        theme,
    );
    let state_inner = render_pane_block(
        frame,
        left_area,
        " Changes · branches · PRs ".to_string(),
        theme,
    );
    let c_max = (clines.len() as u16).saturating_sub(commits_inner.height);
    let l_max = (lines.len() as u16).saturating_sub(state_inner.height);
    dv.set_max_scroll(c_max.max(l_max));

    render_scrolling_pane(frame, commits_inner, clines, dv.scroll.min(c_max));
    render_scrolling_pane(frame, state_inner, lines, dv.scroll.min(l_max));
}

/// A centred detail-modal rect sized to `content_rows` of pane content (plus the
/// pane border + the outer border), at 84% width, and capped at 84% of the
/// screen height with a sensible floor.
fn detail_rect(full: Rect, content_rows: u16) -> Rect {
    let max_h = (full.height * 84 / 100).max(7);
    let h = (content_rows + 4).clamp(7, max_h); // +2 pane border, +2 outer border
    let w = (full.width * 84 / 100).max(20);
    Rect {
        x: full.x + full.width.saturating_sub(w) / 2,
        y: full.y + full.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

/// Draw a detail sub-pane's bordered block and return its inner area.
fn render_pane_block(frame: &mut Frame, area: Rect, title: String, theme: &Theme) -> Rect {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.dim())
        .title(Line::from(title).bold())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// Render `lines` into `inner` scrolled by `offset`, reserving the rightmost
/// column for a scrollbar when the content overflows the pane.
fn render_scrolling_pane(frame: &mut Frame, inner: Rect, lines: Vec<Line<'static>>, offset: u16) {
    let total = lines.len() as u16;
    let viewport = inner.height;
    let overflow = total > viewport;
    let [text_area, bar_area] = if overflow {
        Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).areas(inner)
    } else {
        [inner, Rect { width: 0, ..inner }]
    };
    frame.render_widget(
        Paragraph::new(Text::from(lines)).scroll((offset, 0)),
        text_area,
    );
    if overflow {
        let mut sb = ScrollbarState::new(total as usize)
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

/// Append the GitHub PR + contributor sections to the detail pane's left column,
/// with loading / no-data states so an empty section reads as "nothing" rather
/// than "broken."
fn render_remote_sections(
    lines: &mut Vec<Line<'static>>,
    dv: &crate::app::DetailView,
    theme: &Theme,
) {
    // A one-line GitHub summary (open issues + latest release) above the lists.
    if let Some(r) = &dv.remote {
        detail_section(lines, "GitHub".to_string());
        let release = r.latest_release.clone().unwrap_or_else(|| "—".to_string());
        lines.push(Line::from(vec![
            Span::raw(format!("{} open issues", r.open_issues)),
            Span::styled(format!("  ·  latest {release}"), theme.dim()),
        ]));
    }

    detail_section(
        lines,
        match dv.remote.as_ref().map(|r| r.prs.len()) {
            Some(n) => format!("Pull requests ({n})"),
            None => "Pull requests".to_string(),
        },
    );
    match (&dv.remote, dv.remote_pending) {
        (_, true) => lines.push(Line::from(Span::styled("loading…", theme.dim()))),
        (Some(r), _) if r.prs.is_empty() => {
            lines.push(Line::from(Span::styled("none open", theme.dim())));
        }
        (Some(r), _) => {
            for pr in &r.prs {
                let mut spans = vec![
                    Span::styled(format!("#{} ", pr.number), theme.ahead()),
                    Span::raw(pr.title.clone()),
                ];
                if !pr.author.is_empty() {
                    spans.push(Span::styled(format!("  @{}", pr.author), theme.dim()));
                }
                if pr.draft {
                    spans.push(Span::styled("  draft", theme.warn()));
                }
                lines.push(Line::from(spans));
            }
        }
        (None, _) => lines.push(Line::from(Span::styled(
            "needs a GitHub remote + token",
            theme.dim(),
        ))),
    }

    detail_section(
        lines,
        match dv.remote.as_ref().map(|r| r.contributors.len()) {
            Some(n) => format!("Contributors ({n})"),
            None => "Contributors".to_string(),
        },
    );
    match (&dv.remote, dv.remote_pending) {
        (_, true) => lines.push(Line::from(Span::styled("loading…", theme.dim()))),
        (Some(r), _) if r.contributors.is_empty() => {
            lines.push(Line::from(Span::styled("—", theme.dim())));
        }
        (Some(r), _) => {
            for c in &r.contributors {
                lines.push(Line::from(vec![
                    Span::raw(format!("@{}", c.login)),
                    Span::styled(format!("  ×{}", c.contributions), theme.dim()),
                ]));
            }
        }
        (None, _) => {}
    }
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

    // Two boxed panes — a "Repos" list and the focused repo's output — matching
    // the standup view's structure. Side-by-side, stacking on a narrow terminal.
    let (list_area, out_area) = two_pane(inner, 26, 6);

    // Left: the repos box.
    let repos_block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(" Repos ").bold())
        .padding(Padding::horizontal(1));
    let repos_inner = repos_block.inner(list_area);
    frame.render_widget(repos_block, list_area);

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
                Span::raw(ellipsize(&r.name, 16)),
                Span::styled(note, theme.dim()),
            ]))
        })
        .collect();
    let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut list_state = ListState::default();
    if !run.results.is_empty() {
        list_state.select(Some(run.focus));
    }
    frame.render_stateful_widget(list, repos_inner, &mut list_state);

    // Right: the focused repo's output box (stdout, then a dim stderr divider).
    let focused_name = run
        .results
        .get(run.focus)
        .map(|r| r.name.as_str())
        .unwrap_or("");
    let out_block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(format!(" {focused_name} ")).bold())
        .padding(Padding::new(1, 0, 0, 0));
    let out_inner = out_block.inner(out_area);
    frame.render_widget(out_block, out_area);

    let out_lines = run_output_lines(run, theme);
    let total = out_lines.len() as u16;
    let viewport = out_inner.height;
    let max_scroll = total.saturating_sub(viewport);
    run.set_max_scroll(max_scroll);
    let offset = run.scroll.min(max_scroll);

    let [text_area, bar_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).areas(out_inner);
    frame.render_widget(
        Paragraph::new(Text::from(out_lines)).scroll((offset, 0)),
        text_area,
    );
    if max_scroll > 0 {
        let mut sb = ScrollbarState::new(total as usize)
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

/// The "Open with…" picker: a centered list of installed editors plus "Reveal
/// in folder" and lazygit, with the current default marked. `↑/↓` choose, `⏎`
/// opens, `d` sets the highlighted editor as the default.
fn render_open_with(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    let Some(ow) = &app.open_with else {
        return;
    };

    let repo = app.selected_repo().map(|r| r.name.as_str()).unwrap_or("");
    let title = if repo.is_empty() {
        " Open with… ".to_string()
    } else {
        format!(" Open {repo} with… ")
    };

    // When PATH has no editor CLI, the list is just Reveal/lazygit — explain why
    // and how to fix it, in a two-line note above the list.
    let no_editor = !ow
        .openers
        .iter()
        .any(|o| matches!(o, Opener::Editor { .. }));
    let note_h: u16 = if no_editor { 2 } else { 0 };

    // Size to the content: a fixed-ish width, height = items + note + borders.
    let w = 52.min(full.width.saturating_sub(2)).max(20);
    let h = (ow.openers.len() as u16 + note_h + 2)
        .min(full.height.saturating_sub(2))
        .max(3);
    let x = full.x + full.width.saturating_sub(w) / 2;
    let y = full.y + full.height.saturating_sub(h) / 2;
    let area = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, area);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(title).bold())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split off the note row(s) when there are no detected editors.
    let list_area = if no_editor {
        let [note, list] =
            Layout::vertical([Constraint::Length(note_h), Constraint::Min(0)]).areas(inner);
        let lines = vec![
            Line::from(Span::styled(
                "No editor CLI found on your PATH.",
                theme.warn(),
            )),
            Line::from(Span::styled(
                "Install its shell command to list it here.",
                theme.dim(),
            )),
        ];
        frame.render_widget(Paragraph::new(Text::from(lines)), note);
        list
    } else {
        inner
    };

    let items: Vec<ListItem> = ow
        .openers
        .iter()
        .map(|o| match o {
            Opener::Editor { command, label } => {
                let mut spans = vec![Span::raw(label.clone())];
                if ow.default_command.as_deref() == Some(command.as_str()) {
                    spans.push(Span::styled("  · default", theme.ahead()));
                }
                ListItem::new(Line::from(spans))
            }
            Opener::Reveal => ListItem::new(Span::styled("Reveal in file manager", theme.dim())),
            Opener::Lazygit => ListItem::new(Span::styled("Open in lazygit", theme.dim())),
        })
        .collect();
    let list = List::new(items)
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▌ ");
    let mut state = ListState::default();
    state.select(Some(ow.cursor));
    frame.render_stateful_widget(list, list_area, &mut state);
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
            activity: Vec::new(),
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

    /// On a narrow terminal the header collapses to the spider + lede + the
    /// directory only — no taglines, divider, or info column.
    #[test]
    fn snapshot_header_compact() {
        let app = demo_app();
        insta::assert_snapshot!(render_to_string(&app, 60, 16));
    }

    /// With `h` pressed, the hint boxes collapse to just the toggle divider,
    /// which now reads "unhide".
    #[test]
    fn snapshot_hints_hidden() {
        let mut app = demo_app();
        app.hints_hidden = true;
        insta::assert_snapshot!(render_to_string(&app, 100, 20));
    }

    /// Action/scan feedback shows as a self-dismissing toast (bottom-right, above
    /// the footer), not in the header.
    #[test]
    fn snapshot_toast() {
        let mut app = demo_app();
        app.status = Some("pushed 3 repos".to_string());
        insta::assert_snapshot!(render_to_string(&app, 100, 20));
    }

    /// The "Open with…" picker lists detected openers with the default marked.
    #[test]
    fn snapshot_open_with() {
        use crate::app::{OpenWith, Opener};
        let mut app = demo_app();
        app.mode = Mode::OpenWith;
        let openers = vec![
            Opener::Editor {
                command: "code".to_string(),
                label: "VS Code".to_string(),
            },
            Opener::Editor {
                command: "nvim".to_string(),
                label: "Neovim".to_string(),
            },
            Opener::Reveal,
            Opener::Lazygit,
        ];
        app.open_with = Some(OpenWith::new(openers, Some("code".to_string())));
        insta::assert_snapshot!(render_to_string(&app, 100, 20));
    }

    /// When PATH has no editor CLI, the picker shows a note explaining why and
    /// still offers Reveal / lazygit.
    #[test]
    fn snapshot_open_with_no_editor() {
        use crate::app::{OpenWith, Opener};
        let mut app = demo_app();
        app.mode = Mode::OpenWith;
        let openers = vec![Opener::Reveal, Opener::Lazygit];
        app.open_with = Some(OpenWith::new(openers, None));
        insta::assert_snapshot!(render_to_string(&app, 100, 20));
    }

    /// On a compact terminal the footer's group boxes collapse to single columns
    /// (so the keys never clip) instead of falling back to a different layout.
    #[test]
    fn snapshot_footer_compact() {
        let app = demo_app();
        insta::assert_snapshot!(render_to_string(&app, 56, 22));
    }

    /// On a tall terminal the docked context pane appears below the list and
    /// shows the selected repo at a glance: why it wants attention, its
    /// changes/sync, and the last commit.
    #[test]
    fn snapshot_dock_idle() {
        let mut app = demo_app();
        app.selected = 1; // skip the unreadable repo that sorts to the top
        insta::assert_snapshot!(render_to_string(&app, 120, 34));
    }

    /// ASCII icon mode (also the `NO_COLOR` fallback): the synced state renders as
    /// a steady "ok" word instead of a colour-only, blinking `●`, and a marked repo
    /// uses a `*` gutter instead of `●` — the multi-user portability path.
    #[test]
    fn snapshot_ascii_synced() {
        let mut app = App::new(vec!["(demo)".to_string()], "(demo)".to_string());
        app.icons = cohors_config::IconMode::Ascii;
        app.set_repos(vec![snap(
            "in-sync",
            Branch::Named("main".to_string()),
            Some(("origin/main", 0, 0)),
            (0, 0, 0),
            0,
            Some((NOW - 3600, "chore: nothing to do")),
            None,
        )]);
        app.selection.insert(app.repos[0].id.clone()); // marked → `*` gutter
        insta::assert_snapshot!(render_to_string(&app, 100, 34));
    }

    /// While a bulk action runs, the dock shows the in-flight repos with spinners
    /// instead of the cursor's repo.
    #[test]
    fn snapshot_dock_action() {
        let mut app = demo_app();
        app.busy.insert(app.repos[1].id.clone());
        app.busy.insert(app.repos[2].id.clone());
        insta::assert_snapshot!(render_to_string(&app, 100, 34));
    }

    /// A long commit message wraps across the dock's remaining lines instead of
    /// being truncated with an ellipsis.
    #[test]
    fn snapshot_dock_wraps_long_commit() {
        let mut app = App::new(vec!["(demo)".to_string()], "(demo)".to_string());
        app.set_repos(vec![snap(
            "load-tester",
            Branch::Named("main".to_string()),
            Some(("origin/main", 1, 0)),
            (0, 0, 0),
            0,
            Some((
                NOW - 7 * 86_400,
                "chore: add load-test harnesses (mock origin, HTTP load driver, quota-RPC pgbench) + ignore results and refresh the CI perf matrix",
            )),
            None,
        )]);
        insta::assert_snapshot!(render_to_string(&app, 100, 34));
    }

    /// On a window too short for the whole fleet, the list shows a "… N more ↓"
    /// affordance on its bottom border.
    #[test]
    fn snapshot_repos_scroll_affordance() {
        let mut app = App::new(vec!["(demo)".to_string()], "(demo)".to_string());
        app.set_repos(cohors_core::demo::fleet(NOW));
        app.hints_hidden = true; // give the list room so some rows show and some overflow
        // Short height so the 12-repo fleet overflows the viewport.
        insta::assert_snapshot!(render_to_string(&app, 100, 18));
    }

    /// Scrolled to the end of the list, the "more" hint flips to the top
    /// (`↑ N more`), since the hidden repos are now above.
    #[test]
    fn snapshot_repos_scroll_at_bottom() {
        let mut app = App::new(vec!["(demo)".to_string()], "(demo)".to_string());
        app.set_repos(cohors_core::demo::fleet(NOW));
        app.hints_hidden = true;
        app.selected = app.visible_len() - 1; // cursor at the last repo → scrolled down
        insta::assert_snapshot!(render_to_string(&app, 100, 18));
    }

    /// Scrolled to the middle, both hints show at once — `↑ N more` below the
    /// header and `… N more ↓` at the bottom.
    #[test]
    fn snapshot_repos_scroll_middle() {
        let mut app = App::new(vec!["(demo)".to_string()], "(demo)".to_string());
        app.set_repos(cohors_core::demo::fleet(NOW));
        app.hints_hidden = true;
        // Far enough down that some repos are hidden above *and* below.
        app.selected = app.visible_len().saturating_sub(4);
        insta::assert_snapshot!(render_to_string(&app, 100, 18));
    }

    /// Command mode shows a `:` input line in the top strip.
    #[test]
    fn snapshot_command_mode() {
        let mut app = demo_app();
        app.mode = Mode::Command;
        app.command_line = "sort name".to_string();
        insta::assert_snapshot!(render_to_string(&app, 100, 22));
    }

    /// Shell mode (opened with `!`) collapses the sheet to just the shell row.
    #[test]
    fn snapshot_command_mode_shell() {
        let mut app = demo_app();
        app.mode = Mode::Command;
        app.command_line = "!git checkout main".to_string();
        insta::assert_snapshot!(render_to_string(&app, 100, 22));
    }

    /// The drill-in detail pane renders its sections (commits/changes/branches).
    #[test]
    fn snapshot_detail() {
        use crate::app::DetailView;
        let mut app = demo_app();
        app.mode = Mode::Detail;
        let id = app.repos[0].id.clone();
        let mut dv = DetailView::new(id, "payments".to_string());
        dv.detail = Some(cohors_core::demo::detail(NOW));
        app.detail = Some(dv);
        insta::assert_snapshot!(render_to_string(&app, 100, 28));
    }

    /// Detail with GitHub data: the PR + contributor sections populated.
    #[test]
    fn snapshot_detail_with_remote() {
        use crate::app::DetailView;
        use cohors_core::{Contributor, PullRequest, RemoteDetail};
        let mut app = demo_app();
        app.mode = Mode::Detail;
        let id = app.repos[0].id.clone();
        let mut dv = DetailView::new(id, "payments".to_string());
        dv.detail = Some(cohors_core::demo::detail(NOW));
        dv.remote = Some(RemoteDetail {
            prs: vec![
                PullRequest {
                    number: 142,
                    title: "fix: retry charge on 5xx".to_string(),
                    author: "maya".to_string(),
                    draft: false,
                    branch: "fix/retry".to_string(),
                    url: String::new(),
                },
                PullRequest {
                    number: 147,
                    title: "wip: webhook signatures".to_string(),
                    author: "sam".to_string(),
                    draft: true,
                    branch: "spike/webhooks".to_string(),
                    url: String::new(),
                },
            ],
            contributors: vec![
                Contributor {
                    login: "maya".to_string(),
                    contributions: 128,
                },
                Contributor {
                    login: "sam".to_string(),
                    contributions: 64,
                },
            ],
            open_issues: 7,
            latest_release: Some("v1.4.0".to_string()),
        });
        app.detail = Some(dv);
        insta::assert_snapshot!(render_to_string(&app, 100, 28));
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
        // Tall enough that the whole help fits, so the box collapses to its
        // content (no trailing whitespace) rather than being capped + clipped.
        insta::assert_snapshot!(render_to_string(&app, 100, 64));
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
        let mut view = StandupView::new(commits);
        // Focus the commits with the cursor deep in the list, so the highlight
        // and contextual scroll (the list follows the cursor) are exercised.
        view.commits_focused = true;
        view.commit_cursor = 20;
        app.standup = Some(view);
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

    /// Empty fleet, but repos detected elsewhere: the first-run rescue prompt.
    #[test]
    fn snapshot_empty_picker() {
        let mut app = App::new(vec!["~/projects".to_string()], "cfg".to_string());
        app.suggested_roots = vec!["~/code".to_string(), "~/work".to_string()];
        insta::assert_snapshot!(render_to_string(&app, 100, 14));
    }

    #[test]
    fn snapshot_loading() {
        let mut app = App::new(vec!["~/projects".to_string()], "cfg".to_string());
        app.scanning = true;
        insta::assert_snapshot!(render_to_string(&app, 100, 12));
    }
}
