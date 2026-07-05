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
    /// The docked context pane's card wash: a light grey background with a
    /// dark base foreground. Setting BOTH keeps the card readable on any
    /// terminal theme (spans that pick their own fg still patch over it).
    /// Dropped entirely under NO_COLOR.
    fn dock_card(&self) -> Style {
        if self.color {
            Style::new().bg(Color::Indexed(255)).fg(Color::Indexed(235))
        } else {
            Style::new()
        }
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
    render_header(frame, header_area, app, &theme);
    render_footer(frame, footer_area, app, &theme);

    if app.repos.is_empty() {
        if app.scanning {
            render_loading(frame, body_area, app);
        } else {
            render_empty(frame, body_area, app, &theme);
        }
    } else {
        // A strip on top — the fuzzy input while filtering, otherwise the
        // Attention panel — then the Repositories panel, which hosts the context
        // pane inside its own box (bottom rows) when the terminal is tall enough.
        // With vertical room the attention panel is its full-width card (severity
        // bar + bucket counts, two body rows); on a short terminal it collapses
        // to a single bucket line.
        let strip_height = if app.mode == Mode::Filter {
            1
        } else if body_area.height >= 14 {
            4
        } else {
            3
        };
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
