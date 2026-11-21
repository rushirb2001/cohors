//! The card: the one container every part of the dashboard is drawn in.
//!
//! A card is a rounded frame whose top border carries its title (with the
//! number key that focuses it) and whose bottom border lists the keys that act
//! on it. The focused card gets the accent colour; where colour is off it gets
//! a heavier frame instead, so focus never depends on colour alone.
//!
//! This module also draws the scrolling lists that live inside cards, because
//! every list shares the same rules: a cursor mark in the gutter, a washed
//! background on the cursor row, and an "N more" line when rows are hidden.

use ratatui::crossterm::event::KeyCode;

use super::layout::ListWindow;
use super::*;

/// One key hint on a card's bottom border: the key, then what it does.
pub(crate) struct Hint {
    pub key: String,
    pub label: String,
}

pub(crate) fn hint(key: &str, label: &str) -> Hint {
    Hint {
        key: key.to_string(),
        label: label.to_string(),
    }
}

/// The key a hint stands for, when clicking the hint can press it: one
/// character, Enter, Space, Esc or Tab. Not a range (`1-5`), the arrows
/// (`↑↓`) or `type`, which are more than one key.
pub(crate) fn hint_code(key: &str, theme: &Theme) -> Option<KeyCode> {
    let glyphs = &theme.glyphs;
    if key == glyphs.key_enter() || key == "enter" {
        return Some(KeyCode::Enter);
    }
    if key == glyphs.key_space() || key == "space" {
        return Some(KeyCode::Char(' '));
    }
    match key {
        "esc" => Some(KeyCode::Esc),
        "tab" => Some(KeyCode::Tab),
        _ => {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_graphic() => Some(KeyCode::Char(c)),
                _ => None,
            }
        }
    }
}

/// Note where each hint of a row of them was drawn, so a click on one
/// presses its key. `slots` are the hints with their columns from `x`.
pub(crate) fn record_hints(x: u16, y: u16, slots: &[(usize, &Hint)], theme: &Theme) {
    for (offset, hint) in slots {
        if let Some(code) = hint_code(&hint.key, theme) {
            let width = hint.key.chars().count() + 2 + hint.label.chars().count();
            let area = Rect {
                x: x.saturating_add(*offset as u16),
                y,
                width: width as u16,
                height: 1,
            };
            crate::mouse::record(area, crate::mouse::Target::Key(code));
        }
    }
}

/// What to draw around a card's content.
#[derive(Default)]
pub(crate) struct CardFrame {
    /// The number key that focuses the card, shown in the title.
    pub number: Option<u8>,
    /// For a card with no number of its own: the section whose colour it
    /// takes (the detail view's tasks card is the tasks section's colour).
    pub hue: Option<u8>,
    /// The title text, after the number and the icon.
    pub title: Vec<Span<'static>>,
    /// Keys that act on the card, listed on the bottom border.
    pub hints: Vec<Hint>,
    pub focused: bool,
}

/// Where a card's content goes: `text` is the area inside the frame with one
/// column of padding on each side, and `gutter_x` is the left padding column,
/// which lists use for the cursor mark.
#[derive(Clone, Copy)]
pub(crate) struct CardArea {
    pub gutter_x: u16,
    pub text: Rect,
}

impl CardArea {
    /// The text width in columns, as a `usize` for string arithmetic.
    pub(crate) fn width(&self) -> usize {
        self.text.width as usize
    }

    /// The same area without its first `rows` lines (e.g. below a table header).
    pub(crate) fn below(&self, rows: u16) -> CardArea {
        let rows = rows.min(self.text.height);
        CardArea {
            gutter_x: self.gutter_x,
            text: Rect {
                y: self.text.y + rows,
                height: self.text.height - rows,
                ..self.text
            },
        }
    }
}

/// A card's key hints, shown only while the card has the focus.
///
/// The overview's cards answer to different keys, and a key only works on the
/// card that is focused. Listing every card's keys at once was both crowded
/// and untrue (`f` does nothing while "Do next" has the focus), so a card
/// shows its keys when they apply. The keys that work everywhere are on the
/// key bar at the bottom.
pub(crate) fn hints_when(focused: bool, hints: Vec<Hint>) -> Vec<Hint> {
    if focused { hints } else { Vec::new() }
}

/// Draw a card's frame into `area` and return where its content goes.
pub(crate) fn draw_card(
    frame: &mut Frame,
    area: Rect,
    card: &CardFrame,
    theme: &Theme,
) -> CardArea {
    draw_card_filled(frame, area, card, theme, None)
}

