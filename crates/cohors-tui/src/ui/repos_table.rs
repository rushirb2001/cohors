//! The repository table: card 5 of the overview, and the first line of every
//! entry in the detail view.
//!
//! Both are the same table at two widths, so the columns are described once
//! as data ([`ColumnSpec`]) and one function decides which of them fit
//! ([`columns`]). When the terminal is narrow, columns are dropped in a fixed
//! order, least important first, and the name column gives up width last.
//!
//! Facts in a row (branch, counts, last commit) come straight from the
//! repository's snapshot. Judgments (the severity mark, the task count) come
//! from the shared overview model and are never worked out here.

use cohors_core::{RepoLine, ViewRow};

use super::cards::{
    BLOCK_WIDTH, CardArea, CardFrame, Hint, Item, draw_card, draw_list, hint, severity_block,
};
use super::layout::list_window;
use super::*;

/// The severity mark and the space after it, before the first column.
const MARK_WIDTH: usize = BLOCK_WIDTH + 1; // the severity block and a space

/// Which table is being laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TableKind {
    /// Card 5: one line per repository, ending with the last commit's subject.
    Overview,
    /// The detail view: wider columns, the fetch age, and a second line per
    /// repository for the path and the commit.
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColumnId {
    Name,
    Branch,
    Upstream,
    Staged,
    Modified,
    Untracked,
    Stash,
    Tasks,
    Fetched,
    Age,
    Commit,
}

/// A column as designed: the narrowest it may be and the width it has in the
/// approved layout.
struct ColumnSpec {
    id: ColumnId,
    min: usize,
    max: usize,
}

const fn spec(id: ColumnId, min: usize, max: usize) -> ColumnSpec {
    ColumnSpec { id, min, max }
}

/// Card 5 at full width (the last-commit column takes whatever is left).
const OVERVIEW_COLUMNS: [ColumnSpec; 10] = [
    spec(ColumnId::Name, 14, 24),
    spec(ColumnId::Branch, 10, 15),
    spec(ColumnId::Upstream, 12, 12),
    spec(ColumnId::Staged, 5, 5),
    spec(ColumnId::Modified, 5, 5),
    spec(ColumnId::Untracked, 5, 5),
    spec(ColumnId::Stash, 7, 7),
    spec(ColumnId::Tasks, 7, 7),
    spec(ColumnId::Age, 6, 6),
    spec(ColumnId::Commit, 14, usize::MAX),
];

/// The detail view's list at full width.
const DETAIL_COLUMNS: [ColumnSpec; 10] = [
    spec(ColumnId::Name, 14, 30),
    spec(ColumnId::Branch, 10, 20),
    spec(ColumnId::Upstream, 12, 14),
    spec(ColumnId::Staged, 5, 5),
    spec(ColumnId::Modified, 5, 5),
    spec(ColumnId::Untracked, 5, 5),
    spec(ColumnId::Stash, 7, 7),
    spec(ColumnId::Tasks, 7, 7),
    spec(ColumnId::Fetched, 10, 10),
    spec(ColumnId::Age, 8, 8),
];

/// Columns to give up, first to last, when the table does not fit. The name,
/// the change counts and the commit age are never dropped: they are what the
/// table is for.
const DROP_ORDER: [ColumnId; 6] = [
    ColumnId::Commit,
    ColumnId::Tasks,
    ColumnId::Stash,
    ColumnId::Fetched,
    ColumnId::Upstream,
    ColumnId::Branch,
];

/// Columns that take spare width, in the order they are offered it.
const GROW_ORDER: [ColumnId; 4] = [
    ColumnId::Name,
    ColumnId::Branch,
    ColumnId::Upstream,
    ColumnId::Commit,
];

/// A column that made it onto the screen, with its decided width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Column {
    pub id: ColumnId,
    pub width: usize,
}

