//! The overview: the screen cohors opens on.
//!
//! Top to bottom: the header card, the six key figures, then five numbered
//! cards — 1 Do next, 2 Work at risk, 3 Open tasks beside 4 Commits, and
//! 5 Repositories — and a one-line key bar.
//!
//! Every number on this screen is read from the shared
//! [`cohors_core::Overview`] model (or, for raw facts in the table, from the
//! repository snapshots). Nothing is counted or judged here; this module only
//! decides where things go and how they are worded.

use cohors_core::{ActionKind, KeyFigures, NextAction};

use super::cards::{
    BLOCK_WIDTH, CardFrame, Hint, Item, draw_card, draw_card_filled, draw_lines, draw_list,
    draw_message, hint, hints_when, severity_block,
};
use super::layout::{Figures, OverviewPlan, Wants, list_window, plan_overview};
use super::repos_table::render_repos_card;
use super::*;
use crate::app::{Card, Screen, action_key};

/// Weeks of history in the header's sparkline.
const SPARK_WEEKS: usize = 12;
/// The widest the header's right-hand block (sparkline and scan time) gets.
const HEADER_SIDE_WIDTH: usize = 39;

/// Draw the overview into `area`.
pub(crate) fn render_overview(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
    let plan = plan_overview(area.width, area.height, &wants(app), app.focus);
    // `plan.gap` is a blank row between one band and the next, when the
    // screen has room for it (zero otherwise, which takes no space).
    let [header, _, figures, _, body, keys] = Layout::vertical([
        Constraint::Length(plan.header),
        Constraint::Length(plan.gap),
        Constraint::Length(plan.figures.height()),
        Constraint::Length(plan.gap),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);

    render_header(frame, header, app, now, theme);
    render_key_bar(frame, keys, app, theme);

    // No repositories at all: one card that says why and what to do about it.
    if app.repos.is_empty() {
        // It takes the rows the figures and the cards would have had.
        let rest = Rect {
            y: figures.y,
            height: figures.height + body.height,
            ..figures
        };
        render_no_repositories(frame, rest, app, theme);
        return;
    }

    render_figures(frame, figures, app, plan.figures, theme);
    render_cards(frame, body, app, now, &plan, theme);
}

/// How many rows each card would like, borders included.
fn wants(app: &App) -> Wants {
    let rows = |n: usize| n.min(u16::MAX as usize - 8) as u16;
    let Some(o) = &app.overview else {
        // Not computed yet: each card shows one line saying so.
        return Wants {
            next: 3,
            risk: 3,
            tasks: 3,
            commits: 3,
            repos: rows(app.visible_len()) + 3,
        };
    };
    let at_risk = o.at_risk().count();
    // Tasks: header, up to five repositories, a blank line and two notes.
    let tasks = if o.work.is_empty() {
        4
    } else {
        1 + rows(o.work.len().min(5)) + 3 + 2
    };
    // Commits: up to seven repositories and one note.
    let commits = rows(o.commits.len().clamp(1, 7)) + 1 + 2;
    Wants {
        next: rows(o.actions.len().max(1)) + 2,
        risk: rows(at_risk.clamp(1, 8)) + 2,
        tasks,
        commits,
        repos: rows(app.visible_len()) + 3,
    }
}

/// Lay the five cards out in the rows the plan gave them.
fn render_cards(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    now: i64,
    plan: &OverviewPlan,
    theme: &Theme,
) {
    let focused = |card: Card| app.screen == Screen::Overview && app.focus == card;
    let middle = if plan.stacked {
        plan.tasks + plan.gap + plan.commits
    } else {
        plan.tasks.max(plan.commits)
    };
    let [next, _, risk, _, mid, _, repos] = Layout::vertical([
        Constraint::Length(plan.next),
        Constraint::Length(plan.gap),
        Constraint::Length(plan.risk),
        Constraint::Length(plan.gap),
        Constraint::Length(middle),
        Constraint::Length(plan.gap),
        Constraint::Length(plan.repos),
    ])
    .areas(area);

    if plan.next > 0 {
        render_do_next(frame, next, app, theme, focused(Card::DoNext));
    }
    if plan.risk > 0 {
        render_at_risk(frame, risk, app, now, theme, focused(Card::AtRisk));
    }
    let (tasks, commits) = split_middle(mid, plan);
    if plan.tasks > 0 {
        render_tasks(frame, tasks, app, theme, focused(Card::Tasks));
    }
    if plan.commits > 0 {
        render_commits(frame, commits, app, theme, focused(Card::Commits));
    }
    if plan.repos > 0 {
        render_repos_card(frame, repos, app, now, theme, focused(Card::Repos));
    }
}

/// Split the middle band between cards 3 and 4: stacked when narrow, else
/// side by side with the tasks card taking about two fifths (50 of 119
/// columns in the design).
fn split_middle(area: Rect, plan: &OverviewPlan) -> (Rect, Rect) {
    if plan.stacked {
        let [tasks, _, commits] = Layout::vertical([
            Constraint::Length(plan.tasks),
            Constraint::Length(plan.gap),
            Constraint::Length(plan.commits),
        ])
        .areas(area);
        return (tasks, commits);
    }
    let tasks_width = ((u32::from(area.width) * 42 + 50) / 100) as u16;
    let tasks_width = tasks_width.clamp(38.min(area.width / 2), 50);
    let [tasks, _, commits] = Layout::horizontal([
        Constraint::Length(tasks_width),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(area);
    (tasks, commits)
}

// ── Header ───────────────────────────────────────────────────────────────────

/// The configured root(s) as a compact, home-abbreviated string. With several
/// roots, lead with the one that holds the most repositories, since that is
/// the "home base" a glance wants to see.
pub(crate) fn header_roots(app: &App) -> String {
    match app.roots.as_slice() {
        [] => "(no roots configured)".to_string(),
        [one] => text::abbrev_home(one),
        roots => {
            // Strict `>` keeps the first configured root on a tie.
            let mut busiest = &roots[0];
            let mut most = 0usize;
            for root in roots {
                let n = app
                    .repos
                    .iter()
                    .filter(|r| {
                        r.path
                            .as_ref()
                            .is_some_and(|p| p.starts_with(root.as_str()))
                    })
                    .count();
                if n > most {
                    most = n;
                    busiest = root;
                }
            }
            format!("{} (+{} more)", text::abbrev_home(busiest), roots.len() - 1)
        }
    }
}

/// The one sentence under the wordmark. It is the overview's own headline;
/// before the overview exists it only says what is known so far.
fn headline(app: &App, theme: &Theme) -> String {
    match &app.overview {
        Some(o) if !app.repos.is_empty() => o.headline.clone(),
        _ if app.scanning => format!("Scanning for repositories{}", theme.glyphs.dots()),
        _ if app.repos.is_empty() => "No repositories found.".to_string(),
        _ => format!(
            "{}. {}",
            text::count(app.repos.len(), "repository", "repositories"),
            computing(theme)
        ),
    }
}

/// `scanned 2m ago`, or a spinner while a scan is running.
fn scan_status(app: &App, now: i64, theme: &Theme) -> String {
    if app.scanning {
        return format!(
            "{} scanning{}",
            theme.glyphs.spinner(app.spinner),
            theme.glyphs.dots()
        );
    }
    match app.scanned_at {
        Some(at) => match time::relative(at, now).as_str() {
            "now" => "scanned just now".to_string(),
            age => format!("scanned {age} ago"),
        },
        None => String::new(),
    }
}

/// The fleet's commits per week as a row of bars, oldest week first, with a
/// space between bars: `▄ ▄ ▆ ▅ ▄ █ ▆ ▃ █ ▇ ▇ ▆`. Empty when there is no
/// history to show.
fn sparkline(activity: &[u32], ramp: &[char; 8]) -> String {
    let start = activity.len().saturating_sub(SPARK_WEEKS);
    let weeks = &activity[start..];
    let max = weeks.iter().copied().max().unwrap_or(0);
    if max == 0 {
        return String::new();
    }
    let bars: Vec<String> = weeks
        .iter()
        .map(|&count| {
            // Scale to the eight levels; any week with commits shows at
            // least the second level, so it is never mistaken for silence.
            let level = (u64::from(count) * 7 + u64::from(max) / 2) / u64::from(max);
            let level = if count > 0 { level.max(1) } else { 0 };
            ramp[level.min(7) as usize].to_string()
        })
        .collect();
    bars.join(" ")
}

fn render_header(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
    if area.height >= 5 {
        render_header_card(frame, area, app, now, theme);
    } else if area.height >= 1 {
        render_header_line(frame, area, app, now, theme);
    }
}

/// The full header: the mark, the wordmark, where cohors is looking, the
/// headline, and on the right the fleet's recent commits and the scan time.
fn render_header_card(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
    let inner = draw_card(frame, area, &CardFrame::default(), theme).text;
    let ell = theme.glyphs.ellipsis();
    let brand = theme.accent().add_modifier(Modifier::BOLD);

    // Right-hand block, when there is room for it beside the text.
    let side_width = if inner.width as usize >= HEADER_SIDE_WIDTH + 50 {
        HEADER_SIDE_WIDTH
    } else {
        0
    };
    let [left, side] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(side_width as u16)])
            .areas(inner);

    // The mark is built from block glyphs, so the plain-text tier leaves it out.
    let mark_width = if theme.glyphs.shows_brand_mark() && left.width >= 40 {
        SPIDER_MARK[0].chars().count() + 3
    } else {
        0
    };
    let text_width = (left.width as usize).saturating_sub(mark_width + 2);
    let texts = [
        vec![
            Span::styled("cohors", brand),
            Span::styled("  your git fleet", theme.dim()),
        ],
        vec![Span::styled(
            text::clip(&header_roots(app), text_width, ell),
            theme.dim(),
        )],
        vec![Span::raw(text::clip(
            &headline(app, theme),
            text_width,
            ell,
        ))],
    ];
    let lines: Vec<Line> = texts
        .into_iter()
        .enumerate()
        .map(|(row, text)| {
            let mut spans = Vec::new();
            if mark_width > 0 {
                spans.push(Span::styled(format!("{}   ", SPIDER_MARK[row]), brand));
            }
            spans.extend(text);
            Line::from(spans)
        })
        .collect();
    draw_lines(frame, left, lines);

    if side_width > 0 {
        draw_lines(frame, side, header_side_lines(app, now, theme));
    }
}

/// The header's right-hand block: a caption, the sparkline, the scan time.
fn header_side_lines(app: &App, now: i64, theme: &Theme) -> Vec<Line<'static>> {
    let bars = app
        .overview
        .as_ref()
        .map(|o| sparkline(&o.activity, theme.glyphs.spark_ramp()))
        .unwrap_or_default();
    let status = Line::from(Span::styled(scan_status(app, now, theme), theme.dim()));
    if bars.is_empty() {
        return vec![Line::default(), Line::default(), status];
    }
    let weeks = app
        .overview
        .as_ref()
        .map_or(0, |o| o.activity.len().min(SPARK_WEEKS));
    // The caption sits over the right end of the bars, as in the design.
    let caption = format!("commits, {weeks} weeks");
    let caption = format!("{caption:>width$}", width = bars.chars().count() + 1);
    vec![
        Line::from(Span::styled(caption, theme.dim())),
        Line::from(Span::styled(bars, theme.accent())),
        status,
    ]
}

