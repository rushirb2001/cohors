//! Rendering. Pure view-model → widgets: every frame derives the visible rows
//! from [`App::view`] (i.e. `cohors-core`) and maps them onto ratatui widgets.
//! No state is mutated here.

pub(crate) use cohors_core::{
    AttentionReason, Branch, CiStatus, FleetSummary, RepoSnapshot, Severity, StandupCommit, assess,
    fleet_summary, time,
};
pub(crate) use ratatui::Frame;
pub(crate) use ratatui::buffer::Buffer;
pub(crate) use ratatui::layout::{Alignment, Constraint, Layout, Rect};
pub(crate) use ratatui::style::{Color, Modifier, Style, Stylize};
pub(crate) use ratatui::text::{Line, Span, Text};
pub(crate) use ratatui::widgets::{
    Block, BorderType, Cell, Clear, List, ListItem, ListState, Padding, Paragraph, Row, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Table, TableState, Wrap,
};

pub(crate) use crate::app::{App, ConfirmAction, Mode, Opener, RunState};
pub(crate) use crate::glyphs::Glyphs;

/// Spinner frames (braille) for the scan indicator.
pub(crate) const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(crate) fn spinner_frame(tick: usize) -> &'static str {
    SPINNER[tick % SPINNER.len()]
}

/// Color policy. Colors are dropped when `NO_COLOR` is set; structural
/// modifiers (dim/bold/reversed) are kept so the layout still reads. Also carries
/// the resolved [`Glyphs`] so every cell renders glyphs through one source of
/// truth (with ASCII fallback under `NO_COLOR`).
pub(crate) struct Theme {
    color: bool,
    /// Whether the terminal has a dark background, so the cards flip to a dark
    /// wash (letting the native accent colours read) instead of a light one.
    dark: bool,
    glyphs: Glyphs,
}

impl Theme {
    fn from_env(icons: cohors_config::IconMode, dark: bool) -> Self {
        let color = std::env::var_os("NO_COLOR").is_none();
        Self {
            color,
            dark,
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
    /// The docked context pane's card wash: a light grey background with a
    /// dark base foreground. Setting BOTH keeps the card readable on any
    /// terminal theme (spans that pick their own fg still patch over it).
    /// Dropped entirely under NO_COLOR.
    fn dock_card(&self) -> Style {
        if !self.color {
            Style::new()
        } else if self.dark {
            // Dark terminal: a dark-grey card with a light base fg, so the native
            // accent colours (green / yellow / cyan) read as they do off-card.
            Style::new().bg(Color::Indexed(236)).fg(Color::Indexed(253))
        } else {
            Style::new().bg(Color::Indexed(255)).fg(Color::Indexed(235))
        }
    }

    /// The footer hint bar's card wash: the same idea as [`Self::dock_card`], but
    /// nudged toward the brand — a faint purple (~6% `SPIDER_PURPLE` over white),
    /// with the same dark base fg so keys/labels stay readable on any terminal
    /// theme (accent spans patch their own colour over it). Dropped under NO_COLOR.
    fn hint_card(&self) -> Style {
        if !self.color {
            Style::new()
        } else if self.dark {
            // Dark terminal: a dark purple wash + light base fg, so the native
            // accent colours read while the brand tint stays present.
            Style::new()
                .bg(Color::Rgb(0x2E, 0x27, 0x3E))
                .fg(Color::Indexed(253))
        } else {
            Style::new()
                .bg(Color::Rgb(0xFA, 0xF5, 0xFF))
                .fg(Color::Indexed(235))
        }
    }

    /// The selected-row wash: a soft, dark-adaptive brand tint rather than a full
    /// reverse — so a multi-line (multi-branch) row doesn't become a harsh block,
    /// and dim cells (PRs, activity) stay legible on the highlight (reverse turns
    /// dim text invisible). Under NO_COLOR it falls back to reverse, the only
    /// structural way to mark a row.
    fn selection(&self) -> Style {
        if !self.color {
            Style::new().add_modifier(Modifier::REVERSED)
        } else if self.dark {
            Style::new().bg(Color::Rgb(0x39, 0x30, 0x54))
        } else {
            Style::new().bg(Color::Rgb(0xE7, 0xDE, 0xF9))
        }
    }
}

/// Render the whole dashboard for one frame. `now` (Unix seconds) is injected
/// so relative commit ages are deterministic in tests.
pub fn render(frame: &mut Frame, app: &App, now: i64) {
    let theme = Theme::from_env(app.icons, app.dark);
    let area = frame.area();

    // Top-level layout: the branded header box, the body, and a boxed key-hint
    // footer — one labelled group per row, each wrapping if the terminal is
    // narrow, so no command is ever truncated.
    let footer_h = footer_height(app, area.width);
    let [header_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Min(0),
        Constraint::Length(footer_h),
    ])
    .areas(area);
    render_header(frame, header_area, app, &theme);
    render_footer(frame, footer_area, app, &theme);

    if app.repos.is_empty() {
        if app.scanning {
            render_loading(frame, body_area, app);
        } else {
            render_empty(frame, body_area, app, &theme);
        }
    } else {
        // The Attention panel on top, then the Repositories panel (which hosts the
        // context pane, and the fuzzy filter input, inside its own box). With
        // vertical room the attention panel is its full-width card (severity bar +
        // bucket counts); on a short terminal it collapses to a single bucket line.
        // The attention card stays put while filtering — only the filter input box
        // appears (inside the repos panel), so `/` doesn't reshuffle the layout.
        let strip_height = if body_area.height >= 14 { 4 } else { 3 };
        let [strip, rest] =
            Layout::vertical([Constraint::Length(strip_height), Constraint::Min(0)])
                .areas(body_area);
        render_attention_panel(frame, strip, app, now, &theme);
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
pub(crate) fn dim_area(buf: &mut Buffer, area: Rect) {
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
pub(crate) const SPIDER_PURPLE: Color = Color::Rgb(0xA8, 0x55, 0xF7);

/// The cohors mark — three rows of block "pixels" (width 9). Shared by the TUI
/// header and the CLI's `cohors web` setup hint so the brand reads the same
/// everywhere. Purple where colour is available; the silhouette still reads
/// under `NO_COLOR`.
pub(crate) const SPIDER_MARK: [&str; 3] = ["▜▒▟███▙▒▛", "▟██▌█▐██▙", "▀▐▖▀█▀▗▌▀"];

/// The version string shown in the UI. Fixed under `#[cfg(test)]` so snapshot
/// layouts don't shift when the *length* of the real version changes (e.g.
/// `0.5.9` → `0.5.10` is one column wider); real builds show the crate version.
pub(crate) fn version_str() -> &'static str {
    #[cfg(test)]
    {
        "X.Y.Z"
    }
    #[cfg(not(test))]
    {
        env!("CARGO_PKG_VERSION")
    }
}

/// The brand purple as an RGB triple, for surfaces that emit their own ANSI
/// (the CLI) rather than going through ratatui's [`Theme`]. Kept in lockstep
/// with [`SPIDER_PURPLE`] by construction.
pub(crate) const SPIDER_PURPLE_RGB: (u8, u8, u8) = (0xA8, 0x55, 0xF7);

// The UI is one module split across focused files: `chrome` (header, footer,
// attention panel, command box), `rows` (the repos table, its cells, and the
// dock), and `modals` (detail, help, standup, confirm, run output). Items are
// pub(crate) and re-exported here so the split stays invisible to callers.
mod chrome;
mod modals;
mod rows;
#[cfg(test)]
mod tests;

pub(crate) use chrome::*;
pub(crate) use modals::*;
pub(crate) use rows::*;
