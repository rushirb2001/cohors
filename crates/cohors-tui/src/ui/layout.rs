//! Where things go: how tall each part of the overview is for a given terminal
//! size, and which rows of a list are visible.
//!
//! The design is drawn at 119 columns by about 53 rows. Terminals come in every
//! size, so instead of letting a layout engine squeeze things until they
//! garble, the decisions are made here, on purpose and in one place:
//!
//! - The six figure cards need width; below that they wrap to three by two,
//!   and on a short screen they become a single summary line.
//! - The header card becomes one line on a short screen.
//! - Cards 3 and 4 sit side by side, and stack when the screen is narrow.
//! - When there are not enough rows for every card, cards are hidden in a
//!   fixed order (never the focused one) rather than drawn one line tall.
//!
//! Everything here is pure arithmetic, so it is unit-tested without a terminal.

use crate::app::Card;

/// Below this size nothing useful fits; the UI shows a short message instead.
pub(crate) const MIN_WIDTH: u16 = 48;
pub(crate) const MIN_HEIGHT: u16 = 12;

/// Six figure cards in one row need this much width (17 columns each plus the
/// gaps between them).
const FIGURE_CARDS_MIN_WIDTH: u16 = 107;
/// Three figure cards across need this much.
const FIGURE_GRID_MIN_WIDTH: u16 = 56;
/// Cards 3 and 4 stack below this width.
const STACK_BELOW_WIDTH: u16 = 76;
/// The full header card is used from this height up.
const HEADER_CARD_MIN_HEIGHT: u16 = 30;
/// The row of figure cards is used from this height up.
const FIGURE_CARDS_MIN_HEIGHT: u16 = 40;
/// The three-by-two grid of figure cards costs ten rows, so it needs a tall
/// screen.
const FIGURE_GRID_MIN_HEIGHT: u16 = 50;

/// The fewest rows a card can have and still say something: two borders and
/// one line. The table also needs its column header.
const CARD_MIN: u16 = 3;
const TABLE_MIN: u16 = 4;
/// Blank rows are put between the cards only while the table can still show
/// this many rows (or all it wants, if that is fewer).
const AIRY_TABLE_MIN: u16 = 10;

/// Whether the terminal is big enough to draw the dashboard at all.
pub(crate) fn fits(width: u16, height: u16) -> bool {
    width >= MIN_WIDTH && height >= MIN_HEIGHT
}

/// How the six key figures are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Figures {
    /// Six cards in one row.
    Cards,
    /// Three cards across, two rows.
    Grid,
    /// One line of text.
    Line,
}

impl Figures {
    /// Rows this takes on screen.
    pub(crate) fn height(self) -> u16 {
        match self {
            Figures::Cards => 5,
            Figures::Grid => 10,
            Figures::Line => 1,
        }
    }
}

/// How many rows each card would like, borders included, if space were free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Wants {
    pub next: u16,
    pub risk: u16,
    pub tasks: u16,
    pub commits: u16,
    pub repos: u16,
}

/// The decided layout of the overview. A height of zero means the card is
/// hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OverviewPlan {
    /// 5 for the header card, 1 for the one-line header.
    pub header: u16,
    pub figures: Figures,
    /// Cards 3 and 4 are stacked rather than side by side.
    pub stacked: bool,
    /// Blank rows between one band and the next (after the header, after the
    /// figures, between the cards): 1 when the screen has room, else 0.
    pub gap: u16,
    pub next: u16,
    pub risk: u16,
    pub tasks: u16,
    pub commits: u16,
    pub repos: u16,
}

/// One horizontal band of the overview: a single card, or cards 3 and 4 side
/// by side.
struct Band {
    cards: &'static [Card],
    /// Rows the band would like, never less than its minimum.
    want: u16,
    /// Rows the band has been given so far. Zero once it is hidden.
    height: u16,
}

impl Band {
    /// A band starts at the fewest rows it can be read in.
    fn new(cards: &'static [Card], want: u16) -> Self {
        let min = if cards.contains(&Card::Repos) {
            TABLE_MIN
        } else {
            CARD_MIN
        };
        Self {
            cards,
            want: want.max(min),
            height: min,
        }
    }

    fn shown(&self) -> bool {
        self.height > 0
    }
}

/// The order cards are hidden in when rows run out: the two summary cards
/// first, the table last. The focused card is always skipped.
const HIDE_ORDER: [Card; 5] = [
    Card::Commits,
    Card::Tasks,
    Card::AtRisk,
    Card::DoNext,
    Card::Repos,
];