/// The header on a short screen: everything that matters, on one line.
fn render_header_line(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
    let ell = theme.glyphs.ellipsis();
    let status = scan_status(app, now, theme);
    let status_width = status.chars().count();
    // One column of margin each side, and two between the text and the status.
    let room = (area.width as usize).saturating_sub(2 + status_width + 2 + 8);
    let rest = format!("{}   {}", header_roots(app), headline(app, theme));
    let line = Line::from(vec![
        Span::styled(" cohors", theme.accent().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(text::clip(&rest, room, ell), theme.dim()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(format!("{status} "), theme.dim())))
            .alignment(Alignment::Right),
        area,
    );
}

// ── Key figures ──────────────────────────────────────────────────────────────

/// One key figure: what it counts, the count, and one line that qualifies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Figure {
    pub label: &'static str,
    /// A shorter label for the one-line summary.
    pub short: &'static str,
    pub value: String,
    pub note: String,
    /// Fill the whole tile in this severity's colour: `Risk` for work that
    /// exists only on this machine, `Warn` for stashes going stale. `None`
    /// leaves the tile neutral, which is what a figure with nothing wrong is.
    pub tone: Option<Severity>,
}

/// The six key figures, worded. Every number is one of the overview's own.
pub(crate) fn figures(f: &KeyFigures) -> [Figure; 6] {
    let repos = |n: usize| text::count(n, "repo", "repos");
    let not_backed_up = match (f.not_backed_up, f.unpublished, f.unpushed_repos) {
        (0, _, _) => "all pushed".to_string(),
        // Every one is a branch that was never pushed.
        (_, _, 0) => "local only".to_string(),
        (_, 0, _) => text::count(f.unpushed_commits as usize, "commit", "commits"),
        (_, unpublished, _) => format!("{unpublished} never pushed"),
    };
    let stashes = match (f.stash_repos, f.stale_stash_repos) {
        (0, _) => "none".to_string(),
        (_, 0) => format!("in {}", repos(f.stash_repos)),
        (all, stale) if all == stale => "all stale".to_string(),
        (_, stale) => format!("{} stale", repos(stale)),
    };
    let uncommitted = if f.dirty_repos == 0 {
        "all committed".to_string()
    } else {
        format!("in {}", repos(f.dirty_repos))
    };
    let stale_fetch = if f.stale_fetch_repos == 0 {
        "all recent".to_string()
    } else {
        format!("oldest {}d", f.oldest_fetch_days)
    };
    let tasks = if f.configured_repos == 0 {
        "no launchpad".to_string()
    } else if f.open_tasks == 0 {
        "none open".to_string()
    } else if f.doing_tasks > 0 {
        format!("{} high, {} doing", f.high_tasks, f.doing_tasks)
    } else {
        format!("{} high", f.high_tasks)
    };
    let figure = |label, short, value: String, note, tone| Figure {
        label,
        short,
        value,
        note,
        tone,
    };
    [
        figure(
            "Not backed up",
            "not backed up",
            f.not_backed_up.to_string(),
            not_backed_up,
            (f.not_backed_up > 0).then_some(Severity::Risk),
        ),
        figure(
            "Stashes",
            "stashes",
            f.stashes.to_string(),
            stashes,
            (f.stale_stash_repos > 0).then_some(Severity::Warn),
        ),
        figure(
            "Uncommitted",
            "uncommitted",
            f.uncommitted_files.to_string(),
            uncommitted,
            None,
        ),
        figure(
            "Stale fetch",
            "stale",
            f.stale_fetch_repos.to_string(),
            stale_fetch,
            None,
        ),
        figure("Open tasks", "tasks", f.open_tasks.to_string(), tasks, None),
        figure(
            "Commits 7d",
            "commits 7d",
            f.commits_week.to_string(),
            format!("prev week {}", f.commits_prev_week),
            None,
        ),
    ]
}

