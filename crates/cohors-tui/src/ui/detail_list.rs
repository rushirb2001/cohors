//! The detail view: every repository as a two-line entry in one long list,
//! and under it three cards about the selected one — its state, the signals
//! that make it need attention, and its tasks and run commands.
//!
//! Opened from card 5 of the overview with Enter; `esc` goes back. Enter here
//! opens the full drill-in (commits, branches, changed files), which is the
//! overlay in `modals.rs`.

use cohors_core::RepoLine;
use cohors_core::launchpad::LaunchpadEntry;

use super::cards::{CardFrame, Item, draw_card, draw_lines, draw_list, draw_message, hint};
use super::layout::list_window_tall;
use super::overview::render_key_bar;
use super::repos_table::{
    RepoRow, TableKind, Wanted, columns, empty_message, gutter, header_line, path_line, row_line,
    table_hints, table_title,
};
use super::*;
use crate::app::Pane;

/// Rows the three cards take when the screen has room for them in full.
const CARDS_HEIGHT: u16 = 10;
/// A shorter version for a screen that cannot spare ten rows.
const CARDS_SHORT: u16 = 6;
/// Width of the label column in the state card (`untracked   2`).
const LABEL_WIDTH: usize = 12;

/// Draw the detail view into `area`.
pub(crate) fn render_detail_view(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    now: i64,
    theme: &Theme,
) {
    // The cards are worth their rows only while the list keeps a useful
    // number of its own.
    let cards_height = match area.height {
        h if h >= 26 => CARDS_HEIGHT,
        h if h >= 18 => CARDS_SHORT,
        _ => 0,
    };
    // A blank row between the list and the cards, on a screen tall enough
    // to give one up.
    let gap = u16::from(area.height >= 40);
    let [list, _, cards, keys] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(gap),
        Constraint::Length(cards_height),
        Constraint::Length(1),
    ])
    .areas(area);

    render_list(frame, list, app, now, theme);
    if cards_height > 0 {
        render_repo_cards(frame, cards, app, now, theme);
    }
    render_key_bar(frame, keys, app, theme);
}

/// The long list: a header, then two lines per repository.
fn render_list(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
    let view = app.view();
    let card = CardFrame {
        number: Some(5),
        hue: None,
        title: table_title(app, view.len(), theme),
        hints: table_hints(TableKind::Detail, theme),
        focused: app.pane == Pane::List,
    };
    let inner = draw_card(frame, area, &card, theme);
    if inner.text.height == 0 {
        return;
    }
    let columns = columns(TableKind::Detail, inner.width(), Wanted::of(&app.repos));
    frame.render_widget(
        Paragraph::new(header_line(TableKind::Detail, &columns, theme)),
        Rect {
            height: 1,
            ..inner.text
        },
    );
    let body = inner.below(1);
    if view.is_empty() {
        draw_message(frame, body.text, &empty_message(app), theme);
        return;
    }

    let window = list_window_tall(
        app.scrolls.list.get(),
        app.selected,
        body.text.height as usize,
        2,
        view.len(),
    );
    app.scrolls.list.set(window.offset);
    // The cursor is always shown here: the cards below describe that entry.
    draw_list(frame, body, window, 2, Some(app.selected), theme, |index| {
        let row = RepoRow::new(app, &view[index]);
        Item {
            lines: vec![
                row_line(&row, &columns, now, theme),
                path_line(&row, &columns, inner.width(), theme),
            ],
            gutter: gutter(&row, app, theme),
        }
    });
}

/// Which of the three repository cards are drawn. All three when there is
/// room; fewer on a narrow screen, always including the focused one.
fn visible_panes(width: u16, focus: Pane) -> &'static [Pane] {
    const ALL: [Pane; 3] = [Pane::State, Pane::Signals, Pane::Tasks];
    let fit = match width {
        w if w >= 96 => 3,
        w if w >= 60 => 2,
        _ => 1,
    };
    let focused = ALL.iter().position(|pane| *pane == focus).unwrap_or(0);
    // Slide the window right just far enough to include the focused card.
    let start = (focused + 1).saturating_sub(fit);
    &ALL[start..start + fit]
}