/// [`draw_card`], with the option of saying what colour the card's surface
/// is. Only a scheme that fills its cards uses `fill` (a figure tile that
/// needs attention is filled with its severity's colour); a scheme of lines
/// ignores it and the caller paints the inside itself.
pub(crate) fn draw_card_filled(
    frame: &mut Frame,
    area: Rect,
    card: &CardFrame,
    theme: &Theme,
    fill: Option<Color>,
) -> CardArea {
    record_card(area, card, theme);
    let content = CardArea {
        gutter_x: area.x.saturating_add(1),
        text: Rect {
            x: area.x.saturating_add(2),
            y: area.y.saturating_add(1),
            width: area.width.saturating_sub(4),
            height: area.height.saturating_sub(2),
        },
    };
    if let Some(surface) = theme.surface() {
        // A panel needs a top edge, a bottom edge and something between.
        if area.height >= 3 && area.width >= 2 {
            draw_panel(frame, area, card, fill.unwrap_or(surface), theme);
            return content;
        }
    }

    // Without colour the focused card is told apart by a heavier frame.
    let heavy = card.focused && !theme.color;
    let set = theme.glyphs.card_border(heavy);
    let border_style = theme.card_line(card.number.or(card.hue), card.focused);

    let mut block = Block::bordered().border_set(set).border_style(border_style);
    if card.number.is_some() || !card.title.is_empty() {
        block = block.title_top(title_line(card, set.horizontal_top, border_style, theme));
    }
    if !card.hints.is_empty() {
        block = block.title_bottom(hints_line(
            &card.hints,
            area.width,
            set.horizontal_bottom,
            border_style,
            theme,
        ));
    }
    frame.render_widget(block, area);
    content
}

/// Note where a card was drawn (a numbered one takes the focus when
/// clicked) and where its hints are (each presses its key). The hints sit on
/// the bottom row, starting after the corner, in both of a card's styles.
fn record_card(area: Rect, card: &CardFrame, theme: &Theme) {
    if let Some(number) = card.number
        && let Some(target) = crate::app::Card::ALL
            .into_iter()
            .find(|c| c.number() == number)
    {
        crate::mouse::record(area, crate::mouse::Target::Card(target));
    }
    if area.height >= 2 && !card.hints.is_empty() {
        let slots = hint_slots(&card.hints, area.width);
        record_hints(area.x + 1, area.y + area.height - 1, &slots, theme);
    }
}

/// A card as a filled surface instead of a frame of lines.
///
/// The rows are the same as a framed card's, so nothing inside moves: the top
/// row is a lighter band that carries the title, the bottom row carries the
/// key hints. A row with nothing to carry is drawn with half-height blocks, so
/// the surface stops half a row short and stacked cards get a visible gap
/// without the layout giving up a row for it.
fn draw_panel(frame: &mut Frame, area: Rect, card: &CardFrame, surface: Color, theme: &Theme) {
    let top = Rect { height: 1, ..area };
    let bottom = Rect {
        y: area.y + area.height - 1,
        height: 1,
        ..area
    };
    let body = Rect {
        y: area.y + 1,
        height: area.height - 2,
        ..area
    };
    let titled = card.number.is_some() || !card.title.is_empty();
    let buffer = frame.buffer_mut();
    buffer.set_style(body, Style::new().bg(surface));
    // Half blocks are drawn in the surface colour on the terminal's own
    // background: `▄` fills the lower half of the top row, `▀` the upper half
    // of the bottom row.
    let mut half = |row: Rect, symbol: &str| {
        for x in row.x..row.x + row.width {
            buffer[(x, row.y)]
                .set_symbol(symbol)
                .set_style(Style::new().fg(surface));
        }
    };
    if !titled {
        half(top, "▄");
    }
    if card.hints.is_empty() {
        half(bottom, "▀");
    }

    if titled {
        // The band is the section's colour; the whole title takes the
        // band's text colour, whatever weight each part of it has.
        let (ground, ink) = theme.band(card.number.or(card.hue), card.focused);
        frame.buffer_mut().set_style(top, Style::new().bg(ground));
        let mut line = title_line(card, " ", Style::new(), theme);
        for span in &mut line.spans {
            span.style = span.style.fg(ink);
        }
        frame.render_widget(Paragraph::new(line), after_corner(top));
    }
    if !card.hints.is_empty() {
        frame
            .buffer_mut()
            .set_style(bottom, Style::new().bg(surface));
        let line = hints_line(&card.hints, area.width, " ", Style::new(), theme);
        frame.render_widget(Paragraph::new(line), after_corner(bottom));
    }
}

/// A row without its first column. A framed card's title starts after the
/// corner glyph; a panel has no corner, and skipping the same column keeps
/// its title exactly where the frame would have put it.
fn after_corner(row: Rect) -> Rect {
    Rect {
        x: row.x.saturating_add(1),
        width: row.width.saturating_sub(1),
        ..row
    }
}

