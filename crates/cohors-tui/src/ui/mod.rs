//! Rendering. Pure view-model → widgets: every frame reads the state in
//! [`App`] (the shared overview model and the repository snapshots) and maps it
//! onto ratatui widgets. Apart from remembered scroll offsets, no state is
//! mutated here.
//!
//! The dashboard is two screens with overlays on top:
//!
//! - `overview` — the screen cohors opens on: header, key figures, five cards.
//! - `detail_list` — every repository as a two-line list, plus three cards
//!   about the selected one.
//! - `modals`, `launchpad`, `chrome` — the overlays (help, standup, the full
//!   repository drill-in, confirmations, the command line) and their frames.
//!
//! Shared pieces: `cards` (the card frame and scrolling lists), `repos_table`
//! (the repository table both screens use), `layout` (pure sizing decisions)
//! and `text` (pure clipping and wording helpers).

pub(crate) use cohors_core::{Branch, RepoSnapshot, Severity, StandupCommit, assess, time};
pub(crate) use ratatui::Frame;
pub(crate) use ratatui::buffer::Buffer;
pub(crate) use ratatui::layout::{Alignment, Constraint, Layout, Rect};
pub(crate) use ratatui::style::{Color, Modifier, Style, Stylize};
pub(crate) use ratatui::text::{Line, Span, Text};
pub(crate) use ratatui::widgets::{
    Block, BorderType, Clear, List, ListItem, ListState, Padding, Paragraph, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Wrap,
};

pub(crate) use crate::app::{App, ConfirmAction, Mode, Opener, RunState};
pub(crate) use crate::glyphs::Glyphs;

use crate::app::Screen;

/// Color policy. Colors are dropped when `NO_COLOR` is set; structural
/// modifiers (dim/bold/reversed) are kept so the layout still reads. Also carries
/// the resolved [`Glyphs`] so every cell renders glyphs through one source of
/// truth (with ASCII fallback under `NO_COLOR`).
pub(crate) struct Theme {
    color: bool,
    /// Whether the terminal has a dark background, so washes flip to a dark
    /// tint (letting the native accent colours read) instead of a light one.
    dark: bool,
    glyphs: Glyphs,
}

impl Theme {
    /// The theme for this run: colour is on unless `NO_COLOR` is set.
    fn from_env(icons: cohors_config::IconMode, dark: bool) -> Self {
        Self::new(icons, dark, std::env::var_os("NO_COLOR").is_none())
    }

