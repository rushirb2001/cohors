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

impl Glyphs {
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

    /// The "in sync with upstream" indicator: a glyph in the glyph tiers, a
    /// steady, self-describing word under Ascii (where colour can't carry it).
    pub fn synced(&self) -> &'static str {
        match self.kind {
            IconKind::Ascii => "ok",
            IconKind::Unicode | IconKind::Nerd => "●",
        }
    }

    /// Whether the synced indicator should blink. Only the glyph tiers blink; the
    /// Ascii word stays steady — blinking text is noise, and an animated indicator
    /// must never be the *only* channel a state reads on.
    pub fn blink_synced(&self) -> bool {
        matches!(self.kind, IconKind::Unicode | IconKind::Nerd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_drops_to_ascii_without_colour() {
        assert_eq!(Glyphs::resolve(IconMode::Auto, false).synced(), "●");
        assert_eq!(Glyphs::resolve(IconMode::Auto, true).synced(), "ok");
    }

    #[test]
    fn explicit_modes_ignore_colour() {
        assert_eq!(Glyphs::resolve(IconMode::Unicode, true).synced(), "●");
        assert_eq!(Glyphs::resolve(IconMode::Ascii, false).synced(), "ok");
    }

    #[test]
    fn only_glyph_tiers_blink() {
        assert!(Glyphs::resolve(IconMode::Unicode, false).blink_synced());
        assert!(Glyphs::resolve(IconMode::Nerd, false).blink_synced());
        assert!(!Glyphs::resolve(IconMode::Ascii, false).blink_synced());
    }
}