/// The title on the top border: `─  1  ▶ Do next  5 ` (the leading rule is
/// part of the title so the text sits two columns in from the corner).
fn title_line(card: &CardFrame, rule: &str, border: Style, theme: &Theme) -> Line<'static> {
    let mut label = Vec::new();
    match card.number {
        Some(number) => {
            let key_style = if card.focused {
                theme.accent().add_modifier(Modifier::BOLD)
            } else {
                Style::new().add_modifier(Modifier::BOLD)
            };
            label.push(Span::styled(format!("  {number}  "), key_style));
            let icon = theme.glyphs.card_icon(number);
            if !icon.is_empty() {
                label.push(Span::styled(format!("{icon} "), border));
            }
        }
        None => label.push(Span::raw(" ")),
    }
    label.extend(card.title.iter().cloned());
    label.push(Span::raw(" "));
    // A card that belongs to a section speaks in that section's colour, so
    // the five are told apart whatever the cards are made of. (A panel then
    // recolours the title for its band; without colour this does nothing.)
    let section = card.number.or(card.hue);
    if section.is_some() {
        let hue = theme.fg(theme.section_hue(section));
        for span in &mut label {
            span.style = span.style.patch(hue);
        }
    }
    let mut spans = vec![Span::styled(rule.to_string(), border)];
    spans.extend(label);
    Line::from(spans)
}

/// The key hints on the bottom border: `─  ⏎  open   f  fetch  `. Only whole
/// hints are shown; the ones that do not fit are left off rather than cut in
/// half (they are all listed in the help overlay).
fn hints_line(
    hints: &[Hint],
    card_width: u16,
    rule: &str,
    border: Style,
    theme: &Theme,
) -> Line<'static> {
    let key_style = Style::new().add_modifier(Modifier::BOLD);
    let label_style = theme.dim();
    let slots = hint_slots(hints, card_width);
    if slots.is_empty() {
        // Not even one hint fits: leave the border plain.
        return Line::from(Span::styled(rule.to_string(), border));
    }
    let mut spans = vec![Span::styled(rule.to_string(), border), Span::raw("  ")];
    for (i, (_, hint)) in slots.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(hint.key.clone(), key_style));
        spans.push(Span::styled(format!("  {}", hint.label), label_style));
    }
    spans.push(Span::raw("  "));
    Line::from(spans)
}

/// The hints that fit on a card's bottom border, each with the column it
/// starts at, counted from just after the left corner: the leading rule and
/// two spaces, then the hints three apart.
fn hint_slots(hints: &[Hint], card_width: u16) -> Vec<(usize, &Hint)> {
    // Corners (2), the leading rule (1), two spaces each side of the hints,
    // and at least one rule before the right corner.
    let mut room = (card_width as usize).saturating_sub(2 + 1 + 2 + 2 + 1);
    let mut at = 1 + 2;
    let mut slots = Vec::new();
    for hint in hints {
        let gap = if slots.is_empty() { 0 } else { 3 };
        let width = hint.key.chars().count() + 2 + hint.label.chars().count();
        if gap + width > room {
            break;
        }
        room -= gap + width;
        at += gap;
        slots.push((at, hint));
        at += width;
    }
    slots
}

/// One entry of a list: its line(s) of text, and optionally a mark for the
/// gutter (a "marked for a bulk action" dot, or a spinner).
pub(crate) struct Item {
    pub lines: Vec<Line<'static>>,
    pub gutter: Option<Span<'static>>,
    /// Lines drawn under the entry: what it opens into (a repository's
    /// worktrees under its row). They belong to the entry for a click, but
    /// are not washed by the cursor, and the entries after them move down.
    pub below: Vec<Line<'static>>,
}

impl Item {
    pub(crate) fn line(line: Line<'static>) -> Self {
        Self {
            lines: vec![line],
            gutter: None,
            below: Vec::new(),
        }
    }
}

