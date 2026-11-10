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

/// What to draw around a card's content.
#[derive(Default)]
pub(crate) struct CardFrame {
    /// The number key that focuses the card, shown in the title.
    pub number: Option<u8>,
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
    let border_style = theme.card_line(card.number, card.focused);

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
        let band = Style::new().bg(theme.band(card.focused));
        frame.buffer_mut().set_style(top, band);
        let line = title_line(card, " ", theme.dim(), theme);
        frame.render_widget(Paragraph::new(line), top);
    }
    if !card.hints.is_empty() {
        frame
            .buffer_mut()
            .set_style(bottom, Style::new().bg(surface));
        let line = hints_line(&card.hints, area.width, " ", Style::new(), theme);
        frame.render_widget(Paragraph::new(line), bottom);
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
    // A scheme may restyle the whole title (its colour, or a solid label).
    if let Some(over) = theme.card_title(card.number, card.focused) {
        for span in &mut label {
            span.style = span.style.patch(over);
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
    // Corners (2), the leading rule (1), two spaces each side of the hints,
    // and at least one rule before the right corner.
    let mut room = (card_width as usize).saturating_sub(2 + 1 + 2 + 2 + 1);
    let key_style = theme.hint_base().add_modifier(Modifier::BOLD);
    let label_style = theme.hint_base().patch(theme.dim());
    let mut spans = vec![Span::styled(rule.to_string(), border), Span::raw("  ")];
    let mut shown = 0;
    for hint in hints {
        let gap = if shown == 0 { 0 } else { 3 };
        let width = gap + hint.key.chars().count() + 2 + hint.label.chars().count();
        if width > room {
            break;
        }
        room -= width;
        if gap > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(hint.key.clone(), key_style));
        spans.push(Span::styled(format!("  {}", hint.label), label_style));
        shown += 1;
    }
    if shown == 0 {
        // Not even one hint fits: leave the border plain.
        return Line::from(Span::styled(rule.to_string(), border));
    }
    spans.push(Span::raw("  "));
    Line::from(spans)
}

/// One entry of a list: its line(s) of text, and optionally a mark for the
/// gutter (a "marked for a bulk action" dot, or a spinner).
pub(crate) struct Item {
    pub lines: Vec<Line<'static>>,
    pub gutter: Option<Span<'static>>,
}

impl Item {
    pub(crate) fn line(line: Line<'static>) -> Self {
        Self {
            lines: vec![line],
            gutter: None,
        }
    }
}

/// Draw the visible part of a list into a card.
///
/// `window` says which entries are visible, `item_height` how many lines each
/// takes, and `cursor` which entry (if any) the cursor is on. `build` is only
/// called for the entries that are actually drawn.
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
    for slot in 0..window.shown {
        let index = window.offset + slot;
        let y = text.y + slot as u16 * item_height;
        if y + item_height > text.y + text.height {
            break; // never draw outside the card
        }
        let item = build(index);
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
    }
    if window.clipped() {
        let y = text.y + window.shown as u16 * item_height;
        if y < text.y + text.height {
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