fn render_figures(frame: &mut Frame, area: Rect, app: &App, layout: Figures, theme: &Theme) {
    let all = match &app.overview {
        Some(overview) => figures(&overview.figures),
        // Not counted yet. The cards are drawn anyway, with no number in them,
        // so nothing jumps when the numbers arrive a moment later.
        None => figures(&KeyFigures::default()).map(|figure| Figure {
            value: theme.glyphs.ellipsis().to_string(),
            note: String::new(),
            tone: None,
            ..figure
        }),
    };
    match layout {
        Figures::Cards => render_figure_row(frame, area, &all, theme),
        Figures::Grid => {
            let [top, bottom] =
                Layout::vertical([Constraint::Length(5), Constraint::Length(5)]).areas(area);
            render_figure_row(frame, top, &all[..3], theme);
            render_figure_row(frame, bottom, &all[3..], theme);
        }
        Figures::Line => render_figure_line(frame, area, &all, theme),
    }
}

/// A row of figure cards, equal widths, one column apart.
fn render_figure_row(frame: &mut Frame, area: Rect, figures: &[Figure], theme: &Theme) {
    let cells = Layout::horizontal(vec![Constraint::Fill(1); figures.len()])
        .spacing(1)
        .split(area);
    for (figure, cell) in figures.iter().zip(cells.iter()) {
        // A figure that needs attention is a filled tile: everything inside
        // the frame takes the severity's colour, and the text is drawn in the
        // fill's own contrasting tone (dim text on a coloured ground would be
        // unreadable). A figure with nothing wrong stays neutral.
        let fill = figure.tone.and_then(|tone| theme.fill(tone));
        let ground = fill.and_then(|fill| fill.bg);
        let inner = draw_card_filled(frame, *cell, &CardFrame::default(), theme, ground).text;
        let width = inner.width as usize;
        let ell = theme.glyphs.ellipsis();
        if let Some(fill) = fill {
            let inside = Rect {
                x: cell.x.saturating_add(1),
                y: cell.y.saturating_add(1),
                width: cell.width.saturating_sub(2),
                height: cell.height.saturating_sub(2),
            };
            frame.buffer_mut().set_style(inside, fill);
        }
        let quiet = fill.unwrap_or_else(|| theme.dim());
        let strong = fill.unwrap_or_default().add_modifier(Modifier::BOLD);
        draw_lines(
            frame,
            inner,
            vec![
                Line::from(Span::styled(text::clip(figure.label, width, ell), quiet)),
                Line::from(Span::styled(text::clip(&figure.value, width, ell), strong)),
                Line::from(Span::styled(text::clip(&figure.note, width, ell), quiet)),
            ],
        );
    }
}