/// Decide the overview's layout. Call only when [`fits`] is true.
pub(crate) fn plan_overview(width: u16, height: u16, wants: &Wants, focus: Card) -> OverviewPlan {
    let header = if height >= HEADER_CARD_MIN_HEIGHT {
        5
    } else {
        1
    };
    let figures = if width >= FIGURE_CARDS_MIN_WIDTH && height >= FIGURE_CARDS_MIN_HEIGHT {
        Figures::Cards
    } else if width >= FIGURE_GRID_MIN_WIDTH && height >= FIGURE_GRID_MIN_HEIGHT {
        Figures::Grid
    } else {
        Figures::Line
    };
    let stacked = width < STACK_BELOW_WIDTH;
    // What is left for the cards: everything but the header, the figures and
    // the one-line key bar at the bottom.
    let available = height.saturating_sub(header + figures.height() + 1);

    // First try with a blank row after the header, after the figures and
    // between the bands of cards. That is worth its rows only when every
    // card still gets all it asked for and the table keeps a useful number;
    // otherwise the cards sit edge to edge, as they do on a small screen.
    let band_count: u16 = if stacked { 5 } else { 4 };
    let gaps = 2 + (band_count - 1);
    let airy = plan_bands(wants, stacked, available.saturating_sub(gaps), focus);
    let roomy = available > gaps
        && airy.iter().all(Band::shown)
        && airy.split_last().is_some_and(|(table, cards)| {
            cards.iter().all(|band| band.height >= band.want)
                && table.height >= AIRY_TABLE_MIN.min(table.want)
        });
    let (bands, gap) = if roomy {
        (airy, 1)
    } else {
        (plan_bands(wants, stacked, available, focus), 0)
    };

    let height_of = |card: Card| {
        bands
            .iter()
            .find(|band| band.cards.contains(&card))
            .map_or(0, |band| band.height)
    };
    OverviewPlan {
        header,
        figures,
        stacked,
        gap,
        next: height_of(Card::DoNext),
        risk: height_of(Card::AtRisk),
        tasks: height_of(Card::Tasks),
        commits: height_of(Card::Commits),
        repos: height_of(Card::Repos),
    }
}

/// Share `available` rows between the bands of cards.
fn plan_bands(wants: &Wants, stacked: bool, available: u16, focus: Card) -> Vec<Band> {
    let mut bands = vec![
        Band::new(&[Card::DoNext], wants.next),
        Band::new(&[Card::AtRisk], wants.risk),
    ];
    if stacked {
        bands.push(Band::new(&[Card::Tasks], wants.tasks));
        bands.push(Band::new(&[Card::Commits], wants.commits));
    } else {
        bands.push(Band::new(
            &[Card::Tasks, Card::Commits],
            wants.tasks.max(wants.commits),
        ));
    }
    bands.push(Band::new(&[Card::Repos], wants.repos));

    hide_until_minimums_fit(&mut bands, available, focus);
    grow_toward_wants(&mut bands, available);
    bands
}

/// Hide bands, in [`HIDE_ORDER`], until the ones still shown fit at their
/// minimum height. The band holding the focused card is never hidden.
fn hide_until_minimums_fit(bands: &mut [Band], available: u16, focus: Card) {
    for card in HIDE_ORDER {
        let needed: u16 = bands.iter().map(|band| band.height).sum();
        if needed <= available {
            return;
        }
        if let Some(band) = bands
            .iter_mut()
            .find(|band| band.cards.contains(&card) && !band.cards.contains(&focus))
        {
            band.height = 0;
        }
    }
}

