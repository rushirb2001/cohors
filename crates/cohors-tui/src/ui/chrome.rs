use super::*;

pub(crate) fn render_header(frame: &mut Frame, area: Rect, app: &App, now: i64, theme: &Theme) {
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
pub(crate) fn header_dir(app: &App) -> String {
    match app.roots.as_slice() {
        [] => "(no roots)".to_string(),
        [one] => abbrev_home(one),
        [first, rest @ ..] => format!("{} (+{})", abbrev_home(first), rest.len()),
    }
}

/// The header's right-hand info column: session orientation that isn't shown
/// elsewhere — where we're watching, the config in effect, and the fleet's most
/// recent activity. Three aligned `label value` rows trimmed to `width`.
pub(crate) fn header_info(app: &App, now: i64, width: u16, theme: &Theme) -> Text<'static> {
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
pub(crate) fn abbrev_home(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

/// Trim `s` to at most `max` columns, keeping the tail (most specific part of a
/// path) and prefixing an ellipsis when truncated.
pub(crate) fn truncate_tail(s: &str, max: usize) -> String {
    let len = s.chars().count();
    if max == 0 || len <= max {
        return s.to_string();
    }
    let tail: String = s.chars().skip(len - max.saturating_sub(1)).collect();
    format!("…{tail}")
}

/// A hint's rendered width (`key` + space + `desc`).
pub(crate) fn footer_item_w(key: &str, desc: &str) -> usize {
    key.chars().count() + 1 + desc.chars().count()
}

/// Hints at or below this width pack into the two-column grid; wider ones
/// ("run command", "mark repo"…) drop to full-width rows below the grid, so the
/// columns stay narrow enough to always fit two-up — even on a compact terminal.
pub(crate) const FOOTER_GRID_MAX: usize = 10;

/// One footer hint: `(key, description)`.
type Hint<'a> = &'a (&'a str, &'a str);

/// Split a group's hints into the two-column "short" set and the full-width
/// "long" set, preserving order.
pub(crate) fn footer_partition<'a>(
    items: &'a [(&'a str, &'a str)],
) -> (Vec<Hint<'a>>, Vec<Hint<'a>>) {
    items
        .iter()
        .partition(|(k, d)| footer_item_w(k, d) <= FOOTER_GRID_MAX)
}

/// A group box's content height: the in-box title row + its divider rule, the
/// two-column grid rows, the full-width long rows, and a 1-row horizontal divider
/// between them when both are present.
pub(crate) fn footer_group_rows(items: &[(&str, &str)]) -> u16 {
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
pub(crate) fn footer_height(app: &App, width: u16, theme: &Theme) -> u16 {
    if app.mode == Mode::Normal {
        // The hint bar is collapsed to just its toggle divider.
        if app.hints_hidden {
            return 1;
        }
        // The consolidated one-box bar when it fits; the three stacked group
        // boxes otherwise (narrow terminals).
        let (_, bar_rows, bar_total) = footer_bar_geometry(app);
        if bar_total <= width {
            return bar_rows + 2 + 1; // borders + the toggle divider row
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
pub(crate) fn render_footer(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if app.mode == Mode::Normal {
        // A divider that doubles as the show/hide affordance, then the boxes
        // (unless the user has collapsed them with `h`).
        // A divider that doubles as the show/hide affordance, then the one-box
        // bar (or the stacked group boxes on narrow terminals).
        let [divider, boxes] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
        render_hint_divider(frame, divider, app.hints_hidden, theme);
        if !app.hints_hidden {
            let (_, _, bar_total) = footer_bar_geometry(app);
            if bar_total <= area.width {
                render_footer_bar(frame, boxes, app, theme);
            } else {
                render_footer_boxed(frame, boxes, app, theme);
            }
        }
    } else {
        render_footer_simple(frame, area, footer_lines(app, theme), theme);
    }
}

/// A full-width divider with centered text advertising the `h` toggle:
/// "── press h to hide hints ──" (or "unhide" when the boxes are collapsed).
pub(crate) fn render_hint_divider(frame: &mut Frame, area: Rect, hidden: bool, theme: &Theme) {
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

pub(crate) fn render_footer_simple(
    frame: &mut Frame,
    area: Rect,
    lines: Vec<Line<'static>>,
    theme: &Theme,
) {
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
pub(crate) fn render_footer_boxed(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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
pub(crate) fn render_group_box(
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

/// Each hint-bar section packs its items column-major into at most this many
/// grid rows, growing columns as needed — so a 7-item group becomes 3 columns
/// of ≤3 rather than a tall 2-column stack.
pub(crate) const FOOTER_BAR_ROWS: usize = 3;

/// A section's items chunked into its column-major columns.
fn footer_bar_columns<'a>(
    items: &'a [(&'static str, &'static str)],
) -> impl Iterator<Item = &'a [(&'static str, &'static str)]> {
    let cols = items.len().div_ceil(FOOTER_BAR_ROWS).max(1);
    items.chunks(items.len().div_ceil(cols).max(1))
}

/// The consolidated hint bar's geometry: each section's fitted width (every
/// hint column sized to its own content, so nothing wraps or overflows), the
/// content row count (the label row + the tallest grid), and the total width
/// the bar needs. Used to decide whether the one-box layout fits.
pub(crate) fn footer_bar_geometry(app: &App) -> (Vec<u16>, u16, u16) {
    let groups = footer_groups(app);
    let mut widths: Vec<u16> = Vec::new();
    let mut grid_rows: usize = 1;
    for (label, items) in &groups {
        let mut grid: u16 = 0;
        for (i, chunk) in footer_bar_columns(items).enumerate() {
            grid_rows = grid_rows.max(chunk.len());
            let col = chunk
                .iter()
                .map(|(k, d)| footer_item_w(k, d))
                .max()
                .unwrap_or(0) as u16;
            // Columns are joined by ` │ ` (a light divider with a space each
            // side), so each junction costs 3 cells.
            grid += col + if i > 0 { 3 } else { 0 };
        }
        let label_w = label.chars().count() as u16; // the label row must fit too
        widths.push(grid.max(label_w) + 2); // 1-col padding each side
    }
    let dividers = widths.len().saturating_sub(1) as u16;
    let total = widths.iter().sum::<u16>() + dividers + 2; // + the outer borders
    // label row + its rule + the grid.
    (widths, 2 + grid_rows as u16, total)
}

/// The consolidated hint bar: ONE bordered box, split into the `select` /
/// `act` / `view` sections by thin `│` dividers (with `┬`/`┴` junctions — box
/// drawing only; no text ever sits on a border). Each section starts with its
/// label row (the `act` one carries the live target) and packs its hints below
/// in content-fitted, column-major columns, so every hint fits without
/// wrapping.
pub(crate) fn render_footer_bar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let groups = footer_groups(app);
    let (widths, _rows, _total) = footer_bar_geometry(app);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.dim());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Alternate section / divider constraints. The leftover width is spread
    // evenly across the sections (not dumped into the last one), so the
    // dividers space out and no section ends in a void; the last section
    // absorbs only the rounding remainder.
    let leftover = (inner.width as usize)
        .saturating_sub(widths.iter().sum::<u16>() as usize + widths.len().saturating_sub(1));
    let share = (leftover / widths.len().max(1)) as u16;
    let mut cons: Vec<Constraint> = Vec::new();
    for (i, w) in widths.iter().enumerate() {
        if i > 0 {
            cons.push(Constraint::Length(1));
        }
        if i == widths.len() - 1 {
            cons.push(Constraint::Fill(1));
        } else {
            cons.push(Constraint::Length(*w + share));
        }
    }
    let rects = Layout::horizontal(cons).split(inner);

    let key_style = theme.ahead().add_modifier(Modifier::BOLD);
    let mut section = 0usize;
    for (ri, rect) in rects.iter().enumerate() {
        // Odd chunks are the 1-col dividers: a `│` per row, junctions on the
        // borders above and below.
        if ri % 2 == 1 {
            let bar: Vec<Line> = (0..rect.height)
                .map(|_| Line::from(Span::styled("│", theme.dim())))
                .collect();
            frame.render_widget(Paragraph::new(Text::from(bar)), *rect);
            for (y, ch) in [(area.y, "┬"), (area.y + area.height - 1, "┴")] {
                frame.render_widget(
                    Paragraph::new(Span::styled(ch, theme.dim())),
                    Rect {
                        x: rect.x,
                        y,
                        width: 1,
                        height: 1,
                    },
                );
            }
            continue;
        }

        let (label, items) = &groups[section];
        section += 1;

        let content = Rect {
            x: rect.x + 1,
            width: rect.width.saturating_sub(2),
            ..*rect
        };
        let [label_area, rule_area, grid_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .areas(content);

        // The centered header row, inside the box — never on the border; `act`
        // carries the live target beside it.
        let mut label_spans = vec![Span::styled(
            label.to_string(),
            theme.dim().add_modifier(Modifier::BOLD),
        )];
        if *label == "act"
            && let Some((_, target)) = action_target_hint(app)
        {
            let room = content
                .width
                .saturating_sub(label.chars().count() as u16 + 3) as usize;
            label_spans.push(Span::raw(" "));
            label_spans.push(Span::styled(
                format!("→ {}", ellipsize(&target, room)),
                theme.highlight(),
            ));
        }
        frame.render_widget(
            Paragraph::new(Line::from(label_spans)).alignment(Alignment::Center),
            label_area,
        );

        // A dim rule under the header, spanning the section's content width.
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(rule_area.width as usize),
                theme.dim(),
            ))),
            rule_area,
        );

        // The fitted hint columns (order preserved down-then-across), centered
        // as a group and separated by light `│` dividers.
        let hint = |(k, d): &(&str, &str)| {
            Line::from(vec![
                Span::styled((*k).to_string(), key_style),
                Span::styled(format!(" {d}"), theme.dim()),
            ])
        };
        let chunks: Vec<_> = footer_bar_columns(items).collect();
        let col_ws: Vec<u16> = chunks
            .iter()
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|(k, d)| footer_item_w(k, d))
                    .max()
                    .unwrap_or(0) as u16
            })
            .collect();
        let grid_w: u16 = col_ws.iter().sum::<u16>() + 3 * chunks.len().saturating_sub(1) as u16;
        let grid = Rect {
            x: grid_area.x + grid_area.width.saturating_sub(grid_w) / 2,
            width: grid_w.min(grid_area.width),
            ..grid_area
        };
        let mut col_cons: Vec<Constraint> = Vec::new();
        for (i, w) in col_ws.iter().enumerate() {
            if i > 0 {
                col_cons.push(Constraint::Length(1)); // the light divider column
            }
            col_cons.push(Constraint::Length(*w));
        }
        let col_rects = Layout::horizontal(col_cons).spacing(1).split(grid);
        let sec_rows = chunks.iter().map(|c| c.len()).max().unwrap_or(0) as u16;
        let mut chunk_iter = chunks.iter();
        for (ci, col_rect) in col_rects.iter().enumerate() {
            if ci % 2 == 1 {
                // The light divider between hint columns, only as tall as the
                // section's hint rows.
                let bar: Vec<Line> = (0..sec_rows.min(col_rect.height))
                    .map(|_| Line::from(Span::styled("│", theme.dim())))
                    .collect();
                frame.render_widget(Paragraph::new(Text::from(bar)), *col_rect);
                continue;
            }
            if let Some(chunk) = chunk_iter.next() {
                let text = Text::from(chunk.iter().map(hint).collect::<Vec<_>>());
                frame.render_widget(Paragraph::new(text), *col_rect);
            }
        }
    }
}