    /// A theme with colour explicitly on or off (tests use this to render the
    /// `NO_COLOR` variant without touching the environment).
    pub(crate) fn new(icons: cohors_config::IconMode, dark: bool, color: bool) -> Self {
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

    /// The accent: the brand purple. It marks the focused card, the keys that
    /// act, and the bars. Dropped under `NO_COLOR`, where shape does the work.
    fn accent(&self) -> Style {
        self.fg(SPIDER_PURPLE)
    }

    /// Bronze — the colour of structure in the overlays: titles and section
    /// headings. A lighter bronze on dark terminals, a deeper one on light.
    fn bronze(&self) -> Style {
        if self.dark {
            self.fg(Color::Rgb(0xC8, 0xA0, 0x64))
        } else {
            self.fg(Color::Rgb(0x8F, 0x6A, 0x2A))
        }
    }

    /// The solid badge that leads a risk or warning status (`None` for milder
    /// severities). Uses the terminal's own red/yellow so it follows the
    /// user's palette; under `NO_COLOR` it falls back to reverse video, which
    /// still reads as a block.
    fn badge(&self, sev: Severity) -> Option<(&'static str, Style)> {
        let (word, bg) = match sev {
            Severity::Risk => (" RISK ", Color::Red),
            Severity::Warn => (" WARN ", Color::Yellow),
            _ => return None,
        };
        let style = if self.color {
            Style::new().bg(bg).fg(Color::Black)
        } else {
            Style::new().add_modifier(Modifier::REVERSED)
        };
        Some((word, style.add_modifier(Modifier::BOLD)))
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

    /// The cursor row's wash: a soft, dark-adaptive brand tint rather than a
    /// full reverse, so dim cells stay legible on it (reverse turns dim text
    /// invisible). Under `NO_COLOR` it falls back to reverse, the only
    /// structural way to mark a row.
    fn selection(&self) -> Style {
        if !self.color {
            Style::new().add_modifier(Modifier::REVERSED)
        } else if self.dark {
            Style::new().bg(Color::Rgb(0x4A, 0x2C, 0x47))
        } else {
            Style::new().bg(Color::Rgb(0xEA, 0xD3, 0xE4))
        }
    }
}

/// Render the whole dashboard for one frame. `now` (Unix seconds) is injected
/// so relative commit ages are deterministic in tests.
pub fn render(frame: &mut Frame, app: &App, now: i64) {
    let theme = Theme::from_env(app.icons, app.dark);
    render_with(frame, app, now, &theme);
}

/// [`render`] with an explicit theme.
pub(crate) fn render_with(frame: &mut Frame, app: &App, now: i64, theme: &Theme) {
    let area = frame.area();
    // Too small to lay anything out sensibly: say so, rather than garble.
    if !layout::fits(area.width, area.height) {
        render_too_small(frame, area, theme);
        return;
    }

    match app.screen {
        Screen::Overview => overview::render_overview(frame, area, app, now, theme),
        Screen::Repos => detail_list::render_detail_view(frame, area, app, now, theme),
    }

    // Dim the whole frame behind a modal overlay so the background recedes and
    // the overlay stands out. The overlays `Clear` their own area, so they
    // render crisp on top of the dimmed background.
    let overlay_open = !matches!(app.mode, Mode::Normal | Mode::Filter);
    if overlay_open {
        dim_area(frame.buffer_mut(), area);
    }

    match app.mode {
        Mode::Normal | Mode::Filter => {}
        Mode::Help => render_help(frame, area, app, theme),
        Mode::Standup => render_standup(frame, area, app, theme),
        Mode::Command => render_command_box(frame, area, app, theme),
        Mode::CommandRun => render_command_run(frame, area, app, theme),
        Mode::Confirm => render_confirm(frame, area, app, theme),
        Mode::OpenWith => render_open_with(frame, area, app, theme),
        Mode::PushBranch => render_push_branch(frame, area, app, theme),
        Mode::Detail => render_detail(frame, area, app, now, theme),
        Mode::Launchpad => render_launchpad(frame, area, app, theme),
    }
}

/// What a terminal too small for the dashboard shows instead of it.
fn render_too_small(frame: &mut Frame, area: Rect, theme: &Theme) {
    let lines = vec![
        Line::from(Span::styled(
            "Terminal too small",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(
                "cohors needs {}x{}, this is {}x{}.",
                layout::MIN_WIDTH,
                layout::MIN_HEIGHT,
                area.width,
                area.height
            ),
            theme.dim(),
        )),
        Line::from(Span::styled(
            "Enlarge the window, or q to quit.",
            theme.dim(),
        )),
    ];
    let height = (lines.len() as u16).min(area.height);
    let centered = Rect {
        y: area.y + (area.height - height) / 2,
        height,
        ..area
    };
    frame.render_widget(
        Paragraph::new(Text::from(lines)).alignment(Alignment::Center),
        centered,
    );
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

/// The brand colour for the spider mark: Tyrian purple, lifted enough to read
/// on both dark and light terminals. Dropped under `NO_COLOR` via [`Theme::fg`].
pub(crate) const SPIDER_PURPLE: Color = Color::Rgb(0xB5, 0x5C, 0xA4);

/// The frame of every overlay: a double rule, which sets an overlay apart
/// from the rounded cards underneath it. Rules drawn inside a frame join it
/// with `╟ ╢ ╤ ╧`.
pub(crate) const FRAME: BorderType = BorderType::Double;

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
pub(crate) const SPIDER_PURPLE_RGB: (u8, u8, u8) = (0xB5, 0x5C, 0xA4);

mod cards;
mod chrome;
mod detail_list;
mod launchpad;
mod layout;
mod modals;
mod overview;
mod repos_table;
#[cfg(test)]
mod tests;
mod text;

pub(crate) use chrome::*;
pub(crate) use launchpad::{launchpad_line_of, render_launchpad};
pub(crate) use modals::*;
pub(crate) use text::abbrev_home;