/// The figures as one line, for screens with no room for six cards. Only whole
/// figures are shown; the ones that do not fit are left off.
fn render_figure_line(frame: &mut Frame, area: Rect, figures: &[Figure], theme: &Theme) {
    let mut room = (area.width as usize).saturating_sub(2);
    let mut spans = vec![Span::raw(" ")];
    for (i, figure) in figures.iter().enumerate() {
        let gap = if i == 0 { 0 } else { 2 };
        let width = gap + figure.value.chars().count() + 1 + figure.short.chars().count();
        if width > room {
            break;
        }
        room -= width;
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(
            figure.value.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {}", figure.short), theme.dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── Card 1: Do next ──────────────────────────────────────────────────────────

/// What a card says while the overview is still being computed.
pub(crate) fn computing(theme: &Theme) -> String {
    format!("Working out what needs you{}", theme.glyphs.dots())
}

fn render_do_next(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let card = CardFrame {
        number: Some(1),
        hue: None,
        title: vec![Span::styled("Do next", Style::new().bold())],
        hints: hints_when(
            focused,
            app.overview
                .as_ref()
                .map(|o| do_next_hints(&o.actions, theme))
                .unwrap_or_default(),
        ),
        focused,
    };
    let inner = draw_card(frame, area, &card, theme);
    let Some(overview) = &app.overview else {
        draw_message(frame, inner.text, &computing(theme), theme);
        return;
    };
    let actions = &overview.actions;
    if actions.is_empty() {
        draw_message(
            frame,
            inner.text,
            "Nothing needs doing: no work is at risk and nothing is waiting.",
            theme,
        );
        return;
    }

    // The title column is as wide as the longest title, but always leaves
    // room for the reason; on a narrow card the reason is dropped instead.
    // The severity block, the key, and their spacing: ` RISK   P  `.
    const LEAD: usize = BLOCK_WIDTH + 2 + 1 + 2;
    let width = inner.width();
    let longest = actions
        .iter()
        .map(|a| a.title.chars().count())
        .max()
        .unwrap_or(0);
    let show_why = width >= 70;
    let title_width = if show_why {
        (longest + 4).min(width.saturating_sub(LEAD + 24))
    } else {
        width.saturating_sub(LEAD)
    };
    let why_width = width.saturating_sub(LEAD + title_width);

    let window = list_window(
        app.scrolls.next.get(),
        app.cursors.next,
        inner.text.height as usize,
        actions.len(),
    );
    app.scrolls.next.set(window.offset);
    let cursor = focused.then_some(app.cursors.next);
    draw_list(frame, inner, window, 1, cursor, theme, |index| {
        Item::line(do_next_line(&actions[index], title_width, why_width, theme))
    });
}

/// One step: severity mark, the key that does it, what to do, and why.
fn do_next_line(
    action: &NextAction,
    title_width: usize,
    why_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let ell = theme.glyphs.ellipsis();
    // Leave a gap between a clipped title and the reason.
    let title = text::clip(&action.title, title_width.saturating_sub(2), ell);
    let mut spans = vec![
        severity_block(action.severity, theme),
        Span::raw("  "),
        // The step's letter, or a blank where the step is opened with Enter.
        Span::styled(
            action_key(action.kind).map_or(" ".to_string(), |key| key.to_string()),
            theme.accent().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            text::cell(&title, title_width, ell),
            Style::new().add_modifier(Modifier::BOLD),
        ),
    ];
    if why_width > 0 {
        spans.push(Span::styled(
            text::clip(&text::safe(&action.why), why_width, ell),
            theme.dim(),
        ));
    }
    Line::from(spans)
}

// ── Card 2: Work at risk ─────────────────────────────────────────────────────

/// A step's wording on a card's bottom border: short, and in the model's own
/// words where it has some (`publish 7`, `fetch 17`).
fn step_label(action: &NextAction) -> String {
    match action.kind {
        ActionKind::Unreadable => "unreadable".to_string(),
        ActionKind::Resolve => "paused merges".to_string(),
        ActionKind::Reconcile => "diverged".to_string(),
        _ => action.label.to_lowercase(),
    }
}

/// The keys under "Work at risk": every row is a repository, so these are
/// the repository keys, the same ones the table lists.
fn at_risk_hints(theme: &Theme) -> Vec<Hint> {
    vec![
        hint(theme.glyphs.key_enter(), "open"),
        hint("f", "fetch"),
        hint("p", "pull"),
        hint("P", "push"),
        hint("c", "commit"),
        hint("S", "stash"),
        hint("o", "open with"),
    ]
}

/// The keys under "Do next": Enter for the highlighted step, then the letter
/// of each step that has one (a step that only shows you repositories has
/// none; see [`action_key`]).
fn do_next_hints(actions: &[NextAction], theme: &Theme) -> Vec<Hint> {
    let mut hints = vec![hint(theme.glyphs.key_enter(), "do the highlighted step")];
    let mut seen = Vec::new();
    for action in actions {
        let Some(key) = action_key(action.kind) else {
            continue;
        };
        // Push and Publish share a key, and `t` is on the key bar already.
        if action.kind == ActionKind::StartTask || seen.contains(&key) {
            continue;
        }
        seen.push(key);
        hints.push(hint(&key.to_string(), &step_label(action)));
    }
    hints
}

fn render_at_risk(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    now: i64,
    theme: &Theme,
    focused: bool,
) {
    let lines = app.at_risk();
    let mut title = vec![Span::styled("Work at risk", Style::new().bold())];
    if app.overview.is_some() {
        title.push(Span::styled(format!("  {}", lines.len()), theme.dim()));
    }
    let card = CardFrame {
        number: Some(2),
        hue: None,
        title,
        hints: hints_when(focused, at_risk_hints(theme)),
        focused,
    };
    let inner = draw_card(frame, area, &card, theme);
    if app.overview.is_none() {
        draw_message(frame, inner.text, &computing(theme), theme);
        return;
    }
    if lines.is_empty() {
        draw_message(
            frame,
            inner.text,
            "No work is at risk: everything committed here is also on a remote.",
            theme,
        );
        return;
    }

    let longest_step = lines
        .iter()
        .filter_map(|line| line.action.as_ref())
        .map(|action| action.label.chars().count())
        .max()
        .unwrap_or(0);
    let longest_branch = lines
        .iter()
        .map(|line| text::safe(&line.branch).chars().count())
        .max()
        .unwrap_or(0);
    let columns = RiskColumns::for_width(inner.width(), longest_step, longest_branch);
    let window = list_window(
        app.scrolls.risk.get(),
        app.cursors.risk,
        inner.text.height as usize,
        lines.len(),
    );
    app.scrolls.risk.set(window.offset);
    let cursor = focused.then_some(app.cursors.risk);
    draw_list(frame, inner, window, 1, cursor, theme, |index| {
        Item::line(at_risk_line(lines[index], &columns, now, theme))
    });
}

/// Column widths of the "Work at risk" rows. The reasons take what is left,
/// and the branch and the suggested step give way first on a narrow card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RiskColumns {
    pub name: usize,
    pub branch: usize,
    pub reasons: usize,
    pub age: usize,
    /// Zero when the suggested step is not shown.
    pub step: usize,
}

impl RiskColumns {
    /// `longest_step` is the width of the longest suggested-step label among
    /// the rows, so that column is exactly as wide as it needs to be.
    /// `longest_branch` widens the branch column past its designed 15 (up to
    /// 22) when a branch name needs it, so a name never runs into the reasons.
    pub(crate) fn for_width(width: usize, longest_step: usize, longest_branch: usize) -> Self {
        const MARK: usize = BLOCK_WIDTH + 1; // the severity block and a space
        const AGE: usize = 5;
        let name = if width >= 100 { 22 } else { 18 }.min(width / 3);
        let branch = if width >= 90 {
            (longest_branch + 2).clamp(15, 22)
        } else {
            0
        };
        // Two columns of gap, then the label.
        let step = if width >= 64 && longest_step > 0 {
            2 + longest_step
        } else {
            0
        };
        Self {
            name,
            branch,
            reasons: width.saturating_sub(MARK + name + branch + AGE + step),
            age: AGE,
            step,
        }
    }
}

/// One repository at risk: mark, name, branch, every reason, the age of its
/// last commit, and the step the overview suggests.
fn at_risk_line(
    line: &cohors_core::RepoLine,
    columns: &RiskColumns,
    now: i64,
    theme: &Theme,
) -> Line<'static> {
    let ell = theme.glyphs.ellipsis();
    let reasons = text::capitalize(&line.reasons.join(", "));
    let age = line
        .commit
        .as_ref()
        .map(|c| time::relative(c.timestamp, now))
        .unwrap_or_default();
    let mut spans = vec![
        severity_block(line.severity, theme),
        Span::raw(" "),
        Span::styled(
            text::column(&text::safe(&line.name), columns.name, ell),
            Style::new().add_modifier(Modifier::BOLD),
        ),
    ];
    if columns.branch > 0 {
        spans.push(Span::raw(text::column(
            &text::safe(&line.branch),
            columns.branch,
            ell,
        )));
    }
    // One column short, so clipped reasons never touch the age.
    let reasons = text::clip(&reasons, columns.reasons.saturating_sub(1), ell);
    spans.push(Span::styled(
        text::cell(&reasons, columns.reasons, ell),
        theme.dim(),
    ));
    spans.push(Span::styled(
        text::cell_right(&age, columns.age, ell),
        theme.dim(),
    ));
    if columns.step > 0 {
        let label = line.action.as_ref().map_or("", |a| a.label.as_str());
        spans.push(Span::styled(
            format!("  {}", text::cell(label, columns.step - 2, ell)),
            theme.accent(),
        ));
    }
    Line::from(spans)
}

// ── Card 3: Open tasks ───────────────────────────────────────────────────────

fn render_tasks(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let mut title = vec![Span::styled("Open tasks", Style::new().bold())];
    if let Some(o) = &app.overview {
        title.push(Span::styled(
            format!("  {}", o.figures.open_tasks),
            theme.dim(),
        ));
    }
    let card = CardFrame {
        number: Some(3),
        hue: None,
        title,
        hints: hints_when(
            focused,
            vec![hint("t", "board"), hint(theme.glyphs.key_enter(), "tasks")],
        ),
        focused,
    };
    let inner = draw_card(frame, area, &card, theme);
    let Some(overview) = &app.overview else {
        draw_message(frame, inner.text, &computing(theme), theme);
        return;
    };
    let f = &overview.figures;
    if overview.work.is_empty() {
        let message = if f.configured_repos == 0 {
            "No launchpad files yet. Tasks appear here once a repository has \
             .cohors/repo.json."
                .to_string()
        } else {
            format!(
                "No open tasks in the {} that have a launchpad file.",
                text::count(f.configured_repos, "repository", "repositories")
            )
        };
        draw_message(frame, inner.text, &message, theme);
        return;
    }

    let ell = theme.glyphs.ellipsis();
    // open (4), high (6), focus (7), then three columns of margin.
    const NUMBERS: usize = 4 + 6 + 7 + 3;
    let name_width = inner.width().saturating_sub(NUMBERS);
    let header = format!(
        "{}{:>4}{:>6}{:>7}",
        text::cell("repository", name_width, ell),
        "open",
        "high",
        "focus"
    );
    frame.render_widget(
        Paragraph::new(Span::styled(header, theme.dim())),
        Rect {
            height: 1,
            ..inner.text
        },
    );

    // The two notes (and the blank line above them) are shown when there is
    // room for them under at least a few rows.
    let notes = task_notes(f);
    let body = inner.below(1);
    let body_height = body.text.height as usize;
    let rows_wanted = overview.work.len().min(3);
    let notes_height = if body_height >= rows_wanted + 1 + notes.len() {
        1 + notes.len()
    } else {
        0
    };
    let capacity = body_height - notes_height;
    let window = list_window(
        app.scrolls.tasks.get(),
        app.cursors.tasks,
        capacity,
        overview.work.len(),
    );
    app.scrolls.tasks.set(window.offset);
    let cursor = focused.then_some(app.cursors.tasks);
    let list_area = cards::CardArea {
        gutter_x: body.gutter_x,
        text: Rect {
            height: capacity as u16,
            ..body.text
        },
    };
    draw_list(frame, list_area, window, 1, cursor, theme, |index| {
        let work = &overview.work[index];
        Item::line(Line::from(vec![
            Span::raw(text::column(&text::safe(&work.name), name_width, ell)),
            Span::raw(format!("{:>4}", work.open)),
            Span::styled(
                format!("{:>6}", work.high),
                if work.high > 0 {
                    theme.warn()
                } else {
                    theme.dim()
                },
            ),
            Span::styled(format!("{:>7}", work.focus), theme.dim()),
        ]))
    });

    if notes_height > 0 {
        let lines = notes
            .into_iter()
            .map(|note| {
                Line::from(Span::styled(
                    text::clip(&note, inner.width(), ell),
                    theme.dim(),
                ))
            })
            .collect();
        let area = Rect {
            y: inner.text.y + inner.text.height - (notes_height as u16 - 1),
            height: notes_height as u16 - 1,
            ..inner.text
        };
        draw_lines(frame, area, lines);
    }
}

/// The lines under the task table: what is in progress, and how much of the
/// fleet has no launchpad file at all.
pub(crate) fn task_notes(f: &KeyFigures) -> Vec<String> {
    let mut notes = Vec::new();
    notes.push(match (f.doing_tasks, f.blocked_tasks) {
        (0, 0) => "Nothing is marked in progress.".to_string(),
        (0, blocked) => format!("Nothing is in progress, {blocked} blocked."),
        (doing, 0) => format!("{doing} in progress."),
        (doing, blocked) => format!("{doing} in progress, {blocked} blocked."),
    });
    let unset = f.repos.saturating_sub(f.configured_repos);
    if unset > 0 {
        notes.push(format!(
            "{} no launchpad file.",
            match unset {
                1 => "1 repository has".to_string(),
                n => format!("{n} repositories have"),
            }
        ));
    }
    notes
}

// ── Card 4: Commits, last 7 days ─────────────────────────────────────────────

/// How many cells of a `width`-cell bar `count` fills when `max` fills it all.
/// Anything with commits shows at least one cell.
pub(crate) fn bar_cells(count: u32, max: u32, width: usize) -> usize {
    if max == 0 || count == 0 || width == 0 {
        return 0;
    }
    let cells = (u64::from(count) * width as u64 + u64::from(max) / 2) / u64::from(max);
    (cells as usize).clamp(1, width)
}

fn render_commits(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, focused: bool) {
    let mut title = vec![Span::styled("Commits, last 7 days", Style::new().bold())];
    if let Some(o) = &app.overview {
        title.push(Span::styled(
            format!("  {}", o.figures.commits_week),
            theme.dim(),
        ));
    }
    let card = CardFrame {
        number: Some(4),
        hue: None,
        title,
        hints: hints_when(
            focused,
            vec![
                hint(theme.glyphs.key_enter(), "its commits"),
                hint("Y", "copy the standup as markdown"),
            ],
        ),
        focused,
    };
    let inner = draw_card(frame, area, &card, theme);
    let Some(overview) = &app.overview else {
        draw_message(frame, inner.text, &computing(theme), theme);
        return;
    };
    if overview.commits.is_empty() {
        draw_message(frame, inner.text, "No commits in the last 7 days.", theme);
        return;
    }

    let ell = theme.glyphs.ellipsis();
    let width = inner.width();
    // name, bar, count (7 wide), one column of margin. The bar needs a few
    // cells to mean anything; without them the name takes the room.
    const COUNT: usize = 7;
    const MARGIN: usize = 1;
    let name_width = 22.min(width / 2);
    let bar_width = width.saturating_sub(name_width + COUNT + MARGIN);
    let (name_width, bar_width) = if bar_width >= 6 {
        (name_width, bar_width)
    } else {
        (width.saturating_sub(COUNT + MARGIN), 0)
    };
    let max = overview.commits.iter().map(|c| c.count).max().unwrap_or(0);

    let quiet = overview
        .figures
        .repos
        .saturating_sub(overview.figures.active_repos);
    let note = (quiet > 0).then(|| {
        format!(
            "{} no commits this week.",
            match quiet {
                1 => "1 repository had".to_string(),
                n => format!("{n} repositories had"),
            }
        )
    });
    let height = inner.text.height as usize;
    // The note takes the last line when there is more than one line to share.
    let note_height = usize::from(note.is_some() && height >= 2);
    let capacity = height - note_height;
    let window = list_window(
        app.scrolls.commits.get(),
        app.cursors.commits,
        capacity,
        overview.commits.len(),
    );
    app.scrolls.commits.set(window.offset);
    let cursor = focused.then_some(app.cursors.commits);
    let list_area = cards::CardArea {
        gutter_x: inner.gutter_x,
        text: Rect {
            height: capacity as u16,
            ..inner.text
        },
    };
    draw_list(frame, list_area, window, 1, cursor, theme, |index| {
        let repo = &overview.commits[index];
        let filled = bar_cells(repo.count, max, bar_width);
        Item::line(Line::from(vec![
            Span::raw(text::column(&text::safe(&repo.name), name_width, ell)),
            // The bars take the commits card's own colour.
            Span::styled(theme.glyphs.bar_cell().repeat(filled), theme.section(4)),
            Span::raw(" ".repeat(bar_width - filled)),
            Span::raw(format!("{:>COUNT$}", repo.count)),
        ]))
    });

    if let Some(note) = note.filter(|_| note_height > 0) {
        let area = Rect {
            y: inner.text.y + inner.text.height - 1,
            height: 1,
            ..inner.text
        };
        draw_lines(
            frame,
            area,
            vec![Line::from(Span::styled(
                text::clip(&note, width, ell),
                theme.dim(),
            ))],
        );
    }
}

// ── No repositories ──────────────────────────────────────────────────────────

/// The whole body when the fleet is empty: scanning, or what to do next.
fn render_no_repositories(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if area.height < 3 {
        return;
    }
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let title = if app.scanning {
        "Scanning"
    } else {
        "No repositories found"
    };
    let card = CardFrame {
        number: None,
        hue: None,
        title: vec![Span::styled(title, bold)],
        hints: Vec::new(),
        focused: false,
    };
    let inner = draw_card(frame, area, &card, theme).text;
    let roots = if app.roots.is_empty() {
        "(no roots configured)".to_string()
    } else {
        app.roots
            .iter()
            .map(|root| text::abbrev_home(root))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if app.scanning {
        let line = format!(
            "{} Looking for git repositories under {roots}{}",
            theme.glyphs.spinner(app.spinner),
            theme.glyphs.dots()
        );
        draw_message(frame, inner, &line, theme);
        return;
    }

    let dim = theme.dim();
    let mut lines = vec![
        Line::from(format!("cohors found no git repositories under {roots}.")),
        Line::default(),
    ];
    // Repositories were detected elsewhere: adopting them is one key away.
    if app.empty_picker_active() {
        lines.push(Line::from(format!(
            "It did find some under: {}",
            app.suggested_roots.join(", ")
        )));
        lines.push(Line::from(vec![
            Span::styled("  u", theme.accent().add_modifier(Modifier::BOLD)),
            Span::raw("  use these folders (adds them to your config)"),
        ]));
        lines.push(Line::default());
    }
    lines.push(Line::from("To point cohors at your projects:"));
    lines.push(Line::from(vec![
        Span::styled("  cohors --root ~/code", bold),
        Span::styled("   look in one folder, this run only", dim),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  or add the folder to "),
        Span::styled("roots", bold),
        Span::raw(format!(" in {}", app.config_path)),
    ]));
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("  r", theme.accent().add_modifier(Modifier::BOLD)),
        Span::raw("  scan again"),
        Span::styled("      q", theme.accent().add_modifier(Modifier::BOLD)),
        Span::raw("  quit"),
    ]));
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner,
    );
}

