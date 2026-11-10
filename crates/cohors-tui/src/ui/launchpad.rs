//! The launchpad overlay (`t`): what is waiting across the fleet.
//!
//! One scrolling column in the shared framed-overlay language: the focus board
//! first, then each repo that has open tasks (most urgent first, with the flow
//! you would start it with), then the repos that have not opted in yet.

use cohors_core::launchpad::Task;

use super::*;
// Repo-authored text (a task title, a summary) is untrusted input: control
// characters are dropped before it reaches the terminal buffer (ADR-055).
use super::text::safe;

/// The status mark. Shape, not colour, carries the meaning, so it still reads
/// under `NO_COLOR`.
fn mark(task: &Task) -> &'static str {
    match task.status() {
        "doing" => "◆",
        "blocked" => "✕",
        _ => "○",
    }
}

fn task_line<'a>(task: &Task, theme: &Theme) -> Line<'a> {
    let mark_style = match task.status() {
        "doing" => theme.ahead(),
        "blocked" => theme.risk(),
        _ => theme.dim(),
    };
    let mut spans = vec![
        Span::raw("    "),
        Span::styled(mark(task), mark_style),
        Span::raw(" "),
        Span::raw(safe(&task.title)),
    ];
    match task.priority.as_deref() {
        Some("high") => {
            spans.push(Span::raw("  "));
            match theme.badge(Severity::Warn) {
                Some((_, badge)) => spans.push(Span::styled(" high ", badge)),
                None => spans.push(Span::styled("high", theme.warn().bold())),
            }
        }
        Some("low") => spans.push(Span::styled("  low", theme.dim())),
        _ => {}
    }
    if task.status() != "todo" {
        spans.push(Span::styled(
            format!("  {}", safe(task.status())),
            theme.dim(),
        ));
    }
    Line::from(spans)
}

/// The line a repository's tasks start on, so the overlay can open scrolled to
/// it (Enter on a repository in the overview's tasks card). `None` when that
/// repository has no open tasks to show.
pub(crate) fn launchpad_line_of(view: &crate::app::LaunchpadView, repo: &str) -> Option<usize> {
    // The line numbers do not depend on colours or glyphs, so any theme will do.
    let theme = Theme::new(cohors_config::IconMode::Unicode, false, false);
    let (_, anchors) = body(view, &theme);
    anchors
        .into_iter()
        .find(|(name, _)| name == repo)
        .map(|(_, line)| line)
}

/// Every line of the body, built up front so the overlay can size and scroll.
fn body_lines<'a>(view: &crate::app::LaunchpadView, theme: &Theme) -> Vec<Line<'a>> {
    body(view, theme).0
}

