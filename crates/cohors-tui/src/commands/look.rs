//! How the plain-terminal screens look: the launch screen, the welcome, the
//! action guides and every report (`scan`, `overview`, `discover`, …).
//!
//! Those screens are built from five pieces — the masthead, section headings,
//! guide rows, severity marks and the key bar — so the look lives here, in one
//! place, and every screen keeps its layout whichever look is on. A [`Look`]
//! is three things:
//!
//! - a [`Style`]: the visual treatment (`card`, `panel`, `quiet`, or the
//!   original `classic`);
//! - whether to draw with ASCII only (no box drawing, blocks or arrows): set by
//!   `icons = "ascii"` / `COHORS_ICONS=ascii`, by `NO_COLOR` (the dashboard's
//!   own rule: without colour, shapes must carry the meaning), or by a locale
//!   that is not UTF-8;
//! - the width rules and boxes are drawn to.
//!
//! It is chosen once per process ([`init`]) and read through a thread-local,
//! so tests can render any look on their own thread without racing.

use std::cell::Cell;

use super::{bold, brand, clip, dim, sgr};

/// The visual treatment of the plain-terminal screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// The original: plain text, colour only.
    Classic,
    /// The masthead in a rounded box, accent-barred headings, the keys in a
    /// box of their own.
    Card,
    /// A filled header band like the dashboard's, headings as full-width
    /// rules, keys as keycaps.
    Panel,
    /// Editorial: a hairline under the masthead, letter-spaced headings,
    /// dotted leaders.
    Quiet,
}

impl Style {
    /// Every style, in the order they are offered.
    pub const ALL: [Style; 4] = [Style::Card, Style::Panel, Style::Quiet, Style::Classic];

    pub fn name(self) -> &'static str {
        match self {
            Style::Classic => "classic",
            Style::Card => "card",
            Style::Panel => "panel",
            Style::Quiet => "quiet",
        }
    }

    pub fn parse(name: &str) -> Option<Style> {
        Style::ALL
            .into_iter()
            .find(|s| s.name().eq_ignore_ascii_case(name.trim()))
    }
}

/// A style, the glyph set and the drawing width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    pub style: Style,
    /// Draw with ASCII only.
    pub ascii: bool,
    /// The width rules and boxes are drawn to (the terminal's, capped).
    pub width: usize,
}

/// The widest a rule or box is drawn: past this a screen of short lines
/// reads as a few words lost in a long frame.
const MAX_WIDTH: usize = 96;

/// Until the user picks one, the look a screen gets when nothing says otherwise.
pub const DEFAULT_STYLE: Style = Style::Classic;

thread_local! {
    static LOOK: Cell<Option<Look>> = const { Cell::new(None) };
}

/// Choose the look for this process from the environment and the configured
/// icon mode. `COHORS_LOOK` picks the style.
pub fn init(icons: cohors_config::IconMode) {
    let style = std::env::var("COHORS_LOOK")
        .ok()
        .and_then(|v| Style::parse(&v))
        .unwrap_or(DEFAULT_STYLE);
    let ascii = icons == cohors_config::IconMode::Ascii
        || std::env::var("COHORS_ICONS").is_ok_and(|v| v.eq_ignore_ascii_case("ascii"))
        || std::env::var_os("NO_COLOR").is_some()
        || !utf8_locale();
    set(Look {
        style,
        ascii,
        width: super::term_width().min(MAX_WIDTH),
    });
}

/// Is the locale UTF-8? The first of `LC_ALL`, `LC_CTYPE`, `LANG` that is set
/// decides; with none set (common on macOS terminals) UTF-8 is assumed.
fn utf8_locale() -> bool {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .is_none_or(|v| {
            let v = v.to_ascii_lowercase();
            v.contains("utf-8") || v.contains("utf8")
        })
}

pub fn set(look: Look) {
    LOOK.with(|l| l.set(Some(look)));
}

/// The current look (the default style, Unicode, 80 wide when never set).
pub fn get() -> Look {
    LOOK.with(|l| l.get()).unwrap_or(Look {
        style: DEFAULT_STYLE,
        ascii: false,
        width: 80,
    })
}

/// Run `f` under `look`, then restore the previous one (for tests).
#[cfg(test)]
pub fn with<T>(look: Look, f: impl FnOnce() -> T) -> T {
    let before = LOOK.with(|l| l.get());
    set(look);
    let out = f();
    LOOK.with(|l| l.set(before));
    out
}