// ── Key bar ──────────────────────────────────────────────────────────────────

/// The keys that work everywhere on the current screen, on the bottom line.
/// A status message (an action's result) takes the right end of the same line.
pub(crate) fn render_key_bar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if area.height == 0 {
        return;
    }
    let toast = toast_line(app, theme);
    let toast_width = toast.as_ref().map_or(0, |t| t.width());
    // The hints get what the status message leaves free, less a left margin.
    let room = (area.width as usize).saturating_sub(2 + toast_width);
    let (hints, gap) = fit_key_bar(key_bar_hints(app, theme), room);

    let mut spans = vec![Span::raw("  ")];
    for (i, hint) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" ".repeat(gap)));
        }
        spans.push(Span::styled(
            hint.key.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!("  {}", hint.label), theme.dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    if let Some(toast) = toast {
        frame.render_widget(Paragraph::new(toast).alignment(Alignment::Right), area);
    }
}

/// Columns a row of hints takes with `gap` spaces between them.
fn key_bar_width(hints: &[Hint], gap: usize) -> usize {
    let text: usize = hints
        .iter()
        .map(|hint| hint.key.chars().count() + 2 + hint.label.chars().count())
        .sum();
    text + gap * hints.len().saturating_sub(1)
}

/// Choose which key-bar hints to show in `room` columns, and how far apart.
///
/// The design spaces them four columns apart. On a narrower screen they first
/// move closer together, and then hints are left off from the right. Help and
/// quit are never left off, so even the narrowest screen says how to find
/// every other key and how to leave.
fn fit_key_bar(mut hints: Vec<Hint>, room: usize) -> (Vec<Hint>, usize) {
    const KEEP_LAST: [&str; 2] = ["help", "quit"];
    if key_bar_width(&hints, 4) <= room {
        return (hints, 4);
    }
    while key_bar_width(&hints, 2) > room {
        // Drop the last hint that is not one of the two to keep.
        let droppable = hints
            .iter()
            .rposition(|hint| !KEEP_LAST.contains(&hint.label.as_str()));
        match droppable {
            Some(index) => {
                hints.remove(index);
            }
            None => break, // only help and quit are left; the line clips
        }
    }
    (hints, 2)
}

fn key_bar_hints(app: &App, theme: &Theme) -> Vec<Hint> {
    let g = &theme.glyphs;
    if app.mode == Mode::Filter {
        return vec![
            hint("type", "filter by name"),
            hint(g.key_enter(), "keep"),
            hint("esc", "clear"),
        ];
    }
    match app.screen {
        Screen::Overview => vec![
            hint("1-5", "card"),
            hint("tab", "next"),
            hint(g.key_updown(), "move"),
            hint("s", "standup"),
            hint("t", "tasks"),
            hint(":", "command"),
            hint("r", "rescan"),
            hint("?", "help"),
            hint("q", "quit"),
        ],
        Screen::Repos => vec![
            hint(g.key_updown(), "move"),
            hint(g.key_enter(), "full detail"),
            hint("tab", "next card"),
            hint(":", "command"),
            hint("?", "help"),
            hint("q", "quit"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAMP: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

    #[test]
    fn the_key_bar_closes_up_then_drops_hints_but_never_help_or_quit() {
        let all = || {
            vec![
                hint("1-5", "focus card"),
                hint("tab", "next card"),
                hint("↑↓", "move"),
                hint(":", "command"),
                hint("r", "rescan"),
                hint("?", "help"),
                hint("q", "quit"),
            ]
        };
        let labels = |hints: &[Hint]| -> Vec<String> {
            hints.iter().map(|hint| hint.label.clone()).collect()
        };

        // The designed width: every hint, four columns apart.
        let (hints, gap) = fit_key_bar(all(), 117);
        assert_eq!((hints.len(), gap), (7, 4));

        // 80 columns: closer together, and "rescan" gives way first.
        let (hints, gap) = fit_key_bar(all(), 78);
        assert_eq!(gap, 2);
        assert_eq!(
            labels(&hints),
            ["focus card", "next card", "move", "command", "help", "quit"]
        );
        assert!(key_bar_width(&hints, gap) <= 78);

        // Very narrow: help and quit are the last two standing.
        let (hints, _) = fit_key_bar(all(), 20);
        assert_eq!(labels(&hints), ["help", "quit"]);
        // Narrower than even those: no panic, the line is simply clipped.
        assert_eq!(fit_key_bar(all(), 0).0.len(), 2);
    }

    #[test]
    fn the_sparkline_scales_to_its_tallest_week() {
        assert_eq!(sparkline(&[0, 5, 10], &RAMP), "▁ ▅ █");
        // Only the most recent twelve weeks are drawn.
        let long: Vec<u32> = (1..=20).collect();
        assert_eq!(sparkline(&long, &RAMP).split(' ').count(), SPARK_WEEKS);
        // A week with a single commit is still visibly not empty.
        assert_eq!(sparkline(&[1, 1000], &RAMP), "▂ █");
        // No history, or no commits at all: nothing to draw.
        assert_eq!(sparkline(&[], &RAMP), "");
        assert_eq!(sparkline(&[0, 0, 0], &RAMP), "");
    }

    #[test]
    fn bars_are_proportional_and_never_vanish() {
        assert_eq!(bar_cells(88, 88, 34), 34);
        assert_eq!(bar_cells(48, 88, 34), 19);
        assert_eq!(bar_cells(2, 88, 34), 1);
        assert_eq!(bar_cells(1, 10_000, 34), 1); // tiny, but there
        assert_eq!(bar_cells(0, 88, 34), 0);
        assert_eq!(bar_cells(5, 0, 34), 0); // no division by zero
        assert_eq!(bar_cells(5, 5, 0), 0);
    }

    #[test]
    fn figures_are_worded_from_the_overview_numbers() {
        // The design's fleet: seven never-pushed branches, six stashes all
        // stale, 128 files in 25 repositories, 17 stale fetches.
        let f = KeyFigures {
            repos: 28,
            not_backed_up: 7,
            unpublished: 7,
            stashes: 6,
            stash_repos: 6,
            stale_stash_repos: 6,
            uncommitted_files: 128,
            dirty_repos: 25,
            stale_fetch_repos: 17,
            oldest_fetch_days: 125,
            open_tasks: 19,
            high_tasks: 9,
            configured_repos: 3,
            commits_week: 200,
            commits_prev_week: 222,
            ..Default::default()
        };
        let got: Vec<(String, String)> = figures(&f)
            .into_iter()
            .map(|figure| (figure.value, figure.note))
            .collect();
        let want = [
            ("7", "local only"),
            ("6", "all stale"),
            ("128", "in 25 repos"),
            ("17", "oldest 125d"),
            ("19", "9 high"),
            ("200", "prev week 222"),
        ];
        for ((value, note), (want_value, want_note)) in got.iter().zip(want) {
            assert_eq!((value.as_str(), note.as_str()), (want_value, want_note));
        }
    }

    #[test]
    fn figures_say_so_when_there_is_nothing_to_count() {
        let notes: Vec<String> = figures(&KeyFigures::default())
            .into_iter()
            .map(|figure| figure.note)
            .collect();
        assert_eq!(
            notes,
            [
                "all pushed",
                "none",
                "all committed",
                "all recent",
                "no launchpad",
                "prev week 0"
            ]
        );
    }

    #[test]
    fn mixed_figures_name_the_part_that_matters() {
        let f = KeyFigures {
            not_backed_up: 5,
            unpublished: 2,
            unpushed_repos: 3,
            unpushed_commits: 9,
            stash_repos: 4,
            stale_stash_repos: 1,
            configured_repos: 2,
            open_tasks: 6,
            high_tasks: 2,
            doing_tasks: 1,
            ..Default::default()
        };
        let all = figures(&f);
        assert_eq!(all[0].note, "2 never pushed");
        assert_eq!(all[1].note, "1 repo stale");
        assert_eq!(all[4].note, "2 high, 1 doing");

        // Only unpushed commits, no unpublished branch.
        let pushed_branches = KeyFigures {
            not_backed_up: 3,
            unpushed_repos: 3,
            unpushed_commits: 9,
            ..Default::default()
        };
        assert_eq!(figures(&pushed_branches)[0].note, "9 commits");
    }

    #[test]
    fn task_notes_cover_progress_and_missing_files() {
        let f = KeyFigures {
            repos: 28,
            configured_repos: 3,
            ..Default::default()
        };
        assert_eq!(
            task_notes(&f),
            [
                "Nothing is marked in progress.",
                "25 repositories have no launchpad file."
            ]
        );
        let busy = KeyFigures {
            repos: 3,
            configured_repos: 2,
            doing_tasks: 2,
            blocked_tasks: 1,
            ..Default::default()
        };
        assert_eq!(
            task_notes(&busy),
            [
                "2 in progress, 1 blocked.",
                "1 repository has no launchpad file."
            ]
        );
        // Every repository set up: only the progress line.
        let all_set = KeyFigures {
            repos: 2,
            configured_repos: 2,
            ..Default::default()
        };
        assert_eq!(task_notes(&all_set).len(), 1);
    }

    #[test]
    fn at_risk_columns_give_the_reasons_whatever_is_left() {
        // The designed width, with "Publish branch" (14) as the longest step:
        // the severity block and its space (7), name 22, branch 15, and 50
        // columns of reasons, as drawn.
        let wide = RiskColumns::for_width(115, 14, 13);
        assert_eq!((wide.name, wide.branch, wide.age), (22, 15, 5));
        assert_eq!((wide.reasons, wide.step), (50, 16));
        assert_eq!(
            BLOCK_WIDTH + 1 + wide.name + wide.branch + wide.reasons + wide.age + wide.step,
            115
        );
        // Narrow: the branch goes first, then the suggested step.
        // A long branch name gets the room it needs, within reason.
        assert_eq!(RiskColumns::for_width(115, 14, 14).branch, 16);
        assert_eq!(RiskColumns::for_width(115, 14, 40).branch, 22);
        let medium = RiskColumns::for_width(76, 14, 13);
        assert_eq!(medium.branch, 0);
        assert!(medium.step > 0);
        let narrow = RiskColumns::for_width(44, 14, 13);
        assert_eq!((narrow.branch, narrow.step), (0, 0));
        assert!(narrow.reasons >= 16);
        // No row suggests a step: the column is not there at all.
        assert_eq!(RiskColumns::for_width(115, 0, 13).step, 0);
    }
}
