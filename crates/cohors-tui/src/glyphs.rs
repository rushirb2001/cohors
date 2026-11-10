//! Terminal status glyphs, resolved once per run from the configured icon mode.
//!
//! cohors is a multi-user tool: it has to render on whatever terminal and font a
//! user happens to have, so every status glyph routes through here, in three
//! tiers:
//!   - **Unicode** (default): single-width geometric glyphs present in virtually
//!     all monospace fonts, coloured via ANSI.
//!   - **Ascii**: plain-text labels. Chosen automatically under `NO_COLOR` (where
//!     a colourless glyph can't carry its meaning) or when forced. Maximum
//!     portability.
//!   - **Nerd**: Nerd-Font icons — only when the user explicitly opts in, never
//!     assumed (most terminals don't ship a Nerd Font).

use cohors_config::IconMode;
use cohors_core::Severity;
use ratatui::symbols::border;

/// The resolved rendering tier (no `Auto` — that decision has been made).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    Ascii,
    Unicode,
    Nerd,
}

/// The glyph set in effect for this run.
#[derive(Debug, Clone, Copy)]
pub struct Glyphs {
    kind: IconKind,
}

impl Default for Glyphs {
    fn default() -> Self {
        // The safe baseline: Unicode glyphs render almost everywhere.
        Self {
            kind: IconKind::Unicode,
        }
    }
}

/// A card frame made of characters every terminal has.
const ASCII_BORDER: border::Set<'static> = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// The focused card's frame in the Ascii tier: `=` rules and `#` corners.
const ASCII_BORDER_HEAVY: border::Set<'static> = border::Set {
    top_left: "#",
    top_right: "#",
    bottom_left: "#",
    bottom_right: "#",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "=",
    horizontal_bottom: "=",
};

impl Glyphs {
    // ── The card dashboard ───────────────────────────────────────────────────
    // Every glyph the overview and the detail view draw goes through one of the
    // methods in this first group, so the Ascii tier never emits a character a
    // plain terminal cannot show.