/// The glyphs a look draws with, in one place so ASCII is never forgotten.
struct Ink {
    h: &'static str,
    v: &'static str,
    tl: &'static str,
    tr: &'static str,
    bl: &'static str,
    br: &'static str,
    hair: &'static str,
    bullet: &'static str,
    dot: &'static str,
    accent: &'static str,
}

fn ink(ascii: bool) -> Ink {
    if ascii {
        Ink {
            h: "-",
            v: "|",
            tl: "+",
            tr: "+",
            bl: "+",
            br: "+",
            hair: "-",
            bullet: ">",
            dot: ".",
            accent: "|",
        }
    } else {
        Ink {
            h: "─",
            v: "│",
            tl: "╭",
            tr: "╮",
            bl: "╰",
            br: "╯",
            hair: "┄",
            bullet: "›",
            dot: "·",
            accent: "▌",
        }
    }
}

/// Visible columns of `s`, ignoring ANSI SGR sequences.
fn visible(s: &str) -> usize {
    let mut n = 0;
    let mut in_esc = false;
    for c in s.chars() {
        match (in_esc, c) {
            (false, '\x1b') => in_esc = true,
            (true, 'm') => in_esc = false,
            (true, _) => {}
            (false, _) => n += 1,
        }
    }
    n
}

/// Sentence case, like every heading in the dashboard and the browser.
fn sentence(heading: &str) -> String {
    let mut chars = heading.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `D O   N E X T`: letters spaced, words three apart.
fn spaced_caps(heading: &str) -> String {
    heading
        .split_whitespace()
        .map(|w| {
            w.to_uppercase()
                .chars()
                .map(String::from)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("   ")
}

/// The report's masthead: the mark beside the wordmark, a headline, and where
/// it looked.
pub fn masthead(scanned: &str, where_at: &str, color: bool) -> String {
    let look = get();
    let ink = ink(look.ascii);
    let w = look.width.max(40);
    let mark = color && !look.ascii;
    let version = format!("v{}", crate::ui::version_str());
    let m = &crate::ui::SPIDER_MARK;
    match look.style {
        Style::Classic => classic_masthead(scanned, where_at, color, w),
        Style::Card => {
            // A rounded box `w` wide around the mark and the three lines.
            let inner = w - 4; // "  │" … "│"
            let lead = if mark { 11 } else { 0 }; // mark + two spaces
            let text_w = inner - 2 - lead;
            let line = |glyph: &str, text: String, right: &str| {
                let room = text_w.saturating_sub(right.chars().count() + 1);
                let text = clip(&text, room);
                let pad = text_w - visible(&text) - right.chars().count();
                let lead = if mark {
                    format!("{}  ", brand(color, glyph))
                } else {
                    String::new()
                };
                format!(
                    "  {} {lead}{text}{}{} {}\n",
                    dim(color, ink.v),
                    " ".repeat(pad),
                    dim(color, right),
                    dim(color, ink.v)
                )
            };
            let mut out = String::from("\n");
            out.push_str(&format!(
                "  {}\n",
                dim(
                    color,
                    &format!("{}{}{}", ink.tl, ink.h.repeat(inner), ink.tr)
                )
            ));
            out.push_str(&line(m[0], brand(color, "cohors"), &version));
            out.push_str(&line(m[1], bold(color, scanned), ""));
            out.push_str(&line(m[2], dim(color, where_at), ""));
            out.push_str(&format!(
                "  {}\n\n",
                dim(
                    color,
                    &format!("{}{}{}", ink.bl, ink.h.repeat(inner), ink.br)
                )
            ));
            out
        }
        Style::Panel => {
            if !color {
                // No colour, no band: a ruled title instead.
                let title = format!("{0}{0} cohors {1} ", ink.h, version);
                let rule = ink.h.repeat((w - 2).saturating_sub(title.chars().count()));
                return format!(
                    "\n  {title}{rule}\n  {}\n  {}\n  {}\n\n",
                    clip(scanned, w - 2),
                    clip(where_at, w - 2),
                    ink.h.repeat(w - 2)
                );
            }
            // A filled band, the dashboard's header in the plain terminal: a
            // deep plum that reads on dark and light backgrounds alike.
            let band = |text: String| {
                let pad = w.saturating_sub(visible(&text));
                format!(
                    "\x1b[48;2;58;34;54m{}{}\x1b[0m\n",
                    text.replace("\x1b[0m", "\x1b[0m\x1b[48;2;58;34;54m"),
                    " ".repeat(pad)
                )
            };
            let light = |s: &str| sgr(true, "38;2;243;230;239", s);
            let room = w.saturating_sub(16);
            let mut out = String::from("\n");
            let gap = w.saturating_sub(2 + 9 + 3 + 6 + version.len() + 2);
            out.push_str(&band(format!(
                "  {}   {}{}{}  ",
                sgr(true, "38;2;224;154;212", m[0]),
                sgr(true, "1;38;2;243;230;239", "cohors"),
                " ".repeat(gap),
                sgr(true, "38;2;190;160;184", &version)
            )));
            out.push_str(&band(format!(
                "  {}   {}",
                sgr(true, "38;2;224;154;212", m[1]),
                light(&clip(scanned, room))
            )));
            out.push_str(&band(format!(
                "  {}   {}",
                sgr(true, "38;2;224;154;212", m[2]),
                sgr(true, "38;2;190;160;184", &clip(where_at, room))
            )));
            out.push('\n');
            out
        }
        Style::Quiet => {
            let mut out = String::from("\n");
            let head = if mark {
                format!("{}   {}", brand(color, m[0]), brand(color, "cohors"))
            } else {
                bold(color, "cohors")
            };
            let gap = (w - 2).saturating_sub(visible(&head) + version.len());
            out.push_str(&format!(
                "  {head}{}{}\n",
                " ".repeat(gap),
                dim(color, &version)
            ));
            let room = w.saturating_sub(16);
            if mark {
                out.push_str(&format!(
                    "  {}   {}\n",
                    brand(color, m[1]),
                    clip(scanned, room)
                ));
                out.push_str(&format!(
                    "  {}   {}\n",
                    brand(color, m[2]),
                    dim(color, &clip(where_at, room))
                ));
            } else {
                out.push_str(&format!("  {}\n", clip(scanned, w - 2)));
                out.push_str(&format!("  {}\n", dim(color, &clip(where_at, w - 2))));
            }
            out.push_str(&format!("  {}\n\n", dim(color, &ink.hair.repeat(w - 2))));
            out
        }
    }
}

/// The original masthead, unchanged.
fn classic_masthead(scanned: &str, where_at: &str, color: bool, w: usize) -> String {
    // Keep both lines inside the terminal: the mark and its gutter take 14
    // columns when it is drawn, the plain form only the 2-column indent.
    let room = w.saturating_sub(if color { 15 } else { 3 });
    let (scanned, where_at) = (&clip(scanned, room), &clip(where_at, room));
    let mut out = String::from("\n");
    if color {
        let m = &crate::ui::SPIDER_MARK;
        out.push_str(&format!(
            "  {}   {}\n",
            brand(color, m[0]),
            brand(color, "cohors")
        ));
        out.push_str(&format!(
            "  {}   {}\n",
            brand(color, m[1]),
            dim(color, scanned)
        ));
        out.push_str(&format!(
            "  {}   {}\n\n",
            brand(color, m[2]),
            dim(color, where_at)
        ));
    } else {
        out.push_str("  cohors\n");
        out.push_str(&format!("  {scanned}\n"));
        out.push_str(&format!("  {where_at}\n\n"));
    }
    out
}

/// One report section's heading over (or beside) a one-line descriptor.
pub fn section(heading: &str, descriptor: &str, color: bool) -> String {
    let look = get();
    let ink = ink(look.ascii);
    let w = look.width.max(40);
    let heading = sentence(heading);
    match look.style {
        Style::Classic => format!(
            "  {}\n  {}\n",
            brand(color, &heading),
            dim(color, descriptor)
        ),
        Style::Card => format!(
            "  {} {}   {}\n",
            brand(color, ink.accent),
            brand(color, &heading),
            dim(color, descriptor)
        ),
        Style::Panel => {
            // `── Do next ───────────── most urgent first ──`
            let left = format!("{0}{0} ", ink.h);
            let right = if descriptor.is_empty() {
                format!(" {0}{0}", ink.h)
            } else {
                format!(" {descriptor} {0}{0}", ink.h)
            };
            let used = left.chars().count() + heading.chars().count() + 1 + right.chars().count();
            let fill = ink.h.repeat((w - 2).saturating_sub(used).max(2));
            let right = if descriptor.is_empty() {
                dim(color, &right)
            } else {
                format!(
                    " {} {}",
                    dim(color, descriptor),
                    dim(color, &format!("{0}{0}", ink.h))
                )
            };
            format!(
                "  {}{} {}{}\n",
                dim(color, &left),
                brand(color, &heading),
                dim(color, &fill),
                right
            )
        }
        Style::Quiet => {
            // Letter-spacing suits a short word heading; a long one would
            // wrap, and a path or a name must read exactly as it is.
            let caps = if heading.contains(['/', '~', '.']) {
                heading.clone()
            } else if heading.chars().count() <= 12 {
                spaced_caps(&heading)
            } else {
                heading.to_uppercase()
            };
            if descriptor.is_empty() {
                format!("  {}\n", brand(color, &caps))
            } else {
                format!(
                    "  {}  {}  {}\n",
                    brand(color, &caps),
                    dim(color, ink.dot),
                    dim(color, descriptor)
                )
            }
        }
    }
}

/// A command (or name) and why you'd use it, in a guide list.
pub fn guide_row(cmd: &str, why: &str, width: usize, color: bool) -> String {
    let look = get();
    let ink = ink(look.ascii);
    if why.is_empty() {
        // A bare item (a folder in a list): no padding, leader or reason.
        let bullet = match look.style {
            Style::Classic if look.ascii => ">",
            Style::Classic => "▸",
            _ => ink.bullet,
        };
        let bullet = match look.style {
            Style::Classic => dim(color, bullet),
            _ => brand(color, bullet),
        };
        return format!("    {bullet} {}\n", bold(color, cmd));
    }
    match look.style {
        Style::Classic => format!(
            "    {} {}  {}\n",
            dim(color, if look.ascii { ">" } else { "▸" }),
            bold(color, &format!("{cmd:<width$}")),
            dim(color, why),
        ),
        Style::Card | Style::Panel => format!(
            "    {} {}  {}\n",
            brand(color, ink.bullet),
            bold(color, &format!("{cmd:<width$}")),
            dim(color, why),
        ),
        Style::Quiet => {
            // Dotted leaders carry the eye from the command to its reason.
            let lead = width.saturating_sub(cmd.chars().count()) + 2;
            format!(
                "    {} {} {}\n",
                bold(color, cmd),
                dim(color, &ink.dot.repeat(lead)),
                dim(color, why),
            )
        }
    }
}

/// A severity tag: always [`super::BLOCK_W`] columns, so rows line up.
pub fn severity_mark(severity: cohors_core::Severity, color: bool) -> String {
    use cohors_core::Severity as S;
    let look = get();
    let word = match severity {
        S::Risk => "RISK",
        S::Warn => "WARN",
        S::Notice => "NOTE",
        S::Info => "INFO",
        S::Ok => return " ".repeat(super::BLOCK_W),
    };
    let chip = |bg: &str| sgr(color, &format!("1;97;48;2;{bg}"), &format!(" {word} "));
    match look.style {
        Style::Classic | Style::Card => match severity {
            S::Risk => chip("180;72;63"),
            S::Warn => chip("168;116;42"),
            _ => dim(color, &format!(" {word} ")),
        },
        // Every severity a filled chip: the band's language carried down.
        Style::Panel if color => match severity {
            S::Risk => chip("180;72;63"),
            S::Warn => chip("168;116;42"),
            S::Notice => chip("74;98;128"),
            _ => chip("88;88;96"),
        },
        Style::Panel => format!("[{word}]"),
        // A shape and a word, no fill: the shape says it without colour.
        Style::Quiet => {
            let shape = match (severity, look.ascii) {
                (S::Risk, false) => "▲",
                (S::Warn, false) => "◆",
                (S::Notice, false) => "●",
                (_, false) => "·",
                (S::Risk, true) => "!",
                (S::Warn, true) => "*",
                (S::Notice, true) => "o",
                (_, true) => ".",
            };
            let text = format!("{shape} {word}");
            match severity {
                S::Risk => sgr(color, "1;38;2;214;96;84", &text),
                S::Warn => sgr(color, "1;38;2;214;158;72", &text),
                _ => dim(color, &text),
            }
        }
    }
}

/// The keys that work here: `enter  open the dashboard    q  quit`.
pub fn key_bar(keys: &[(&str, &str)], color: bool) -> String {
    let look = get();
    let ink = ink(look.ascii);
    match look.style {
        Style::Classic => {
            let parts: Vec<String> = keys
                .iter()
                .map(|(key, what)| format!("{}  {}", bold(color, key), dim(color, what)))
                .collect();
            format!("  {}\n", parts.join("    "))
        }
        Style::Card => {
            // The keys in a box of their own, like a row of buttons.
            let parts: Vec<String> = keys
                .iter()
                .map(|(key, what)| format!("{}  {}", brand(color, key), what))
                .collect();
            let body = parts.join(&format!("   {}   ", dim(color, ink.dot)));
            let inner = visible(&body) + 2;
            format!(
                "  {}\n  {} {body} {}\n  {}\n",
                dim(
                    color,
                    &format!("{}{}{}", ink.tl, ink.h.repeat(inner), ink.tr)
                ),
                dim(color, ink.v),
                dim(color, ink.v),
                dim(
                    color,
                    &format!("{}{}{}", ink.bl, ink.h.repeat(inner), ink.br)
                ),
            )
        }
        Style::Panel => {
            // Keycaps: reverse video, or brackets without colour.
            let parts: Vec<String> = keys
                .iter()
                .map(|(key, what)| {
                    let cap = if color {
                        sgr(color, "1;7", &format!(" {key} "))
                    } else {
                        format!("[{key}]")
                    };
                    format!("{cap} {}", dim(color, what))
                })
                .collect();
            let w = look.width.max(40);
            format!(
                "  {}\n  {}\n",
                dim(color, &ink.h.repeat(w - 2)),
                parts.join("    ")
            )
        }
        Style::Quiet => {
            let w = look.width.max(40);
            let parts: Vec<String> = keys
                .iter()
                .map(|(key, what)| {
                    let glyph = match (*key, look.ascii) {
                        ("enter", false) => "↵ ",
                        _ => "",
                    };
                    format!(
                        "{}{}  {}",
                        dim(color, glyph),
                        bold(color, key),
                        dim(color, what)
                    )
                })
                .collect();
            format!(
                "  {}\n  {}\n",
                dim(color, &ink.hair.repeat(w - 2)),
                parts.join("      ")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_style_parses_by_name() {
        for s in Style::ALL {
            assert_eq!(Style::parse(s.name()), Some(s));
            assert_eq!(Style::parse(&s.name().to_uppercase()), Some(s));
        }
        assert_eq!(Style::parse("nope"), None);
    }

    /// ASCII means ASCII: no piece of any style draws a non-ASCII glyph, and
    /// without colour no escape sequence is written.
    #[test]
    fn ascii_looks_draw_only_ascii() {
        for style in Style::ALL {
            let look = Look {
                style,
                ascii: true,
                width: 80,
            };
            let out = with(look, || {
                let mut s = masthead("24 repositories", "~/code", false);
                s.push_str(&section("do next", "most urgent first", false));
                s.push_str(&guide_row("cohors scan", "see the fleet", 12, false));
                for sev in [
                    cohors_core::Severity::Risk,
                    cohors_core::Severity::Warn,
                    cohors_core::Severity::Notice,
                    cohors_core::Severity::Info,
                ] {
                    s.push_str(&severity_mark(sev, false));
                }
                s.push_str(&key_bar(&[("enter", "open"), ("q", "quit")], false));
                s
            });
            assert!(out.is_ascii(), "{style:?}: {out}");
            assert!(!out.contains('\x1b'), "{style:?}");
        }
    }

    /// Severity marks keep their width in every look, so rows stay aligned.
    #[test]
    fn severity_marks_are_always_block_wide() {
        for style in Style::ALL {
            for ascii in [false, true] {
                let look = Look {
                    style,
                    ascii,
                    width: 80,
                };
                with(look, || {
                    for color in [false, true] {
                        let m = severity_mark(cohors_core::Severity::Warn, color);
                        assert_eq!(visible(&m), super::super::BLOCK_W, "{style:?} {m:?}");
                    }
                });
            }
        }
    }

    /// Boxes and rules never run past the drawing width.
    #[test]
    fn nothing_is_wider_than_the_look() {
        for style in Style::ALL {
            let look = Look {
                style,
                ascii: false,
                width: 72,
            };
            let out = with(look, || {
                let mut s = masthead(
                    "a very long headline that would run past the edge of any box drawn here",
                    "~/a/very/long/path/that/keeps/going/and/going/and/going/on",
                    true,
                );
                s.push_str(&section("do next", "most urgent first", true));
                s
            });
            for line in out.lines() {
                assert!(
                    visible(line) <= 72,
                    "{style:?}: {} cols: {line}",
                    visible(line)
                );
            }
        }
    }
}