/// Decide which columns are shown in `text_width` columns, and how wide.
pub(crate) fn columns(kind: TableKind, text_width: usize) -> Vec<Column> {
    let specs: &[ColumnSpec] = match kind {
        TableKind::Overview => &OVERVIEW_COLUMNS,
        TableKind::Detail => &DETAIL_COLUMNS,
    };
    let mut kept: Vec<&ColumnSpec> = specs.iter().collect();
    let needed = |kept: &[&ColumnSpec]| MARK_WIDTH + kept.iter().map(|c| c.min).sum::<usize>();

    // Drop columns until the rest fit at their narrowest.
    for id in DROP_ORDER {
        if needed(&kept) <= text_width {
            break;
        }
        kept.retain(|c| c.id != id);
    }

    // Start everything at its narrowest, then hand out what is left.
    let mut result: Vec<Column> = kept
        .iter()
        .map(|c| Column {
            id: c.id,
            width: c.min,
        })
        .collect();
    let mut spare = text_width.saturating_sub(needed(&kept));
    for id in GROW_ORDER {
        let Some(index) = kept.iter().position(|c| c.id == id) else {
            continue;
        };
        let extra = (kept[index].max - kept[index].min).min(spare);
        result[index].width += extra;
        spare -= extra;
    }
    result
}

/// The width of column `id`, or zero when it was dropped.
fn width_of(columns: &[Column], id: ColumnId) -> usize {
    columns.iter().find(|c| c.id == id).map_or(0, |c| c.width)
}

/// Everything a row needs: the snapshot for the facts and the overview's line
/// for the judgments.
pub(crate) struct RepoRow<'a> {
    pub snap: &'a RepoSnapshot,
    pub line: Option<&'a RepoLine>,
    /// Character positions in the name that matched the typed filter.
    pub highlights: &'a [u32],
    /// Marked for a bulk action.
    pub marked: bool,
    /// An action is running on it.
    pub busy: bool,
}

impl<'a> RepoRow<'a> {
    pub(crate) fn new(app: &'a App, view_row: &'a ViewRow) -> Self {
        let snap = &app.repos[view_row.index];
        Self {
            snap,
            line: app.line(&snap.id),
            highlights: &view_row.name_highlights,
            marked: app.selection.contains(&snap.id),
            busy: app.busy.contains(&snap.id),
        }
    }
}

/// How the current branch relates to its upstream, in a word or two.
pub(crate) fn upstream_label(snap: &RepoSnapshot, theme: &Theme) -> (String, Style) {
    match &snap.upstream {
        Some(up) if up.ahead == 0 && up.behind == 0 => ("level".to_string(), theme.dim()),
        Some(up) => (
            theme.glyphs.ahead_behind(up.ahead, up.behind),
            theme.ahead(),
        ),
        // A remote exists but this branch was never pushed to it.
        None if snap.remote_url.is_some() => ("no upstream".to_string(), theme.warn()),
        None => ("no remote".to_string(), theme.dim()),
    }
}

/// When the remote was last fetched: `recent` within a day, else `36d ago`.
pub(crate) fn fetched_label(fetched: Option<i64>, now: i64) -> String {
    const DAY: i64 = 24 * 60 * 60;
    match fetched {
        None => "never".to_string(),
        Some(at) if now - at < DAY => "recent".to_string(),
        Some(at) => format!("{}d ago", (now - at) / DAY),
    }
}

/// A count cell: right-aligned, dim when zero so the non-zero ones stand out.
fn count_cell(n: u32, width: usize, style: Style, theme: &Theme) -> Span<'static> {
    let text = text::cell_right(&n.to_string(), width, theme.glyphs.ellipsis());
    Span::styled(text, if n == 0 { theme.dim() } else { style })
}