/// The "Attention" panel: a titled box summarizing what needs the user, in
/// plain words ("3 dirty · 1 behind") rather than terse glyphs.
pub(crate) fn render_attention_panel(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    now: i64,
    theme: &Theme,
) {
    let s = fleet_summary(&app.repos, now);

    // Wide terminals get the web's overview strip: a fleet-activity card (the
    // heartbeat sparkline) beside the attention card (a severity bar + the four
    // bucket counts). Narrow ones keep a single-line chip summary — two cards
    // would crush each other.
    if area.height >= 4 && area.width >= 76 {
        let [act_area, attn_area] =
            Layout::horizontal([Constraint::Length(34), Constraint::Min(0)])
                .spacing(1)
                .areas(area);
        render_activity_card(frame, act_area, app, theme);
        render_attention_card(frame, attn_area, &s, theme);
        return;
    }

    // The "N of M repositories" count lives in the box title; the body is the
    // severity buckets (or an all-clear message) — the same four numbers the
    // wide layout and the web show.
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
        Line::from(severity_bucket_spans(&s, theme))
    };

    frame.render_widget(Paragraph::new(line), inner);
}

/// The four severity-bucket chips — `N at risk · N warning · N minor · N calm`
/// — each coloured by its severity, zero buckets skipped (calm always shows).
/// The same numbers, wording, and colour order as the web's attention legend.
pub(crate) fn severity_bucket_spans(s: &FleetSummary, theme: &Theme) -> Vec<Span<'static>> {
    let mut items: Vec<(String, Style)> = Vec::new();
    if s.risk > 0 {
        items.push((format!("{} at risk", s.risk), theme.risk()));
    }
    if s.warn > 0 {
        items.push((
            format!("{} warning", s.warn),
            severity_style(Severity::Warn, theme),
        ));
    }
    if s.notice > 0 {
        items.push((
            format!("{} minor", s.notice),
            severity_style(Severity::Notice, theme),
        ));
    }
    items.push((format!("{} calm", s.calm), theme.ok()));

    let mut spans: Vec<Span> = Vec::new();
    for (i, (text, style)) in items.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", theme.dim()));
        }
        spans.push(Span::styled(text, style));
    }
    spans
}

