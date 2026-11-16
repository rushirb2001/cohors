//! The mouse: what a click or a turn of the wheel landed on.
//!
//! The terminal reports a click as a column and a row, nothing more. To know
//! what is there, the renderer notes, as it draws each frame, where it put the
//! things a click can act on: each numbered card, each row of a list, each key
//! hint. A click then looks up the most specific of those under the pointer.
//!
//! The notes are kept in a thread-local list while a frame is drawn (the
//! drawing helpers in `ui::cards` are shared by every card and do not get the
//! `App`), and [`crate::ui::render`] hands the finished list to the `App`,
//! which is what the event loop asks about the next click.
//!
//! A click never does anything a key could not: it moves the focus or a
//! cursor, or it presses a key (a key hint, or Enter on the row that already
//! has the cursor). So a click can never reach a work-destroying action
//! without the same confirmation a key press gets.

use std::cell::RefCell;

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};

use crate::app::{App, Card, Mode, Pane, Screen};

/// Something a click can land on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A numbered card: on the overview, cards 1 to 5; on the detail view,
    /// card 5 is its list of repositories.
    Card(Card),
    /// One of the detail view's three cards about the selected repository.
    Pane(Pane),
    /// Entry `n` of the list drawn here (which list: the card around it).
    Row(usize),
    /// A key hint: clicking it is pressing the key.
    Key(KeyCode),
    /// An overlay's box: a click outside it closes the overlay.
    Overlay,
    /// Entry `n` of an overlay's side list (the standup's repositories).
    Side(usize),
}

/// Where one [`Target`] was drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub area: Rect,
    pub target: Target,
}

thread_local! {
    /// The hits of the frame being drawn, in the order they were drawn.
    static FRAME: RefCell<Vec<Hit>> = const { RefCell::new(Vec::new()) };
}

/// Note that `target` was drawn over `area` in this frame.
pub(crate) fn record(area: Rect, target: Target) {
    if area.width > 0 && area.height > 0 {
        FRAME.with(|frame| frame.borrow_mut().push(Hit { area, target }));
    }
}

/// Forget what was drawn so far: at the start of a frame, and before an
/// overlay is drawn (a click while one is open is about the overlay only).
pub(crate) fn clear() {
    FRAME.with(|frame| frame.borrow_mut().clear());
}

/// The frame's hits, taken out of the list.
pub(crate) fn take() -> Vec<Hit> {
    FRAME.with(|frame| std::mem::take(&mut *frame.borrow_mut()))
}

/// Everything under the cell at (`x`, `y`), the most specific first: what is
/// drawn later sits inside or on top of what was drawn before it (a row
/// inside its card, a hint on its card's border).
pub(crate) fn targets_at(hits: &[Hit], x: u16, y: u16) -> Vec<Target> {
    hits.iter()
        .rev()
        .filter(|hit| hit.area.contains(Position { x, y }))
        .map(|hit| hit.target)
        .collect()
}

fn press(code: KeyCode) -> Option<KeyEvent> {
    Some(KeyEvent::new(code, KeyModifiers::NONE))
}

