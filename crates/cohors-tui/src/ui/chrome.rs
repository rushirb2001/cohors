//! The chrome the overlays share: the title row and key-hint row drawn inside
//! every overlay's frame, the command line, and the status message.

use super::*;

/// The status message: the latest action's result (fetch, pull, push, stash,
/// copy…), shown at the right end of the key bar. `None` when there is nothing
/// to say. In-progress messages stay until the work finishes; results clear
/// after a few seconds (the event loop owns the timer).
pub(crate) fn toast_line(app: &App, theme: &Theme) -> Option<Line<'static>> {
    // A scan announces itself in the header ("scanning…"), not here.
    if app.scanning {
        return None;
    }
    let msg = app.status.as_ref()?;
    // A failure reads red; everything else is a confirmation (green + ✓).
    let failed = msg.contains("fail")
        || msg.contains("error")
        || msg.contains("reject")
        || msg.contains("needs a ")
        || msg.starts_with("unknown ")
        || msg.starts_with("no ");
    let style = if failed {
        theme.risk()
    } else {
        theme.ok().add_modifier(Modifier::BOLD)
    };
    // While actions are still running the message is progress, not a result,
    // so it leads with the spinner. Afterwards the mark's shape says pass or
    // fail, so the colour is not needed to tell.
    let icon = if app.busy.is_empty() {
        theme.glyphs.result_mark(failed)
    } else {
        theme.glyphs.spinner(app.spinner)
    };
    // Control characters are dropped: a message can carry text git printed.
    let msg = theme.glyphs.plain(&text::safe(msg));
    Some(Line::from(Span::styled(format!(" {icon} {msg} "), style)))
}

/// The keys an overlay answers to, as `(key, what it does)`. The two screens
/// underneath list their own keys on their cards and on the key bar, so they
/// have none here.
pub(crate) fn overlay_hints(app: &App) -> Vec<(&'static str, &'static str)> {
    match app.mode {
        Mode::Normal | Mode::Filter => Vec::new(),
        Mode::Command => vec![
            ("⏎", "run"),
            ("tab", "complete"),
            ("↑/↓", "earlier lines"),
            ("Esc", "cancel"),
        ],
        Mode::Help => vec![("? / Esc", "close")],
        Mode::Launchpad => vec![("↑/↓", "scroll"), ("t / Esc", "close")],
        Mode::Standup => {
            if app.standup.as_ref().is_some_and(|s| s.commits_focused) {
                vec![
                    ("↑/↓", "scroll commits"),
                    ("←", "back to repos"),
                    ("w", "window"),
                    ("y", "copy"),
                    ("q", "close"),
                ]
            } else {
                vec![
                    ("↑/↓", "repo"),
                    ("→/⏎", "view commits"),
                    ("w", "window"),
                    ("y", "copy"),
                    ("q", "close"),
                ]
            }
        }
        Mode::CommandRun => vec![
            ("↑/↓", "repo"),
            ("PgUp/PgDn", "scroll output"),
            ("y", "copy"),
            ("Esc", "close"),
        ],
        Mode::Confirm => vec![("y", "confirm"), ("N / Esc", "cancel")],
        Mode::Detail => vec![("↑/↓", "scroll"), ("g/G", "top / bottom"), ("Esc", "close")],
        Mode::OpenWith => vec![
            ("↑/↓", "choose"),
            ("⏎", "open"),
            ("d", "set default"),
            ("Esc", "cancel"),
        ],
        Mode::PushBranch => vec![("↑/↓", "choose"), ("⏎", "push"), ("Esc", "cancel")],
    }
}

/// The current overlay's key hints as one styled line: keys accent-bold,
/// descriptions dim, `·`-separated. Rendered *inside* an overlay, above its
/// bottom border, by [`render_modal_hints`].
pub(crate) fn modal_hint_line(app: &App, theme: &Theme) -> Line<'static> {
    let key_style = theme.ahead().add_modifier(Modifier::BOLD);
    let mut spans: Vec<Span> = Vec::new();
    for (key, desc) in overlay_hints(app) {
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", theme.dim()));
        }
        spans.push(Span::styled(key.to_string(), key_style));
        spans.push(Span::styled(format!(" {desc}"), theme.dim()));
    }
    Line::from(spans)
}