/// The repository name as a padded cell, with the characters that matched the
/// filter picked out.
fn name_spans(row: &RepoRow, width: usize, theme: &Theme) -> Vec<Span<'static>> {
    let at_risk = row.line.is_some_and(|line| line.at_risk);
    let base = if row.snap.has_error() {
        theme.error()
    } else if at_risk {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };
    let padded = text::column(&text::safe(&row.snap.name), width, theme.glyphs.ellipsis());
    if row.highlights.is_empty() {
        return vec![Span::styled(padded, base)];
    }
    // One span per character keeps this simple; a name is at most a few dozen.
    padded
        .chars()
        .enumerate()
        .map(|(i, ch)| {
            let hit = row.highlights.contains(&(i as u32));
            Span::styled(ch.to_string(), if hit { theme.highlight() } else { base })
        })
        .collect()
}

/// One column's cell for one repository.
fn cell_spans(row: &RepoRow, column: Column, now: i64, theme: &Theme) -> Vec<Span<'static>> {
    let ell = theme.glyphs.ellipsis();
    let snap = row.snap;
    let width = column.width;
    let blank = || vec![Span::raw(" ".repeat(width))];
    // An unreadable repository has a name and a reason, and nothing else.
    if snap.has_error() {
        return match column.id {
            ColumnId::Name => name_spans(row, width, theme),
            ColumnId::Branch => vec![Span::styled(
                text::cell("unreadable", width, ell),
                theme.risk(),
            )],
            ColumnId::Commit => vec![Span::styled(
                format!(
                    "  {}",
                    text::cell(
                        &text::safe(snap.error.as_deref().unwrap_or("")),
                        width.saturating_sub(2),
                        ell
                    )
                ),
                theme.dim(),
            )],
            _ => blank(),
        };
    }
    match column.id {
        ColumnId::Name => name_spans(row, width, theme),
        ColumnId::Branch => {
            let style = match snap.branch {
                Branch::Named(_) => Style::new(),
                Branch::Detached(_) | Branch::Unborn => theme.detached(),
            };
            vec![Span::styled(
                text::column(&text::safe(&snap.branch.label()), width, ell),
                style,
            )]
        }
        ColumnId::Upstream => {
            let (label, style) = upstream_label(snap, theme);
            vec![Span::styled(text::column(&label, width, ell), style)]
        }
        ColumnId::Staged => vec![count_cell(
            snap.worktree.staged,
            width,
            theme.staged(),
            theme,
        )],
        ColumnId::Modified => vec![count_cell(
            snap.worktree.modified,
            width,
            theme.modified(),
            theme,
        )],
        ColumnId::Untracked => vec![count_cell(
            snap.worktree.untracked,
            width,
            Style::new(),
            theme,
        )],
        ColumnId::Stash => vec![count_cell(snap.stash_count, width, Style::new(), theme)],
        ColumnId::Tasks => match row.line {
            Some(line) => vec![count_cell(
                line.tasks_open as u32,
                width,
                Style::new(),
                theme,
            )],
            None => blank(),
        },
        ColumnId::Fetched => {
            // A repository with no remote has nothing to fetch.
            let label = if snap.remote_url.is_some() {
                fetched_label(snap.last_fetch, now)
            } else {
                String::new()
            };
            vec![Span::styled(
                text::cell_right(&label, width, ell),
                theme.dim(),
            )]
        }
        ColumnId::Age => {
            let age = snap
                .last_commit
                .as_ref()
                .map(|c| time::relative(c.timestamp, now))
                .unwrap_or_default();
            vec![Span::styled(
                text::cell_right(&age, width, ell),
                theme.dim(),
            )]
        }
        ColumnId::Commit => {
            let subject = snap
                .last_commit
                .as_ref()
                .map(|c| text::safe(&c.summary))
                .unwrap_or_default();
            vec![Span::styled(
                format!("  {}", text::cell(&subject, width.saturating_sub(2), ell)),
                theme.dim(),
            )]
        }
    }
}