/// Draw the visible part of a list into a card.
///
/// `window` says which entries are visible, `item_height` how many lines each
/// takes, and `cursor` which entry (if any) the cursor is on. `build` is only
/// called for the entries that are actually drawn. An entry may bring lines
/// of its own to draw under it ([`Item::below`]); whoever worked out `window`
/// has to have left room for them.
pub(crate) fn draw_list(
    frame: &mut Frame,
    area: CardArea,
    window: ListWindow,
    item_height: u16,
    cursor: Option<usize>,
    theme: &Theme,
    mut build: impl FnMut(usize) -> Item,
) {
    let text = area.text;
    let bottom = text.y + text.height;
    // Where the next entry goes: entries are `item_height` apart, except
    // after one that drew lines of its own beneath it.
    let mut next_y = text.y;
    for slot in 0..window.shown {
        let index = window.offset + slot;
        let y = next_y;
        if y + item_height > bottom {
            break; // never draw outside the card
        }
        let item = build(index);
        crate::mouse::record(
            Rect {
                x: area.gutter_x,
                y,
                width: text.width + 2,
                height: item_height,
            },
            crate::mouse::Target::Row(index),
        );
        let on_cursor = cursor == Some(index);
        if on_cursor {
            // Wash the whole row, gutter and right padding included.
            let row = Rect {
                x: area.gutter_x,
                y,
                width: text.width + 2,
                height: item_height,
            };
            frame.buffer_mut().set_style(row, theme.selection());
        }
        for (i, line) in item
            .lines
            .into_iter()
            .take(item_height as usize)
            .enumerate()
        {
            let line_area = Rect {
                y: y + i as u16,
                height: 1,
                ..text
            };
            frame.render_widget(Paragraph::new(line), line_area);
        }
        if on_cursor && let Some(ink) = theme.selection_text() {
            // A solid cursor row: its text takes one colour that reads on it.
            let row = Rect {
                x: area.gutter_x,
                y,
                width: text.width + 2,
                height: item_height,
            };
            frame.buffer_mut().set_style(row, ink);
        }
        let gutter = item.gutter.or_else(|| {
            on_cursor.then(|| Span::styled(theme.glyphs.cursor(), theme.accent().bold()))
        });
        if let Some(mark) = gutter {
            let cell = Rect {
                x: area.gutter_x,
                y,
                width: 1,
                height: 1,
            };
            frame.render_widget(Paragraph::new(mark), cell);
        }
        next_y = y + item_height;
        for line in item.below {
            if next_y >= bottom {
                break;
            }
            let line_area = Rect {
                y: next_y,
                height: 1,
                ..text
            };
            // A click on what an entry opened into is a click on the entry.
            crate::mouse::record(
                Rect {
                    x: area.gutter_x,
                    width: text.width + 2,
                    ..line_area
                },
                crate::mouse::Target::Row(index),
            );
            frame.render_widget(Paragraph::new(line), line_area);
            next_y += 1;
        }
    }
    if window.clipped() {
        let y = next_y;
        if y < bottom {
            let note = Rect {
                y,
                height: 1,
                ..text
            };
            frame.render_widget(Paragraph::new(more_line(window, theme)), note);
        }
    }
}

/// `  ↓ 16 more` — how many entries are hidden, and in which direction.
fn more_line(window: ListWindow, theme: &Theme) -> Line<'static> {
    let mut parts = Vec::new();
    if window.below > 0 {
        parts.push(format!(
            "{} {} more",
            theme.glyphs.more_arrow(true),
            window.below
        ));
    }
    if window.above > 0 {
        parts.push(format!(
            "{} {} above",
            theme.glyphs.more_arrow(false),
            window.above
        ));
    }
    Line::from(Span::styled(
        format!("  {}", parts.join("   ")),
        theme.dim(),
    ))
}

/// Draw plain lines into a card, top-aligned and clipped to it (used for the
/// notes and empty states that are not lists).
pub(crate) fn draw_lines(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

/// A dim, word-wrapped message: what an empty card says.
pub(crate) fn draw_message(frame: &mut Frame, area: Rect, message: &str, theme: &Theme) {
    let lines: Vec<Line> = text::wrap(message, area.width as usize)
        .into_iter()
        .map(|line| Line::from(Span::styled(line, theme.dim())))
        .collect();
    draw_lines(frame, area, lines);
}

/// The style a severity is drawn in. The mark's shape already says which
/// severity it is; the colour only reinforces it.
pub(crate) fn severity_style(severity: Severity, theme: &Theme) -> Style {
    match severity {
        Severity::Ok => theme.dim(),
        Severity::Info => theme.dim(),
        Severity::Notice => theme.ahead(),
        Severity::Warn => theme.warn(),
        Severity::Risk => theme.risk(),
    }
}

/// How many columns a severity block takes: ` RISK `, ` WARN `, ` NOTE `.
pub(crate) const BLOCK_WIDTH: usize = 6;

/// The severity block that leads a row in the lists a person scans for
/// trouble (Do next, Work at risk, Repositories): filled for risk and
/// warning, quiet for the rest, blank when clean. See [`Theme::block`].
pub(crate) fn severity_block(severity: Severity, theme: &Theme) -> Span<'static> {
    let (word, style) = theme.block(severity);
    Span::styled(word, style)
}

/// The severity mark as a styled span.
pub(crate) fn severity_span(severity: Severity, theme: &Theme) -> Span<'static> {
    Span::styled(
        theme.glyphs.severity_mark(severity),
        severity_style(severity, theme),
    )
}