/// Draw a modal's key-hint footer *inside* its box: a solid separator joined
/// into the side borders (`├─┤`, like the dock card), then the centered hints
/// on the row below. Returns the content area above the footer, so the caller
/// lays its body out in what's left. Pass the outer `area` (for the border
/// junctions) and the block's `inner`.
pub(crate) fn render_modal_hints(
    frame: &mut Frame,
    area: Rect,
    inner: Rect,
    app: &App,
    theme: &Theme,
) -> Rect {
    if inner.height < 3 {
        return inner; // no room to carve out a footer
    }
    let rule_y = inner.y + inner.height - 2;
    let hint_y = inner.y + inner.height - 1;
    // A panel needs no rule: the row above the hints is left empty instead.
    if theme.overlay_surface().is_none() {
        modal_rule(frame, area, rule_y, theme);
    }
    frame.render_widget(
        Paragraph::new(modal_hint_line(app, theme).centered()),
        Rect {
            x: inner.x,
            y: hint_y,
            width: inner.width,
            height: 1,
        },
    );
    Rect {
        height: inner.height - 2,
        ..inner
    }
}

/// A modal's title *inside* the box — a header row, then a solid separator joined
/// into the side borders — returning the content area below it. The companion to
/// [`render_modal_hints`]; pass the block's `inner` and the outer `area`.
pub(crate) fn render_modal_header(
    frame: &mut Frame,
    area: Rect,
    inner: Rect,
    theme: &Theme,
    mut title: Line<'static>,
) -> Rect {
    if inner.height < 3 {
        return inner;
    }
    if let Some(surface) = theme.overlay_surface() {
        // A panel: the title sits on a band in its section's colour.
        let (band, ink) = theme.band(theme.overlay.get(), true);
        paint_overlay_panel(frame, area, inner, surface, band);
        for (index, span) in title.spans.iter_mut().enumerate() {
            span.style = span.style.fg(ink);
            if index == 0 {
                span.style = span.style.add_modifier(Modifier::BOLD);
            }
        }
        frame.render_widget(Paragraph::new(title), Rect { height: 1, ..inner });
        return Rect {
            y: inner.y + 2,
            height: inner.height - 2,
            ..inner
        };
    }
    // Every overlay's title speaks in the same voice: bold bronze. Only the
    // first span is the title proper; anything after it (counts, context)
    // keeps the style its caller chose.
    if let Some(first) = title.spans.first_mut() {
        first.style = first
            .style
            .patch(theme.bronze().add_modifier(Modifier::BOLD));
    }
    frame.render_widget(Paragraph::new(title), Rect { height: 1, ..inner });
    modal_rule(frame, area, inner.y + 1, theme);
    Rect {
        y: inner.y + 2,
        height: inner.height - 2,
        ..inner
    }
}

/// Turn an overlay that has just been drawn as a box of lines into a panel,
/// the same way cards are panels: a filled surface with no lines around it and
/// the title on a coloured band.
///
/// Every overlay draws its own frame and then calls [`render_modal_header`],
/// so repainting here restyles all of them in one place. The frame's cells are
/// reused rather than removed, so nothing inside the overlay moves: the top
/// border and the rule under the title become the upper and lower half of the
/// band, the side borders become surface, and the bottom border becomes the
/// surface's lower edge (half a row, like a card's).
fn paint_overlay_panel(frame: &mut Frame, area: Rect, inner: Rect, surface: Color, band: Color) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let buffer = frame.buffer_mut();
    let right = area.x + area.width - 1;
    let bottom = area.y + area.height - 1;
    buffer.set_style(area, Style::new().bg(surface));
    for y in area.y..=bottom {
        buffer[(area.x, y)].set_symbol(" ");
        buffer[(right, y)].set_symbol(" ");
    }
    // Half blocks drawn on the terminal's own background.
    let edge = |colour: Color| Style::new().fg(colour).bg(Color::Reset);
    for x in area.x..=right {
        buffer[(x, area.y)].set_symbol("▄").set_style(edge(band));
        buffer[(x, bottom)].set_symbol("▀").set_style(edge(surface));
    }
    // The title row is the band; the row under it is the band's lower half.
    let title_row = Rect {
        y: inner.y,
        height: 1,
        ..area
    };
    buffer.set_style(title_row, Style::new().bg(band));
    for x in area.x..=right {
        buffer[(x, inner.y + 1)]
            .set_symbol("▀")
            .set_style(Style::new().fg(band).bg(surface));
    }
}

/// A horizontal rule across a modal at row `y`. In a box of lines it is solid
/// and joined into the double side borders with `╟`/`╢`, so it reads as part
/// of the frame; in a panel, which has no side borders, it is a quiet line
/// that stops short of the edges.
pub(crate) fn modal_rule(frame: &mut Frame, area: Rect, y: u16, theme: &Theme) {
    if theme.overlay_surface().is_some() {
        let line = "─".repeat(area.width.saturating_sub(4) as usize);
        frame.render_widget(
            Paragraph::new(Span::styled(line, theme.dim())),
            Rect {
                x: area.x + 2,
                y,
                width: area.width.saturating_sub(4),
                height: 1,
            },
        );
        return;
    }
    let style = Style::new();
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(area.width as usize),
            style,
        ))),
        Rect {
            x: area.x,
            y,
            width: area.width,
            height: 1,
        },
    );
    for (x, ch) in [(area.x, "╟"), (area.x + area.width.saturating_sub(1), "╢")] {
        frame.render_widget(
            Paragraph::new(Span::styled(ch, style)),
            Rect {
                x,
                y,
                width: 1,
                height: 1,
            },
        );
    }
}