/// The column header line.
pub(crate) fn header_line(kind: TableKind, columns: &[Column], theme: &Theme) -> Line<'static> {
    let ell = theme.glyphs.ellipsis();
    let mut text = " ".repeat(MARK_WIDTH);
    for column in columns {
        let width = column.width;
        let label = match column.id {
            ColumnId::Name => text::cell("repository", width, ell),
            ColumnId::Branch => text::cell("branch", width, ell),
            ColumnId::Upstream => text::cell("upstream", width, ell),
            ColumnId::Staged => text::cell_right("stg", width, ell),
            ColumnId::Modified => text::cell_right("mod", width, ell),
            ColumnId::Untracked => text::cell_right("new", width, ell),
            ColumnId::Stash => text::cell_right("stash", width, ell),
            ColumnId::Tasks => text::cell_right("tasks", width, ell),
            ColumnId::Fetched => text::cell_right("fetched", width, ell),
            // The detail view has no subject column on this line, so its age
            // column says what the age is of.
            ColumnId::Age if kind == TableKind::Detail => text::cell_right("commit", width, ell),
            ColumnId::Age => text::cell_right("age", width, ell),
            ColumnId::Commit => format!("  {}", text::cell("last commit", width - 2, ell)),
        };
        text.push_str(&label);
    }
    Line::from(Span::styled(text, theme.dim()))
}

/// A repository's row: the severity mark, then one cell per column.
pub(crate) fn row_line(
    row: &RepoRow,
    columns: &[Column],
    now: i64,
    theme: &Theme,
) -> Line<'static> {
    let mark = match row.line {
        Some(line) => severity_block(line.severity, theme),
        // Not judged yet (the overview is still being computed).
        None => Span::raw(" ".repeat(BLOCK_WIDTH)),
    };
    let mut spans = vec![mark, Span::raw(" ")];
    for column in columns {
        spans.extend(cell_spans(row, *column, now, theme));
    }
    Line::from(spans)
}

/// The detail view's second line: where the repository is, and its last
/// commit (short id and subject).
pub(crate) fn path_line(
    row: &RepoRow,
    columns: &[Column],
    text_width: usize,
    theme: &Theme,
) -> Line<'static> {
    const SHA_WIDTH: usize = 7;
    let ell = theme.glyphs.ellipsis();
    // The path sits under the name and branch columns.
    let path_width = width_of(columns, ColumnId::Name) + width_of(columns, ColumnId::Branch);
    let path = row
        .snap
        .path
        .as_ref()
        .map(|p| text::abbrev_home(p.as_str()))
        .unwrap_or_default();
    let path = text::clip_start(&text::safe(&path), path_width.saturating_sub(2), ell);
    let mut spans = vec![
        Span::raw(" ".repeat(MARK_WIDTH)),
        Span::styled(text::cell(&path, path_width, ell), theme.dim()),
    ];
    // Two columns of margin on the right, as in the design.
    let rest = text_width.saturating_sub(MARK_WIDTH + path_width + 2);
    match (&row.snap.error, &row.snap.last_commit) {
        (Some(error), _) => spans.push(Span::styled(
            text::clip(&text::safe(error), rest, ell),
            theme.risk(),
        )),
        (None, Some(commit)) => {
            // Git's own abbreviation can be longer than seven characters in a
            // big repository. Any prefix of a hash is still a valid short id,
            // so cut it rather than mark it with an ellipsis ("eee44f…" reads
            // as damage and shows less of the hash).
            let short: String = text::safe(&commit.short_id)
                .chars()
                .take(SHA_WIDTH)
                .collect();
            let sha = format!("{short:<SHA_WIDTH$}");
            let subject_width = rest.saturating_sub(SHA_WIDTH + 2);
            spans.push(Span::styled(sha, theme.ahead()));
            spans.push(Span::raw("  "));
            spans.push(Span::raw(text::clip(
                &text::safe(&commit.summary),
                subject_width,
                ell,
            )));
        }
        (None, None) => spans.push(Span::styled("no commits yet", theme.dim())),
    }
    Line::from(spans)
}