/// The body's lines, plus where each repository's block starts (its name and
/// the index of its heading line).
fn body<'a>(
    view: &crate::app::LaunchpadView,
    theme: &Theme,
) -> (Vec<Line<'a>>, Vec<(String, usize)>) {
    let brand = theme.fg(SPIDER_PURPLE).bold();
    let heading = theme.bronze().bold();
    let mut lines: Vec<Line> = Vec::new();
    let mut anchors: Vec<(String, usize)> = Vec::new();

    let focus: Vec<_> = view
        .board
        .focus
        .iter()
        .filter(|f| f.status.as_deref() != Some("done"))
        .collect();
    if !focus.is_empty() {
        lines.push(Line::from(Span::styled("Focus", heading)));
        for item in focus {
            lines.push(Line::from(vec![
                Span::styled("  ▸ ", theme.dim()),
                Span::styled(safe(&item.title), Style::new().bold()),
                Span::styled(
                    format!(
                        "  {} · {}",
                        safe(item.repo.as_deref().unwrap_or("—")),
                        safe(item.status.as_deref().unwrap_or("todo"))
                    ),
                    theme.dim(),
                ),
            ]));
        }
        lines.push(Line::default());
    }

    for entry in view.entries.iter().filter(|e| !e.open_tasks().is_empty()) {
        anchors.push((entry.name.clone(), lines.len()));
        let mut head = vec![
            Span::styled("  ▸ ", theme.dim()),
            Span::styled(safe(&entry.name), brand),
        ];
        if let Some(summary) = entry.contribution.as_ref().map(|c| c.summary.as_str())
            && !summary.is_empty()
        {
            head.push(Span::styled(format!("  {}", safe(summary)), theme.dim()));
        }
        lines.push(Line::from(head));
        for task in entry.open_tasks() {
            lines.push(task_line(task, theme));
        }
        if let Some(flow) = entry.lead_flow() {
            lines.push(Line::from(vec![
                Span::styled("    run ", theme.dim()),
                Span::styled(safe(&flow.run), theme.ok()),
            ]));
        }
        lines.push(Line::default());
    }

    for entry in view.entries.iter().filter(|e| e.invalid.is_some()) {
        lines.push(Line::from(vec![
            Span::styled("  ! ", theme.risk()),
            Span::styled(safe(&entry.name), Style::new().bold()),
            Span::styled(
                format!("  {}", safe(entry.invalid.as_deref().unwrap_or(""))),
                theme.risk(),
            ),
        ]));
    }

    let unset: Vec<String> = view
        .entries
        .iter()
        .filter(|e| !e.configured && e.invalid.is_none())
        .map(|e| safe(&e.name))
        .collect();
    if !unset.is_empty() {
        lines.push(Line::from(Span::styled("Not set up yet", heading)));
        lines.push(Line::from(Span::styled(
            "  in one of these, ask your agent: \"set up this repo for the launchpad\"",
            theme.dim(),
        )));
        lines.push(Line::from(Span::styled(
            format!("  {}", unset.join(" · ")),
            theme.dim(),
        )));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "Nothing waiting: no focus items and no open tasks.",
            theme.dim(),
        )));
    }
    (lines, anchors)
}

pub(crate) fn render_launchpad(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    let lines = app
        .launchpad
        .as_ref()
        .map(|view| body_lines(view, theme))
        .unwrap_or_default();
    // Content height + title(1) + header rule(1) + hints rule(1) + hints(1) +
    // borders(2), capped to the screen; anything past the cap scrolls.
    let wanted = lines.len().clamp(3, 40) as u16 + 6;
    let h = wanted.min(full.height.saturating_sub(2)).max(9);
    let w = (full.width as u32 * 84 / 100) as u16;
    let area = Rect {
        x: full.x + full.width.saturating_sub(w) / 2,
        y: full.y + full.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, area);
    let outer = Block::bordered()
        .border_type(FRAME)
        .padding(Padding::new(1, 1, 0, 0));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let Some(view) = &app.launchpad else {
        let inner = render_modal_header(frame, area, inner, theme, Line::from("Launchpad").bold());
        frame.render_widget(
            Paragraph::new(Span::styled(
                "Reading each repo's launchpad file…",
                theme.dim(),
            )),
            inner,
        );
        return;
    };

    let configured = view.entries.iter().filter(|e| e.configured).count();
    let open: usize = view.entries.iter().map(|e| e.open_tasks().len()).sum();
    let header = Line::from(vec![
        Span::styled("Launchpad", Style::new().bold()),
        Span::styled(
            format!(
                "   ·   {open} open task{} · {configured} of {} repos set up",
                if open == 1 { "" } else { "s" },
                view.entries.len()
            ),
            theme.dim(),
        ),
    ]);
    let inner = render_modal_header(frame, area, inner, theme, header);
    let inner = render_modal_hints(frame, area, inner, app, theme);

    // Tell the key handler how far it may scroll, then clamp what we draw.
    let max_scroll = lines.len().saturating_sub(inner.height as usize);
    view.max_scroll.set(max_scroll);
    let scroll = view.scroll.min(max_scroll) as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        inner,
    );
}