/// Hand out the spare rows one at a time, top to bottom, to every band that
/// still wants more. Whatever is left after all are satisfied goes to the last
/// band (the table), so the key bar stays at the bottom of the screen.
fn grow_toward_wants(bands: &mut [Band], available: u16) {
    let used: u16 = bands.iter().map(|band| band.height).sum();
    let mut spare = available.saturating_sub(used);
    while spare > 0 {
        let mut grew = false;
        for band in bands.iter_mut() {
            if spare > 0 && band.shown() && band.height < band.want {
                band.height += 1;
                spare -= 1;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    if let Some(last) = bands.iter_mut().rev().find(|band| band.shown()) {
        last.height += spare;
    }
}

/// The first visible row of a list, so that `cursor` stays on screen.
///
/// `previous` is where the list was scrolled to last frame: the list only moves
/// when the cursor would otherwise leave it, which is how a list feels still
/// while the cursor moves inside it.
pub(crate) fn scroll_window(previous: usize, cursor: usize, visible: usize, total: usize) -> usize {
    if visible == 0 || total <= visible {
        return 0;
    }
    let offset = if cursor < previous {
        cursor
    } else if cursor >= previous + visible {
        cursor + 1 - visible
    } else {
        previous
    };
    offset.min(total - visible)
}

/// Which rows of a list to draw in `capacity` lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ListWindow {
    /// Index of the first row drawn.
    pub offset: usize,
    /// How many rows are drawn.
    pub shown: usize,
    /// Rows hidden above and below. When either is non-zero the last line of
    /// the card is used to say so instead of showing a row.
    pub above: usize,
    pub below: usize,
}

impl ListWindow {
    /// Whether a "N more" line is needed.
    pub(crate) fn clipped(&self) -> bool {
        self.above + self.below > 0
    }
}

/// Fit `total` one-line rows into `lines` lines, keeping `cursor` visible.
/// When the rows do not all fit, one line is given up to say how many are
/// hidden.
pub(crate) fn list_window(
    previous: usize,
    cursor: usize,
    lines: usize,
    total: usize,
) -> ListWindow {
    list_window_tall(previous, cursor, lines, 1, total)
}

/// [`list_window`] for entries that are each `item_height` lines tall (the
/// detail view's two-line list).
pub(crate) fn list_window_tall(
    previous: usize,
    cursor: usize,
    lines: usize,
    item_height: usize,
    total: usize,
) -> ListWindow {
    let item_height = item_height.max(1);
    let fit = lines / item_height;
    if total <= fit {
        return ListWindow {
            offset: 0,
            shown: total,
            above: 0,
            below: 0,
        };
    }
    // Keep one line back for the "N more" note. If that leaves no room for
    // even one entry, show what fits and skip the note.
    let with_note = lines.saturating_sub(1) / item_height;
    let shown = if with_note > 0 { with_note } else { fit };
    let offset = scroll_window(previous, cursor, shown, total);
    ListWindow {
        offset,
        shown,
        above: offset,
        below: total - offset - shown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the cards ask for with a fleet like the one in the design.
    const WANTS: Wants = Wants {
        next: 7,
        risk: 9,
        tasks: 9,
        commits: 10,
        repos: 31,
    };

    fn plan(width: u16, height: u16) -> OverviewPlan {
        plan_overview(width, height, &WANTS, Card::DoNext)
    }

    /// Rows the plan uses, key bar included. Must never exceed the screen.
    fn used(p: &OverviewPlan) -> u16 {
        let (middle, bands) = if p.stacked {
            (p.tasks + p.commits, 5)
        } else {
            (p.tasks.max(p.commits), 4)
        };
        // With gaps: one after the header, one after the figures, and one
        // between each band of cards and the next.
        let gaps = p.gap * (2 + bands - 1);
        p.header + p.figures.height() + p.next + p.risk + middle + p.repos + gaps + 1
    }

    #[test]
    fn the_designed_size_gets_the_designed_layout() {
        let p = plan(119, 53);
        assert_eq!((p.header, p.figures, p.stacked), (5, Figures::Cards, false));
        // Every card gets what it asked for, a blank row separates each
        // band from the next (five of them), and the table takes the rest.
        assert_eq!(p.gap, 1);
        assert_eq!((p.next, p.risk, p.tasks, p.commits), (7, 9, 10, 10));
        assert_eq!(p.repos, 53 - 5 - 5 - 7 - 9 - 10 - 5 - 1);
        assert_eq!(used(&p), 53);
    }

    #[test]
    fn gaps_are_given_up_before_any_card_loses_a_row() {
        // Tall: gaps. A little shorter: the same cards, edge to edge.
        assert_eq!(plan(119, 53).gap, 1);
        let tight = plan(119, 46);
        assert_eq!(tight.gap, 0);
        assert_eq!((tight.next, tight.risk), (7, 9));
        assert_eq!(used(&tight), 46);
        // A small terminal never spends rows on gaps.
        assert_eq!(plan(80, 24).gap, 0);
    }

    #[test]
    fn a_shorter_screen_shares_rows_instead_of_dropping_cards() {
        let p = plan(120, 45);
        assert_eq!((p.header, p.figures), (5, Figures::Cards));
        assert!(p.next >= 5 && p.risk >= 5 && p.tasks >= 5 && p.repos >= 5);
        assert_eq!(used(&p), 45);
    }

    #[test]
    fn a_classic_80_by_24_terminal_keeps_all_five_cards() {
        let p = plan(80, 24);
        assert_eq!((p.header, p.figures, p.stacked), (1, Figures::Line, false));
        for height in [p.next, p.risk, p.tasks, p.commits] {
            assert!(height >= CARD_MIN);
        }
        assert!(p.repos >= TABLE_MIN);
        assert_eq!(used(&p), 24);
    }

    #[test]
    fn figure_cards_wrap_then_collapse_as_the_screen_narrows() {
        assert_eq!(plan(119, 53).figures, Figures::Cards);
        // Too narrow for six across, tall enough for three by two.
        assert_eq!(plan(90, 60).figures, Figures::Grid);
        // Too short for the grid: one summary line.
        assert_eq!(plan(90, 40).figures, Figures::Line);
        assert_eq!(plan(50, 60).figures, Figures::Line);
    }

    #[test]
    fn cards_three_and_four_stack_when_narrow() {
        assert!(!plan(76, 40).stacked);
        let p = plan(75, 60);
        assert!(p.stacked);
        assert!(p.tasks > 0 && p.commits > 0);
        assert_eq!(used(&p), 60);
    }

    #[test]
    fn a_tiny_screen_hides_cards_in_order_but_never_the_focused_one() {
        // 12 rows leaves 9 for cards: only two of them fit.
        let p = plan(60, 12);
        assert_eq!((p.tasks, p.commits, p.risk), (0, 0, 0));
        assert!(p.next >= CARD_MIN && p.repos >= TABLE_MIN);
        assert!(used(&p) <= 12);

        // Focus the commits card: it survives, and another card gives way.
        let p = plan_overview(60, 12, &WANTS, Card::Commits);
        assert!(p.commits >= CARD_MIN, "{p:?}");
        assert!(used(&p) <= 12);
    }

    #[test]
    fn no_size_ever_overflows_the_screen() {
        for width in MIN_WIDTH..=200 {
            for height in MIN_HEIGHT..=80 {
                for focus in Card::ALL {
                    let p = plan_overview(width, height, &WANTS, focus);
                    assert!(used(&p) <= height, "{width}x{height} {focus:?}: {p:?}");
                }
            }
        }
    }

    #[test]
    fn an_empty_fleet_still_gets_readable_cards() {
        let nothing = Wants {
            next: 0,
            risk: 0,
            tasks: 0,
            commits: 0,
            repos: 0,
        };
        let p = plan_overview(119, 53, &nothing, Card::DoNext);
        assert!(p.next >= CARD_MIN && p.risk >= CARD_MIN && p.repos >= TABLE_MIN);
        assert_eq!(used(&p), 53);
    }

    #[test]
    fn the_size_floor() {
        assert!(fits(MIN_WIDTH, MIN_HEIGHT));
        assert!(!fits(MIN_WIDTH - 1, 40));
        assert!(!fits(120, MIN_HEIGHT - 1));
    }

    #[test]
    fn a_list_scrolls_only_when_the_cursor_would_leave_it() {
        // Ten rows, four visible.
        assert_eq!(scroll_window(0, 0, 4, 10), 0);
        assert_eq!(scroll_window(0, 3, 4, 10), 0); // still inside
        assert_eq!(scroll_window(0, 4, 4, 10), 1); // stepped past the bottom
        assert_eq!(scroll_window(3, 5, 4, 10), 3); // moving inside: stays put
        assert_eq!(scroll_window(3, 2, 4, 10), 2); // stepped past the top
        assert_eq!(scroll_window(9, 9, 4, 10), 6); // never scrolls past the end
        assert_eq!(scroll_window(5, 0, 4, 3), 0); // everything fits
        assert_eq!(scroll_window(5, 2, 0, 3), 0); // nothing visible: no panic
    }

    #[test]
    fn a_clipped_list_gives_one_line_to_the_more_note() {
        // Everything fits: no note.
        let all = list_window(0, 0, 5, 5);
        assert_eq!((all.shown, all.clipped()), (5, false));

        // Twelve rows in five lines: four rows and "8 more".
        let top = list_window(0, 0, 5, 12);
        assert_eq!((top.offset, top.shown, top.above, top.below), (0, 4, 0, 8));

        // Cursor at the end: the note counts what is above instead.
        let end = list_window(0, 11, 5, 12);
        assert_eq!((end.offset, end.shown, end.above, end.below), (8, 4, 8, 0));

        // Degenerate sizes never panic or show more than fits.
        assert_eq!(list_window(0, 3, 1, 12).shown, 1);
        assert_eq!(list_window(0, 3, 0, 12).shown, 0);
        assert_eq!(list_window(0, 0, 4, 0).shown, 0);
    }

    #[test]
    fn two_line_entries_leave_a_line_for_the_note() {
        // Eight lines hold four two-line entries exactly.
        assert_eq!(list_window_tall(0, 0, 8, 2, 4).shown, 4);
        // A fifth entry: three are shown (six lines) and the note takes one.
        let clipped = list_window_tall(0, 0, 8, 2, 5);
        assert_eq!((clipped.shown, clipped.below), (3, 2));
        // An odd line count already has the spare line for the note.
        assert_eq!(list_window_tall(0, 0, 9, 2, 9).shown, 4);
        // What is shown always fits, whatever the sizes.
        for lines in 0..20 {
            for total in 0..12 {
                let w = list_window_tall(0, total / 2, lines, 2, total);
                assert!(w.shown * 2 <= lines, "{lines} lines, {total} entries");
                assert_eq!(w.above + w.shown + w.below, total);
            }
        }
    }
}