/// A centered modal of fixed width (% of the frame) and height, for the command
/// overlays.
pub(crate) fn centered_modal(full: Rect, pct_w: u16, h: u16) -> Rect {
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
pub(crate) fn render_command_box(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    // Shell mode (opened with `!`): the accent shifts to a warning colour and the
    // sheet collapses to just the highlighted shell row.
    let shell = app.command_line.starts_with('!');
    let accent = if shell { theme.warn() } else { theme.ahead() };
    let cmd = |t: &str| Span::styled(t.to_string(), accent.add_modifier(Modifier::BOLD));
    let dim = |t: &str| Span::styled(t.to_string(), theme.dim());

    // (command spans, description) — a structured two-column cheat sheet.
    let rows: Vec<(Vec<Span<'static>>, &str)> = if shell {
        vec![(
            vec![cmd(":!"), Span::styled("<cmd>", accent)],
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
                vec![cmd(":commit"), dim(" <message>")],
                "commit every change in the targets (asks first)",
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
                "needs-attention · fuzzy filter",
            ),
            (
                vec![cmd(":jump"), dim(" <repo>")],
                "move the cursor to a repo",
            ),
            (
                vec![cmd(":standup"), dim("  "), cmd(":tasks")],
                "your commits · the launchpad",
            ),
            (
                vec![cmd(":refresh"), dim("  "), cmd(":quit")],
                "rescan · exit",
            ),
        ]
    };
    let desc_style = if shell { accent } else { theme.dim() };

    // +2 for the header (title + rule), +2 for the hint footer (rule + keys).
    let area = centered_modal(full, 74, rows.len() as u16 + 8);
    frame.render_widget(Clear, area);
    let title = if shell { "Shell command" } else { "Command" };
    let block = Block::bordered()
        .border_type(FRAME)
        .border_style(if shell { accent } else { Style::new() })
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let title_style = if shell { accent } else { Style::new() };
    let inner = render_modal_header(
        frame,
        area,
        inner,
        theme,
        Line::from(Span::styled(
            title,
            title_style.add_modifier(Modifier::BOLD),
        )),
    );
    let inner = render_modal_hints(frame, area, inner, app, theme);

    // Prompt, a solid rule, then the two-column cheat sheet (commands │ what they
    // do) as one framed table — the divider joined into the rule above (`┬`) and the
    // footer rule below (`┴`), matching the standup and detail panes.
    let [prompt_area, mid_rule_area, grid_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);

    let (before, after) = app.command_edit.split(&app.command_line);
    let typed = if shell { accent } else { Style::new() };
    // While Tab is cycling through completions, say how many there are.
    let cycling = app
        .command_edit
        .completing()
        .filter(|(_, count)| *count > 1)
        .map(|(showing, count)| format!("   {showing} of {count}, tab for the next"))
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(": ", accent.add_modifier(Modifier::BOLD)),
            // The cursor is drawn where it is, which is not always the end.
            Span::styled(before.to_string(), typed),
            Span::raw("▏"),
            Span::styled(after.to_string(), typed),
            Span::styled(cycling, theme.dim()),
        ])),
        prompt_area,
    );

    // Commands column | descriptions column.
    let [cmd_col, div_col, desc_col] = Layout::horizontal([
        Constraint::Length(26),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .spacing(1)
    .areas(grid_area);

    // The solid rule under the prompt, and the solid vertical divider joined into
    // it (`┬`) and the footer rule below (`┴`).
    modal_rule(frame, area, mid_rule_area.y, theme);
    for y in div_col.y..div_col.y + div_col.height {
        frame.render_widget(
            Paragraph::new(Span::raw("│")),
            Rect {
                x: div_col.x,
                y,
                width: 1,
                height: 1,
            },
        );
    }
    for (ch, y) in [
        ("┬", mid_rule_area.y),
        ("┴", grid_area.y + grid_area.height),
    ] {
        frame.render_widget(
            Paragraph::new(Span::raw(ch)),
            Rect {
                x: div_col.x,
                y,
                width: 1,
                height: 1,
            },
        );
    }

    let cmd_lines: Vec<Line> = rows.iter().map(|(s, _)| Line::from(s.clone())).collect();
    let desc_lines: Vec<Line> = rows
        .iter()
        .map(|(_, d)| Line::from(Span::styled(d.to_string(), desc_style)))
        .collect();
    frame.render_widget(Paragraph::new(Text::from(cmd_lines)), cmd_col);
    frame.render_widget(Paragraph::new(Text::from(desc_lines)), desc_col);
}