    /// The severity mark that leads a row. The *shape* carries the meaning, so
    /// it still reads without colour: a triangle for work at risk, a diamond
    /// for a warning, a ring for something to note, a dot for clean.
    pub fn severity_mark(&self, severity: Severity) -> &'static str {
        match (self.kind, severity) {
            (IconKind::Ascii, Severity::Risk) => "!",
            (IconKind::Ascii, Severity::Warn) => "+",
            (IconKind::Ascii, Severity::Notice | Severity::Info) => "-",
            (IconKind::Ascii, Severity::Ok) => ".",
            (_, Severity::Risk) => "▲",
            (_, Severity::Warn) => "◆",
            (_, Severity::Notice | Severity::Info) => "○",
            (_, Severity::Ok) => "·",
        }
    }

    /// The gutter marker on the row the cursor is on.
    pub fn cursor(&self) -> &'static str {
        match self.kind {
            IconKind::Ascii => ">",
            IconKind::Unicode | IconKind::Nerd => "▸",
        }
    }

    /// The character that ends clipped text.
    pub fn ellipsis(&self) -> char {
        match self.kind {
            IconKind::Ascii => '~',
            IconKind::Unicode | IconKind::Nerd => '…',
        }
    }

    /// What a count of zero is drawn as: a small dot instead of the digit, so
    /// the numbers that mean something are the only numbers on the row.
    pub fn zero(&self) -> &'static str {
        match self.kind {
            IconKind::Ascii => ".",
            IconKind::Unicode | IconKind::Nerd => "·",
        }
    }

    /// One cell of a ranked bar (commits per repository).
    pub fn bar_cell(&self) -> &'static str {
        match self.kind {
            IconKind::Ascii => "#",
            IconKind::Unicode | IconKind::Nerd => "█",
        }
    }

    /// What ends a "still working" message: `Scanning…`.
    pub fn dots(&self) -> &'static str {
        match self.kind {
            IconKind::Ascii => "...",
            IconKind::Unicode | IconKind::Nerd => "…",
        }
    }

    /// A status message as this tier can show it. Messages are written once,
    /// with typographic punctuation (`fetching 3 repos…`, `skipped — reason`);
    /// the Ascii tier swaps those few characters for plain ones. Text that
    /// came from a repository (a name) is left exactly as it is.
    pub fn plain(&self, text: &str) -> String {
        if self.kind != IconKind::Ascii {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            match c {
                '…' => out.push_str("..."),
                '—' | '–' | '·' => out.push('-'),
                other => out.push(other),
            }
        }
        out
    }

    /// The arrow in "N more" when rows are hidden below (`true`) or above.
    pub fn more_arrow(&self, below: bool) -> &'static str {
        match (self.kind, below) {
            (IconKind::Ascii, true) => "v",
            (IconKind::Ascii, false) => "^",
            (_, true) => "↓",
            (_, false) => "↑",
        }
    }

    /// The small bullet before a plain list item (a task that is not started).
    pub fn bullet(&self) -> &'static str {
        match self.kind {
            IconKind::Ascii => "-",
            IconKind::Unicode | IconKind::Nerd => "·",
        }
    }

    /// The caret drawn after text that is being typed (the filter).
    pub fn caret(&self) -> &'static str {
        match self.kind {
            IconKind::Ascii => "_",
            IconKind::Unicode | IconKind::Nerd => "▏",
        }
    }

    /// How the Enter key is written in a key hint.
    pub fn key_enter(&self) -> &'static str {
        match self.kind {
            IconKind::Ascii => "enter",
            IconKind::Unicode | IconKind::Nerd => "⏎",
        }
    }

    /// How the space bar is written in a key hint.
    pub fn key_space(&self) -> &'static str {
        match self.kind {
            IconKind::Ascii => "space",
            IconKind::Unicode | IconKind::Nerd => "␣",
        }
    }

    /// How the up/down arrow keys are written in a key hint.
    pub fn key_updown(&self) -> &'static str {
        match self.kind {
            IconKind::Ascii => "up/dn",
            IconKind::Unicode | IconKind::Nerd => "↑↓",
        }
    }

    /// Commits ahead of / behind the upstream, e.g. `↑2 ↓1` (`+2 -1` in Ascii).
    pub fn ahead_behind(&self, ahead: u32, behind: u32) -> String {
        let (up, down) = match self.kind {
            IconKind::Ascii => ("+", "-"),
            IconKind::Unicode | IconKind::Nerd => ("↑", "↓"),
        };
        match (ahead, behind) {
            (a, 0) => format!("{up}{a}"),
            (0, b) => format!("{down}{b}"),
            (a, b) => format!("{up}{a} {down}{b}"),
        }
    }

    /// The icon in a card's title: 1 Do next, 2 Work at risk, 3 Open tasks,
    /// 4 Commits, 5 Repositories. Any other number has no icon.
    pub fn card_icon(&self, number: u8) -> &'static str {
        let (unicode, ascii) = match number {
            1 => ("▶", ">"),
            2 => ("▲", "!"),
            3 => ("✓", "v"),
            4 => ("●", "*"),
            5 => ("▤", "="),
            _ => ("", ""),
        };
        match self.kind {
            IconKind::Ascii => ascii,
            IconKind::Unicode | IconKind::Nerd => unicode,
        }
    }

    /// The box-drawing set for a card. A card has rounded corners; `heavy` asks
    /// for a visibly different frame, used to mark the focused card where
    /// colour is not available to do it.
    pub fn card_border(&self, heavy: bool) -> border::Set<'static> {
        match (self.kind, heavy) {
            (IconKind::Ascii, false) => ASCII_BORDER,
            (IconKind::Ascii, true) => ASCII_BORDER_HEAVY,
            (_, false) => border::ROUNDED,
            (_, true) => border::DOUBLE,
        }
    }

    /// The mark that leads an action's result: a tick, or a cross when it
    /// failed (`ok` / `!!` in the Ascii tier).
    pub fn result_mark(&self, failed: bool) -> &'static str {
        match (self.kind, failed) {
            (IconKind::Ascii, false) => "ok",
            (IconKind::Ascii, true) => "!!",
            (_, false) => "✓",
            (_, true) => "✗",
        }
    }

    /// One frame of the "working" spinner for animation tick `tick`.
    pub fn spinner(&self, tick: usize) -> &'static str {
        const BRAILLE: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        const ASCII: [&str; 4] = ["|", "/", "-", "\\"];
        match self.kind {
            IconKind::Ascii => ASCII[tick % ASCII.len()],
            IconKind::Unicode | IconKind::Nerd => BRAILLE[tick % BRAILLE.len()],
        }
    }

    /// The brand mark, or nothing in the Ascii tier (it is built from block
    /// elements that a plain terminal cannot draw).
    pub fn shows_brand_mark(&self) -> bool {
        self.kind != IconKind::Ascii
    }

    // ── Construction, and glyphs older than the cards ────────────────────────

    /// Resolve the configured [`IconMode`] against the terminal's colour support.
    /// `Auto` picks Unicode, but drops to Ascii when colour is off — a colourless
    /// `●` can't say synced-vs-anything, so a plain word reads better. `Nerd` is
    /// only ever chosen when the user explicitly asked for it.
    pub fn resolve(mode: IconMode, no_color: bool) -> Self {
        let kind = match mode {
            IconMode::Ascii => IconKind::Ascii,
            IconMode::Unicode => IconKind::Unicode,
            IconMode::Nerd => IconKind::Nerd,
            IconMode::Auto if no_color => IconKind::Ascii,
            IconMode::Auto => IconKind::Unicode,
        };
        Self { kind }
    }

    /// The selection-gutter marker for a repo marked for a bulk action. A filled
    /// dot in the glyph tiers, an asterisk under Ascii (where `●` may be missing).
    pub fn marked(&self) -> &'static str {
        match self.kind {
            IconKind::Ascii => "*",
            IconKind::Unicode | IconKind::Nerd => "●",
        }
    }

    /// The 8-level ramp for activity sparklines (fleet heartbeat + per-repo
    /// cadence), lowest to highest. Block elements in the glyph tiers; a plain
    /// ASCII density ramp otherwise — the *shape* carries the reading either
    /// way, so no colour is required.
    pub fn spark_ramp(&self) -> &'static [char; 8] {
        match self.kind {
            IconKind::Unicode | IconKind::Nerd => &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'],
            IconKind::Ascii => &['.', ':', '-', '=', '+', '*', '#', '@'],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_drops_to_ascii_without_colour() {
        assert_eq!(Glyphs::resolve(IconMode::Auto, false).marked(), "●");
        assert_eq!(Glyphs::resolve(IconMode::Auto, true).marked(), "*");
    }

    #[test]
    fn explicit_modes_ignore_colour() {
        assert_eq!(Glyphs::resolve(IconMode::Unicode, true).marked(), "●");
        assert_eq!(Glyphs::resolve(IconMode::Ascii, false).marked(), "*");
        // Nerd is only ever chosen explicitly, and draws the same shapes here.
        assert_eq!(Glyphs::resolve(IconMode::Nerd, true).marked(), "●");
    }

    #[test]
    fn status_messages_lose_their_typographic_punctuation_in_ascii() {
        let message = "fetching 3 repos…  ·  1 skipped — conflict";
        let ascii = Glyphs::resolve(IconMode::Ascii, false);
        assert_eq!(
            ascii.plain(message),
            "fetching 3 repos...  -  1 skipped - conflict"
        );
        // The other tiers show it as written, and a repository's own name is
        // never rewritten, whatever it contains.
        assert_eq!(
            Glyphs::resolve(IconMode::Unicode, false).plain(message),
            message
        );
        assert_eq!(ascii.plain("pushed café"), "pushed café");
    }

    #[test]
    fn the_spinner_cycles_and_has_an_ascii_form() {
        let ascii = Glyphs::resolve(IconMode::Ascii, false);
        let frames: Vec<&str> = (0..8).map(|tick| ascii.spinner(tick)).collect();
        assert!(frames.iter().all(|frame| frame.is_ascii()));
        assert_eq!(frames[0], frames[4]); // four frames, then around again
        let unicode = Glyphs::resolve(IconMode::Unicode, false);
        assert_ne!(unicode.spinner(0), unicode.spinner(1));
        // Any tick is a valid frame: the index wraps rather than overflowing.
        let _ = unicode.spinner(usize::MAX);
    }

    #[test]
    fn severity_reads_as_a_shape_in_every_tier() {
        let unicode = Glyphs::resolve(IconMode::Unicode, false);
        let ascii = Glyphs::resolve(IconMode::Ascii, false);
        let all = [
            Severity::Risk,
            Severity::Warn,
            Severity::Notice,
            Severity::Ok,
        ];
        // Four distinct marks per tier, and the Ascii ones really are ASCII.
        for glyphs in [unicode, ascii] {
            let marks: std::collections::HashSet<_> =
                all.iter().map(|s| glyphs.severity_mark(*s)).collect();
            assert_eq!(marks.len(), all.len());
        }
        assert!(all.iter().all(|s| ascii.severity_mark(*s).is_ascii()));
        assert_eq!(unicode.severity_mark(Severity::Risk), "▲");
        // Info and Notice share the ring: both mean "worth a look, not urgent".
        assert_eq!(
            unicode.severity_mark(Severity::Info),
            unicode.severity_mark(Severity::Notice)
        );
    }

    #[test]
    fn the_ascii_tier_draws_cards_with_plain_characters() {
        let ascii = Glyphs::resolve(IconMode::Ascii, false);
        for heavy in [false, true] {
            let set = ascii.card_border(heavy);
            for piece in [set.top_left, set.horizontal_top, set.vertical_left] {
                assert!(piece.is_ascii(), "{piece}");
            }
        }
        for number in 1..=5 {
            assert!(ascii.card_icon(number).is_ascii());
        }
        assert!(ascii.key_enter().is_ascii() && ascii.key_space().is_ascii());
        assert!(ascii.ellipsis().is_ascii() && ascii.bar_cell().is_ascii());
        assert!(ascii.cursor().is_ascii() && ascii.caret().is_ascii());
        assert!(!ascii.shows_brand_mark());
        // The focused frame differs from the plain one, so focus shows without colour.
        assert_ne!(
            ascii.card_border(true).horizontal_top,
            ascii.card_border(false).horizontal_top
        );
    }

    #[test]
    fn ahead_behind_names_only_the_sides_that_moved() {
        let unicode = Glyphs::resolve(IconMode::Unicode, false);
        assert_eq!(unicode.ahead_behind(2, 0), "↑2");
        assert_eq!(unicode.ahead_behind(0, 3), "↓3");
        assert_eq!(unicode.ahead_behind(2, 3), "↑2 ↓3");
        let ascii = Glyphs::resolve(IconMode::Ascii, false);
        assert_eq!(ascii.ahead_behind(2, 3), "+2 -3");
    }
}