/// The three cards about the selected repository.
fn render_repo_cards(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
    let panes = visible_panes(area.width, app.pane);
    let cells = Layout::horizontal(vec![Constraint::Fill(1); panes.len()])
        .spacing(1)
        .split(area);
    let snap = app.selected_repo();
    let line = snap.and_then(|snap| app.line(&snap.id));
    let entry = snap.and_then(|snap| app.entry(&snap.id));

    for (pane, cell) in panes.iter().zip(cells.iter()) {
        let focused = app.pane == *pane;
        let bold = Style::new().add_modifier(Modifier::BOLD);
        let (title, hints) = match pane {
            Pane::State => (
                vec![
                    Span::styled(snap.map(|s| text::safe(&s.name)).unwrap_or_default(), bold),
                    Span::styled("  state", theme.dim()),
                ],
                vec![hint("d", "changes")],
            ),
            Pane::Signals => (
                vec![Span::styled("signals", bold)],
                vec![hint("f", "fetch")],
            ),
            _ => (
                {
                    let mut title = vec![Span::styled("tasks", bold)];
                    if let Some(line) = line {
                        title.push(Span::styled(format!("  {}", line.tasks_open), theme.dim()));
                    }
                    title
                },
                vec![hint("t", "board"), hint("!", "run"), hint("a", "agent")],
            ),
        };
        let card = CardFrame {
            number: None,
            // The three cards take the colours of the overview cards they
            // expand on: the repository's row, work at risk, open tasks.
            hue: Some(match pane {
                Pane::State => 5,
                Pane::Signals => 2,
                _ => 3,
            }),
            title,
            hints,
            focused,
        };
        let inner = draw_card(frame, *cell, &card, theme).text;
        let width = inner.width as usize;
        let lines = match (pane, snap) {
            (_, None) => vec![Line::from(Span::styled(
                "No repository selected.",
                theme.dim(),
            ))],
            (Pane::State, Some(snap)) => state_lines(snap, now, width, theme),
            (Pane::Signals, Some(snap)) => signal_lines(snap, line, now, width, theme),
            (_, Some(_)) => task_lines(entry, width, theme),
        };
        draw_scrolled(frame, inner, lines, app, focused);
    }
}

/// Draw a card's lines. The focused card scrolls with the arrow keys, so it
/// reports how far it can go; the others always show their top.
fn draw_scrolled(
    frame: &mut Frame,
    area: Rect,
    lines: Vec<Line<'static>>,
    app: &App,
    focused: bool,
) {
    let max = lines.len().saturating_sub(area.height as usize);
    let offset = if focused {
        app.scrolls.pane_max.set(max);
        app.pane_scroll.min(max)
    } else {
        0
    };
    draw_lines(frame, area, lines.into_iter().skip(offset).collect());
}

/// `label       value`, the value clipped to the card.
fn field(label: &str, value: &str, width: usize, theme: &Theme) -> Line<'static> {
    let ell = theme.glyphs.ellipsis();
    Line::from(vec![
        Span::styled(text::cell(label, LABEL_WIDTH, ell), theme.dim()),
        Span::raw(text::clip(
            &text::safe(value),
            width.saturating_sub(LABEL_WIDTH),
            ell,
        )),
    ])
}

/// When the remote was last fetched, in words: `52 days ago`.
pub(crate) fn fetched_words(fetched: Option<i64>, has_remote: bool, now: i64) -> String {
    const DAY: i64 = 24 * 60 * 60;
    if !has_remote {
        return "no remote".to_string();
    }
    match fetched.map(|at| (now - at) / DAY) {
        None => "never".to_string(),
        Some(days) if days < 1 => "today".to_string(),
        Some(1) => "1 day ago".to_string(),
        Some(days) => format!("{days} days ago"),
    }
}

/// The state card: the plain facts about the repository's checkout.
fn state_lines(snap: &RepoSnapshot, now: i64, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    if let Some(error) = &snap.error {
        let mut lines = vec![Line::from(Span::styled(
            "This repository could not be read.",
            theme.risk(),
        ))];
        lines.extend(
            text::wrap(&text::safe(error), width)
                .into_iter()
                .map(|line| Line::from(Span::styled(line, theme.dim()))),
        );
        return lines;
    }
    let upstream = match &snap.upstream {
        Some(up) => up.name.clone(),
        None if snap.remote_url.is_some() => "none (never pushed)".to_string(),
        None => "no remote".to_string(),
    };
    let commit = match &snap.last_commit {
        Some(c) => format!("{}, {} ago", c.short_id, time::relative(c.timestamp, now)),
        None => "none yet".to_string(),
    };
    let number = |label: &str, n: u32| field(label, &n.to_string(), width, theme);
    let mut lines = vec![
        field("branch", &snap.branch.label(), width, theme),
        field("upstream", &upstream, width, theme),
        field(
            "fetched",
            &fetched_words(snap.last_fetch, snap.remote_url.is_some(), now),
            width,
            theme,
        ),
        number("staged", snap.worktree.staged),
        number("modified", snap.worktree.modified),
        number("untracked", snap.worktree.untracked),
        number("stashes", snap.stash_count),
        field("commit", &commit, width, theme),
    ];
    // Conflicts are rare and urgent, so they get a line only when present.
    if snap.worktree.conflicted > 0 {
        lines.insert(6, number("conflicted", snap.worktree.conflicted));
    }
    lines
}

