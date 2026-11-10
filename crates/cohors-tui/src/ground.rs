//! Finding out the terminal's background colour.
//!
//! The dashboard fills its cards with a shade just off the terminal's
//! background, which only looks right if that background is actually known.
//! A terminal can be asked: write `ESC ] 11 ; ? ESC \` and it writes back
//! `ESC ] 11 ; rgb:RRRR/GGGG/BBBB ESC \` on the same channel keystrokes
//! arrive on. Three things make that awkward:
//!
//! - Some terminals never answer (and one may answer only while its window
//!   is showing). Silence must mean "not known", never "light" or "dark".
//! - The answer can be slow, so there is no safe moment to stop listening.
//! - Reading the answer must not take keys the user typed.
//!
//! So the answer is read from the same event stream as the keys. The key
//! reader hands it over as a burst of key events (it does not know the
//! sequence): Alt+`]`, then the text one character at a time, then Alt+`\`
//! (or Ctrl+G, for a terminal that ends it with BEL). [`GroundReply`]
//! recognises that burst wherever it turns up, at startup or minutes later,
//! takes it out of the stream, and reports the colour. Nothing blocks and no
//! second reader exists to swallow a keystroke.
//!
//! Everything here is pure: it is fed key events and returns values.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The question, as written to the terminal.
pub const QUERY: &str = "\x1b]11;?\x1b\\";

/// The longest an answer can be: `11;rgb:ffff/ffff/ffff` is 21 characters.
/// A burst longer than this is not an answer and is let go.
const LONGEST: usize = 40;

/// What became of a key event shown to [`GroundReply::feed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fed {
    /// An ordinary key: handle it as usual.
    NotPart,
    /// Part of the terminal's answer: drop it.
    Taken,
    /// The last piece of the answer. `Some` is the background colour as red,
    /// green and blue; `None` means the burst ended but was not a colour.
    Answer(Option<(u8, u8, u8)>),
}

/// Recognises the terminal's answer in the stream of key events.
#[derive(Debug, Default)]
pub struct GroundReply {
    /// The text of the answer so far, while one is arriving.
    arriving: Option<String>,
}

impl GroundReply {
    /// Show the next key event to the recogniser.
    pub fn feed(&mut self, key: &KeyEvent) -> Fed {
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(text) = &mut self.arriving else {
            // An answer opens with `ESC ]`, which arrives as Alt+`]`.
            if alt && key.code == KeyCode::Char(']') {
                self.arriving = Some(String::new());
                return Fed::Taken;
            }
            return Fed::NotPart;
        };
        match key.code {
            // `ESC \` or BEL closes it.
            KeyCode::Char('\\') if alt => self.finish(),
            KeyCode::Char('g') if ctrl => self.finish(),
            KeyCode::Char(c) if !alt && !ctrl && text.len() < LONGEST => {
                text.push(c);
                Fed::Taken
            }
            // Anything else means this was not an answer after all (the user
            // really did press Alt+`]`). Stop taking keys.
            _ => {
                self.arriving = None;
                Fed::NotPart
            }
        }
    }

    fn finish(&mut self) -> Fed {
        let text = self.arriving.take().unwrap_or_default();
        Fed::Answer(parse(&text))
    }
}

/// Read `11;rgb:RRRR/GGGG/BBBB` (each part one to four hex digits).
fn parse(text: &str) -> Option<(u8, u8, u8)> {
    let colour = text.strip_prefix("11;")?;
    let parts = colour
        .strip_prefix("rgb:")
        .or_else(|| colour.strip_prefix("rgba:"))?;
    let mut channels = parts.split('/').map(channel);
    let (r, g, b) = (channels.next()??, channels.next()??, channels.next()??);
    Some((r, g, b))
}

/// One channel, scaled to 0..=255 whatever its precision (`f`, `ff`, `fff`
/// and `ffff` all mean full strength).
fn channel(hex: &str) -> Option<u8> {
    if hex.is_empty() || hex.len() > 4 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    let full = (1u32 << (4 * hex.len())) - 1;
    Some((value * 255 / full) as u8)
}

/// Whether a background is dark, by its perceived brightness (ITU-R BT.601).
pub fn is_dark((r, g, b): (u8, u8, u8)) -> bool {
    let brightness = f32::from(r) * 0.299 + f32::from(g) * 0.587 + f32::from(b) * 0.114;
    brightness < 128.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn alt(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
    }

    /// Feed a whole answer the way the key reader delivers it.
    fn answer(reply: &mut GroundReply, body: &str, end: KeyEvent) -> Fed {
        assert_eq!(reply.feed(&alt(']')), Fed::Taken);
        for c in body.chars() {
            assert_eq!(reply.feed(&key(c)), Fed::Taken, "{c}");
        }
        reply.feed(&end)
    }

    #[test]
    fn an_answer_is_taken_out_of_the_keys_and_read() {
        let mut reply = GroundReply::default();
        // Ordinary keys before it are left alone.
        assert_eq!(reply.feed(&key('q')), Fed::NotPart);
        assert_eq!(
            answer(&mut reply, "11;rgb:1c1c/1b1b/1a1a", alt('\\')),
            Fed::Answer(Some((0x1C, 0x1B, 0x1A)))
        );
        // And ordinary keys after it too: nothing is left half-open.
        assert_eq!(reply.feed(&key('1')), Fed::NotPart);
        // A terminal that ends the answer with BEL (seen as Ctrl+G).
        let bel = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL);
        assert_eq!(
            answer(&mut reply, "11;rgb:ffff/ffff/ffff", bel),
            Fed::Answer(Some((255, 255, 255)))
        );
    }

    #[test]
    fn every_precision_of_colour_is_understood() {
        assert_eq!(parse("11;rgb:0000/2b2b/3636"), Some((0x00, 0x2B, 0x36)));
        assert_eq!(parse("11;rgb:00/2b/36"), Some((0x00, 0x2B, 0x36)));
        assert_eq!(parse("11;rgb:f/0/8"), Some((255, 0, 136)));
        assert_eq!(
            parse("11;rgba:fdfd/f6f6/e3e3/ffff"),
            Some((0xFD, 0xF6, 0xE3))
        );
        // Not a colour: nothing is learned, and nothing is guessed.
        for junk in [
            "",
            "11;",
            "11;rgb:",
            "11;rgb:12/34",
            "10;rgb:1/2/3",
            "11;#fff",
            "11;rgb:zz/00/00",
        ] {
            assert_eq!(parse(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn a_real_alt_bracket_does_not_eat_the_keys_that_follow() {
        let mut reply = GroundReply::default();
        // The user pressed Alt+] and then Enter: Enter is theirs.
        assert_eq!(reply.feed(&alt(']')), Fed::Taken);
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(reply.feed(&enter), Fed::NotPart);
        assert_eq!(reply.feed(&key('5')), Fed::NotPart);
        // A burst far too long to be an answer is let go as well.
        assert_eq!(reply.feed(&alt(']')), Fed::Taken);
        let mut taken = 0;
        while reply.feed(&key('x')) == Fed::Taken {
            taken += 1;
            assert!(taken <= LONGEST);
        }
        assert_eq!(reply.feed(&key('y')), Fed::NotPart);
    }

    #[test]
    fn brightness_decides_dark_or_light() {
        assert!(is_dark((0x1C, 0x1B, 0x1A)));
        assert!(is_dark((0x00, 0x2B, 0x36))); // Solarized dark
        assert!(!is_dark((0xFD, 0xF6, 0xE3))); // Solarized light
        assert!(!is_dark((255, 255, 255)));
    }
}