/// The fleet-activity card: every repo's weekly commit counts summed into one
/// 12-week sparkline (each week two cells wide, so the shape reads), captioned
/// with the totals — the TUI's version of the web's "fleet activity" block.
fn render_activity_card(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(Span::styled(
            " Fleet activity ",
            Style::new().add_modifier(Modifier::BOLD),
        )))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Element-wise sum of each repo's 12-week activity (oldest first). Repos
    // scanned before the activity field existed contribute nothing.
    let mut weeks = [0u64; 12];
    for repo in &app.repos {
        for (w, n) in repo.activity.iter().take(12).enumerate() {
            weeks[w] += u64::from(*n);
        }
    }
    let total: u64 = weeks.iter().sum();

    let ramp = theme.glyphs.spark_ramp();
    let max = weeks.iter().copied().max().unwrap_or(0);
    let mut spark = String::new();
    for &n in &weeks {
        let idx = if max == 0 {
            0
        } else {
            ((n * 7).div_ceil(max)) as usize
        };
        let ch = ramp[idx.min(7)];
        spark.push(ch);
        spark.push(ch); // two cells per week: wide enough to read as a bar
    }

    let caption = if total == 0 {
        "quiet · 12 weeks".to_string()
    } else {
        format!("{total} commits · 12 weeks")
    };
    let lines = vec![
        Line::from(Span::styled(spark, theme.fg(SPIDER_PURPLE))),
        Line::from(Span::styled(caption, theme.dim())),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The attention card: a full-width severity bar (segments proportional to the
/// four buckets) over the bucket counts — the web's attention breakdown. Under
/// Ascii the segments use distinct characters, so the split reads without
/// colour.
fn render_attention_card(frame: &mut Frame, area: Rect, s: &FleetSummary, theme: &Theme) {
    let mut title = vec![Span::styled(
        " Attention ",
        Style::new().add_modifier(Modifier::BOLD),
    )];
    title.push(Span::styled(
        format!("({} of {} repositories) ", s.needs_attention, s.total),
        theme.dim(),
    ));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(title))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // The bar: one coloured run per non-empty bucket, at least one cell each,
    // rounding drift absorbed by the widest bucket so the bar always fills.
    let buckets: [(usize, Style); 4] = [
        (s.risk, theme.risk()),
        (s.warn, severity_style(Severity::Warn, theme)),
        (s.notice, severity_style(Severity::Notice, theme)),
        (s.calm, theme.ok()),
    ];
    let width = inner.width as usize;
    let mut cells: Vec<usize> = buckets
        .iter()
        .map(|(n, _)| {
            if *n == 0 {
                0
            } else {
                ((n * width) / s.total.max(1)).max(1)
            }
        })
        .collect();
    // Trim/pad the widest run until the total matches the width exactly.
    let assigned: usize = cells.iter().sum();
    if let Some(widest) = (0..4).max_by_key(|&i| cells[i])
        && cells[widest] > 0
    {
        cells[widest] = (cells[widest] + width).saturating_sub(assigned).max(1);
    }
    let mut bar: Vec<Span> = Vec::new();
    for (i, ((_, style), n)) in buckets.iter().zip(&cells).enumerate() {
        if *n > 0 {
            bar.push(Span::styled(
                theme.glyphs.bar(i).to_string().repeat(*n),
                *style,
            ));
        }
    }

    let lines = vec![Line::from(bar), Line::from(severity_bucket_spans(s, theme))];
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The self-dismissing toast line — scan progress or the latest action result
/// (fetch/pull/push/stash/copy…) — shown on the **top-right of the Repositories
/// box border**, not crowding the header. `None` when there's nothing to show.
/// In-progress messages stay until the work finishes; results clear after a few
/// seconds (the event loop owns the timer).
pub(crate) fn toast_line(app: &App, theme: &Theme) -> Option<Line<'static>> {
    // Scanning shows no toast at all — the refresh announces itself via the
    // braille spinners animating inside the table's Sync/PRs/CI cells.
    if app.scanning {
        return None;
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
pub(crate) fn footer_groups(app: &App) -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
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
                    ("d", "attention"),
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
pub(crate) fn footer_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
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
pub(crate) fn action_target_hint(app: &App) -> Option<(String, String)> {
    let n = app.selection.len();
    if n > 0 {
        Some(("   → acts on ".to_string(), format!("{n} selected")))
    } else {
        app.selected_repo()
            .map(|r| ("   → acts on ".to_string(), r.name.clone()))
    }
}

pub(crate) fn render_filter_input(frame: &mut Frame, area: Rect, app: &App) {
    let line = Line::from(vec![
        Span::raw("/ "),
        Span::raw(app.filter.clone()),
        Span::raw("▏"),
    ]);
    frame.render_widget(Paragraph::new(line), area);
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
                "needs-attention · fuzzy filter",
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

pub(crate) fn render_loading(frame: &mut Frame, area: Rect, app: &App) {
    let text = format!(
        "{} Scanning {}…",
        spinner_frame(app.spinner),
        roots_label(app)
    );
    let para = Paragraph::new(text).alignment(Alignment::Center);
    frame.render_widget(para, center_v(area, 1));
}

pub(crate) fn render_empty(frame: &mut Frame, area: Rect, app: &App) {
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

pub(crate) fn roots_label(app: &App) -> String {
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
pub(crate) fn keep_visible(prev: usize, selected: usize, viewport: usize, total: usize) -> usize {
    let mut off = prev;
    if selected < off {
        off = selected;
    } else if viewport > 0 && selected >= off + viewport {
        off = selected + 1 - viewport;
    }
    off.min(total.saturating_sub(viewport))
}