/// The mark in a row's gutter: a spinner while an action runs on it, a dot
/// when it is marked for a bulk action.
pub(crate) fn gutter(row: &RepoRow, app: &App, theme: &Theme) -> Option<Span<'static>> {
    if row.busy {
        Some(Span::styled(
            theme.glyphs.spinner(app.spinner),
            theme.accent(),
        ))
    } else if row.marked {
        Some(Span::styled(theme.glyphs.marked(), theme.ahead().bold()))
    } else {
        None
    }
}

/// The title of the table card: its name and count, then whatever is
/// narrowing or reordering it, so a short list is never a mystery.
pub(crate) fn table_title(app: &App, shown: usize, theme: &Theme) -> Vec<Span<'static>> {
    let total = app.repos.len();
    let count = if shown == total {
        total.to_string()
    } else {
        format!("{shown} of {total}")
    };
    let mut spans = vec![
        Span::styled("Repositories", Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {count}"), theme.dim()),
    ];
    let mut note = |text: String, style: Style| {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(text, style));
    };
    if let Some(scope) = &app.scope {
        note(format!("only: {}", scope.label), theme.warn());
    }
    if app.dirty_only {
        note("only: needs attention".to_string(), theme.warn());
    }
    if app.mode == Mode::Filter || !app.filter.is_empty() {
        let caret = if app.mode == Mode::Filter {
            theme.glyphs.caret()
        } else {
            ""
        };
        note(
            format!("/ {}{caret}", text::safe(&app.filter)),
            theme.highlight(),
        );
    }
    if app.sort != cohors_core::SortMode::default() {
        note(format!("sort: {}", app.sort.label()), theme.dim());
    }
    if !app.selection.is_empty() {
        note(format!("{} marked", app.selection.len()), theme.ahead());
    }
    spans
}

/// The keys on the table card's bottom border. `open` names what Enter does,
/// which differs between the two screens.
pub(crate) fn table_hints(kind: TableKind, theme: &Theme) -> Vec<Hint> {
    let mut hints = vec![hint(theme.glyphs.key_space(), "select")];
    if kind == TableKind::Overview {
        hints.push(hint(theme.glyphs.key_enter(), "detail"));
    }
    for (key, label) in [
        ("f", "fetch"),
        ("p", "pull"),
        ("P", "push"),
        ("c", "commit"),
        ("S", "stash"),
        ("o", "open"),
        ("/", "filter"),
        (">", "sort"),
    ] {
        hints.push(hint(key, label));
    }
    if kind == TableKind::Detail {
        hints.push(hint("esc", "overview"));
    }
    hints
}

/// What the table says when it has no rows, and how to get them back.
pub(crate) fn empty_message(app: &App) -> String {
    if !app.filter.trim().is_empty() {
        format!(
            "No repository matches \"{}\". Press esc to clear the filter.",
            text::safe(&app.filter)
        )
    } else if app.scope.is_some() {
        "None of those repositories are in the fleet any more. Press esc to show them all."
            .to_string()
    } else if app.dirty_only {
        "No repository needs attention. Type :dirty to show them all again.".to_string()
    } else {
        "No repositories.".to_string()
    }
}

/// Card 5 of the overview: the repository table.
pub(crate) fn render_repos_card(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    now: i64,
    theme: &Theme,
    focused: bool,
) {
    let view = app.view();
    let card = CardFrame {
        number: Some(5),
        title: table_title(app, view.len(), theme),
        hints: table_hints(TableKind::Overview, theme),
        focused,
    };
    let inner = draw_card(frame, area, &card, theme);
    if inner.text.height == 0 {
        return;
    }
    let columns = columns(TableKind::Overview, inner.width());
    frame.render_widget(
        Paragraph::new(header_line(TableKind::Overview, &columns, theme)),
        Rect {
            height: 1,
            ..inner.text
        },
    );
    let body = inner.below(1);
    if view.is_empty() {
        cards::draw_message(frame, body.text, &empty_message(app), theme);
        return;
    }
    draw_rows(frame, body, app, &view, &columns, now, theme, focused);
}