impl App {
    /// Handle a mouse event. A click or a wheel turn changes what has the
    /// focus or the cursor right here; when it means a key press (a key hint,
    /// or a second click on the row under the cursor, which opens it), that
    /// key is returned for the caller to handle like one typed.
    pub fn on_mouse(&mut self, event: MouseEvent) -> Option<KeyEvent> {
        match event.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                // The wheel scrolls the card under the pointer, which takes
                // the focus, as it would on a click.
                if matches!(self.mode, Mode::Normal) {
                    self.focus_under(event.column, event.row);
                }
                self.on_mouse_scroll(event.kind == MouseEventKind::ScrollUp);
                None
            }
            MouseEventKind::Down(MouseButton::Left) => self.on_click(event.column, event.row),
            _ => None,
        }
    }

    /// A left click at (`x`, `y`).
    pub fn on_click(&mut self, x: u16, y: u16) -> Option<KeyEvent> {
        let under = targets_at(&self.hits.borrow(), x, y);
        if let Some(Target::Key(code)) = under.first() {
            return press(*code);
        }
        match self.mode {
            Mode::Normal | Mode::Filter => {}
            // The help is read and dismissed: a click anywhere closes it.
            Mode::Help => return press(KeyCode::Esc),
            _ => return self.on_overlay_click(&under),
        }
        let row = under.iter().find_map(|target| match target {
            Target::Row(index) => Some(*index),
            _ => None,
        });
        let card = under.iter().find_map(|target| match target {
            Target::Card(card) => Some(*card),
            _ => None,
        });
        match self.screen {
            Screen::Overview => {
                let card = card?;
                let had_focus = self.focus == card;
                self.focus = card;
                let index = row?;
                if had_focus && self.cursor(card) == index {
                    return press(KeyCode::Enter);
                }
                self.set_cursor(card, index);
                None
            }
            Screen::Repos => {
                if let Some(Target::Pane(pane)) = under
                    .iter()
                    .find(|target| matches!(target, Target::Pane(_)))
                {
                    self.set_pane(*pane);
                    return None;
                }
                card.filter(|card| *card == Card::Repos)?;
                let had_focus = self.pane == Pane::List;
                self.set_pane(Pane::List);
                let index = row?;
                if had_focus && self.selected == index {
                    return press(KeyCode::Enter);
                }
                self.set_cursor(Card::Repos, index);
                None
            }
        }
    }

    /// A click while an overlay is open. Outside its box it closes the
    /// overlay (a confirmation is cancelled, never accepted); on a row of
    /// one of its lists it moves that list's cursor there, and a second
    /// click on the row under the cursor presses Enter.
    fn on_overlay_click(&mut self, under: &[Target]) -> Option<KeyEvent> {
        if !under.contains(&Target::Overlay) {
            // `q` closes the standup whichever pane has the focus (its Esc
            // steps out of the commits first).
            return press(match self.mode {
                Mode::Standup => KeyCode::Char('q'),
                _ => KeyCode::Esc,
            });
        }
        let row = under.iter().find_map(|target| match target {
            Target::Row(index) => Some(*index),
            _ => None,
        });
        if let Some(Target::Side(index)) = under.iter().find(|t| matches!(t, Target::Side(_)))
            && let Some(view) = &mut self.standup
        {
            // A repository in the standup's left column.
            if view.focus != *index {
                view.focus = *index;
                view.commit_cursor = 0;
                view.set_offset(0);
            }
            view.commits_focused = false;
            return None;
        }
        let index = row?;
        match self.mode {
            Mode::OpenWith => {
                let picker = self.open_with.as_mut()?;
                if picker.cursor == index {
                    return press(KeyCode::Enter);
                }
                picker.cursor = index;
            }
            Mode::PushBranch => {
                let picker = self.push_pick.as_mut()?;
                if picker.cursor == index {
                    return press(KeyCode::Enter);
                }
                picker.cursor = index;
            }
            Mode::CommandRun => {
                let run = self.run.as_mut()?;
                if run.focus != index {
                    run.focus = index;
                    run.scroll = 0;
                }
            }
            Mode::Standup => {
                let view = self.standup.as_mut()?;
                view.commits_focused = true;
                view.commit_cursor = index;
            }
            _ => {}
        }
        None
    }

    /// Give the focus to the card under (`x`, `y`), if there is one.
    fn focus_under(&mut self, x: u16, y: u16) {
        let under = targets_at(&self.hits.borrow(), x, y);
        for target in under {
            match (self.screen, target) {
                (Screen::Overview, Target::Card(card)) => {
                    self.focus = card;
                    return;
                }
                (Screen::Repos, Target::Pane(pane)) => {
                    self.set_pane(pane);
                    return;
                }
                (Screen::Repos, Target::Card(Card::Repos)) => {
                    self.set_pane(Pane::List);
                    return;
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_most_specific_target_comes_first() {
        let card = Rect::new(0, 0, 40, 10);
        let row = Rect::new(2, 3, 36, 1);
        let hits = [
            Hit {
                area: card,
                target: Target::Card(Card::AtRisk),
            },
            Hit {
                area: row,
                target: Target::Row(2),
            },
        ];
        assert_eq!(
            targets_at(&hits, 5, 3),
            [Target::Row(2), Target::Card(Card::AtRisk)]
        );
        assert_eq!(targets_at(&hits, 5, 5), [Target::Card(Card::AtRisk)]);
        assert!(targets_at(&hits, 45, 3).is_empty());
    }

    #[test]
    fn empty_areas_are_never_recorded() {
        clear();
        record(Rect::new(3, 3, 0, 4), Target::Row(0));
        record(Rect::new(3, 3, 4, 1), Target::Row(1));
        assert_eq!(take().len(), 1);
        assert!(take().is_empty(), "take empties the list");
    }
}