/// The signals card: every reason the repository needs attention, then its
/// last commit and where it lives.
fn signal_lines(
    snap: &RepoSnapshot,
    line: Option<&RepoLine>,
    now: i64,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let ell = theme.glyphs.ellipsis();
    let mut lines = Vec::new();
    match line {
        None => lines.push(Line::from(Span::styled(
            super::overview::computing(theme),
            theme.dim(),
        ))),
        Some(line) if line.reasons.is_empty() => lines.push(Line::from(vec![
            cards::severity_span(Severity::Ok, theme),
            Span::styled(" Nothing needs attention", theme.dim()),
        ])),
        Some(line) => {
            // The overview gives the reasons as words. Their individual
            // severities come from the same core judgment, reason for reason;
            // if the two ever disagree in length, fall back to the row's own.
            let severities: Vec<Severity> = assess(snap, now)
                .reasons
                .iter()
                .map(|reason| reason.severity())
                .collect();
            for (i, reason) in line.reasons.iter().enumerate() {
                let severity = if severities.len() == line.reasons.len() {
                    severities[i]
                } else {
                    line.severity
                };
                lines.push(Line::from(vec![
                    cards::severity_span(severity, theme),
                    Span::raw(" "),
                    Span::raw(text::clip(
                        &text::capitalize(reason),
                        width.saturating_sub(2),
                        ell,
                    )),
                ]));
            }
        }
    }
    let label = |text: &'static str| Line::from(Span::styled(text, theme.dim()));
    if let Some(commit) = &snap.last_commit {
        lines.push(Line::default());
        lines.push(label("last commit"));
        lines.push(Line::from(text::clip(
            &text::safe(&commit.summary),
            width,
            ell,
        )));
    }
    if let Some(path) = &snap.path {
        lines.push(Line::default());
        lines.push(label("path"));
        lines.push(Line::from(text::clip_start(
            &text::safe(&text::abbrev_home(path.as_str())),
            width,
            ell,
        )));
    }
    lines
}

/// The tasks card: the repository's open tasks, then the commands its
/// launchpad file says start it.
fn task_lines(entry: Option<&LaunchpadEntry>, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let ell = theme.glyphs.ellipsis();
    let dim = |text: String| Line::from(Span::styled(text, theme.dim()));
    let Some(entry) = entry.filter(|entry| entry.configured || entry.invalid.is_some()) else {
        return text::wrap(
            "No launchpad file. Press a to have your coding agent read this \
             repository and write .principia/repo.json; tasks appear here \
             once it exists.",
            width,
        )
        .into_iter()
        .map(dim)
        .collect();
    };
    if let Some(problem) = &entry.invalid {
        let mut lines = vec![Line::from(Span::styled(
            "The launchpad file is not valid:",
            theme.risk(),
        ))];
        lines.extend(text::wrap(&text::safe(problem), width).into_iter().map(dim));
        return lines;
    }

    let mut lines = Vec::new();
    let tasks = entry.open_tasks();
    if tasks.is_empty() {
        lines.push(dim("No open tasks.".to_string()));
    }
    for task in tasks {
        // A task title is written by whoever edited the file: treat it as
        // untrusted text.
        let style = if task.priority.as_deref() == Some("high") {
            theme.warn()
        } else {
            theme.dim()
        };
        lines.push(Line::from(vec![
            Span::styled(theme.glyphs.bullet(), style),
            Span::raw(" "),
            Span::raw(text::clip(
                &text::safe(&task.title),
                width.saturating_sub(2),
                ell,
            )),
        ]));
    }

    let declared = entry.contribution.iter().flat_map(|c| c.flows.iter());
    let commands: Vec<String> = declared
        .chain(entry.detected_flows.iter())
        .map(|flow| text::safe(&flow.run))
        .filter(|run| !run.is_empty())
        .collect();
    if !commands.is_empty() {
        lines.push(Line::default());
        lines.push(dim("run".to_string()));
        for command in commands {
            lines.push(Line::from(Span::styled(
                text::clip(&command, width, ell),
                theme.ok(),
            )));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_age_in_words() {
        const DAY: i64 = 86_400;
        let now = 1_700_000_000;
        assert_eq!(
            fetched_words(Some(now - 52 * DAY), true, now),
            "52 days ago"
        );
        assert_eq!(fetched_words(Some(now - DAY), true, now), "1 day ago");
        assert_eq!(fetched_words(Some(now - 60), true, now), "today");
        assert_eq!(fetched_words(None, true, now), "never");
        assert_eq!(fetched_words(Some(now), false, now), "no remote");
    }

    #[test]
    fn narrow_screens_keep_the_focused_card_visible() {
        use Pane::{Signals, State, Tasks};
        assert_eq!(visible_panes(119, Pane::List), [State, Signals, Tasks]);
        assert_eq!(visible_panes(80, Pane::List), [State, Signals]);
        assert_eq!(visible_panes(80, Tasks), [Signals, Tasks]);
        assert_eq!(visible_panes(50, Pane::List), [State]);
        assert_eq!(visible_panes(50, Signals), [Signals]);
        assert_eq!(visible_panes(50, Tasks), [Tasks]);
    }
}