/// The table's rows, scrolled to keep the cursor in view.
#[allow(clippy::too_many_arguments)] // a renderer: frame, area, data, clock, theme
fn draw_rows(
    frame: &mut Frame,
    body: CardArea,
    app: &App,
    view: &[ViewRow],
    columns: &[Column],
    now: i64,
    theme: &Theme,
    focused: bool,
) {
    let window = list_window(
        app.scrolls.table.get(),
        app.selected,
        body.text.height as usize,
        view.len(),
    );
    app.scrolls.table.set(window.offset);
    // The cursor only shows while the card has the focus.
    let cursor = focused.then_some(app.selected);
    draw_list(frame, body, window, 1, cursor, theme, |index| {
        let row = RepoRow::new(app, &view[index]);
        Item {
            lines: vec![row_line(&row, columns, now, theme)],
            gutter: gutter(&row, app, theme),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(columns: &[Column]) -> Vec<ColumnId> {
        columns.iter().map(|c| c.id).collect()
    }

    fn total(columns: &[Column]) -> usize {
        MARK_WIDTH + columns.iter().map(|c| c.width).sum::<usize>()
    }

    #[test]
    fn the_designed_width_gets_the_designed_columns() {
        // 119 columns: 115 of text inside the card.
        let overview = columns(TableKind::Overview, 115);
        let widths: Vec<usize> = overview.iter().map(|c| c.width).collect();
        // The severity block takes seven columns; the commit subject gives them up.
        assert_eq!(widths, [24, 15, 12, 5, 5, 5, 7, 7, 6, 22]);
        assert_eq!(total(&overview), 115);

        let detail = columns(TableKind::Detail, 115);
        let widths: Vec<usize> = detail.iter().map(|c| c.width).collect();
        assert_eq!(widths, [30, 19, 12, 5, 5, 5, 7, 7, 10, 8]);
    }

    #[test]
    fn columns_drop_in_order_as_the_table_narrows() {
        // 80-column terminal: the commit subject goes first, then the task
        // count (card 3 already shows tasks per repository).
        let at_80 = columns(TableKind::Overview, 76);
        assert!(!ids(&at_80).contains(&ColumnId::Commit));
        assert!(!ids(&at_80).contains(&ColumnId::Tasks));
        assert!(ids(&at_80).contains(&ColumnId::Stash));
        // A little wider and the task count is back.
        assert!(ids(&columns(TableKind::Overview, 84)).contains(&ColumnId::Tasks));

        // Narrower still: tasks, then stash, then upstream, then branch.
        let narrow = columns(TableKind::Overview, 44);
        assert_eq!(
            ids(&narrow),
            [
                ColumnId::Name,
                ColumnId::Staged,
                ColumnId::Modified,
                ColumnId::Untracked,
                ColumnId::Age
            ]
        );

        let detail = columns(TableKind::Detail, 70);
        assert!(!ids(&detail).contains(&ColumnId::Fetched));
        assert!(ids(&detail).contains(&ColumnId::Upstream));
    }

    #[test]
    fn a_table_never_overflows_its_card() {
        for kind in [TableKind::Overview, TableKind::Detail] {
            for width in 44..200 {
                let cols = columns(kind, width);
                assert!(total(&cols) <= width, "{kind:?} at {width}: {cols:?}");
                // The name is always there, and never narrower than its floor.
                assert_eq!(cols[0].id, ColumnId::Name);
                assert!(cols[0].width >= 14);
            }
        }
    }

    #[test]
    fn fetched_age_reads_in_days() {
        const DAY: i64 = 86_400;
        let now = 1_700_000_000;
        assert_eq!(fetched_label(None, now), "never");
        assert_eq!(fetched_label(Some(now - 3_600), now), "recent");
        assert_eq!(fetched_label(Some(now - 36 * DAY), now), "36d ago");
        // A fetch stamped in the future (clock skew) is simply recent.
        assert_eq!(fetched_label(Some(now + 60), now), "recent");
    }
}
