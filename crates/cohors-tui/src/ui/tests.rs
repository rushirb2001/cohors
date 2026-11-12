//! Rendering tests. A text snapshot is what the screen would show, drawn on
//! ratatui's in-memory `TestBackend`; the `.snap` files under `snapshots/` are
//! the reviewed, expected output. Colours are not part of a text snapshot, so
//! the few things only colour or style can show are asserted on the buffer
//! directly.

use super::*;
use crate::app::{App, Card, Mode, Pane, Screen};
use cohors_core::launchpad::Board;
use cohors_core::{
    Branch, BranchInfo, CiStatus, CommitMeta, RemoteInfo, RepoId, RepoSnapshot, Upstream,
    WorktreeStatus,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// Fixed "now" so relative commit ages render deterministically.
const NOW: i64 = 1_700_000_000;

#[allow(clippy::too_many_arguments)]
fn snap(
    name: &str,
    branch: Branch,
    upstream: Option<(&str, u32, u32)>,
    worktree: (u32, u32, u32),
    stash: u32,
    commit: Option<(i64, &str)>,
    error: Option<&str>,
) -> RepoSnapshot {
    RepoSnapshot {
        id: RepoId(name.to_string()),
        name: name.to_string(),
        path: Some(camino::Utf8PathBuf::from(format!("/repos/{name}"))),
        branch,
        upstream: upstream.map(|(n, a, b)| Upstream {
            name: n.to_string(),
            ahead: a,
            behind: b,
        }),
        worktree: WorktreeStatus {
            staged: worktree.0,
            modified: worktree.1,
            untracked: worktree.2,
            conflicted: 0,
        },
        stash_count: stash,
        stash_latest: None,
        remote_url: None,
        operation: None,
        default_branch: None,
        last_fetch: None,
        latest_tag: None,
        remote: None,
        last_commit: commit.map(|(ts, summary)| CommitMeta {
            short_id: "a1b2c3d".to_string(),
            author: "Dev".to_string(),
            timestamp: ts,
            summary: summary.to_string(),
        }),
        error: error.map(str::to_string),
        activity: Vec::new(),
        groups: Vec::new(),
        branches: Vec::new(),
    }
}

/// Give `app` the overview of its own snapshots, as the event loop would
/// after a scan (here without any launchpad files).
fn with_overview(mut app: App) -> App {
    let overview = cohors_core::overview(&app.repos, &[], &Board::default(), NOW);
    app.set_overview(overview, Vec::new());
    app.scanned_at = Some(NOW - 120);
    app
}

/// A small hand-built fleet: five repositories in clearly different states.
/// The overlay tests draw on top of it.
fn small_app() -> App {
    let mut app = App::new(
        vec!["~/projects".to_string(), "~/work".to_string()],
        "~/.config/cohors/config.toml".to_string(),
    );
    let remote = |open_prs: u32, ci: CiStatus| {
        Some(RemoteInfo {
            host: "github.com".to_string(),
            owner: "demo".to_string(),
            repo: "demo".to_string(),
            default_branch: "main".to_string(),
            open_prs,
            open_prs_by_base: std::collections::HashMap::from([("main".to_string(), open_prs)]),
            prs_awaiting_review: 0,
            ci,
            ci_by_branch: std::collections::HashMap::new(),
            description: None,
            topics: Vec::new(),
            stars: 0,
            forks: 0,
            watchers: 0,
        })
    };
    let mut payments = snap(
        "payments",
        Branch::Named("main".into()),
        Some(("origin/main", 2, 0)),
        (0, 3, 1),
        1,
        Some((NOW - 7200, "fix: retry on 5xx")),
        None,
    );
    payments.remote = remote(2, CiStatus::Passing);
    // Two local branches, so the "push which branch?" picker has a choice.
    payments.branches = vec![
        BranchInfo {
            name: "main".into(),
            ahead: 2,
            behind: 0,
            has_upstream: true,
            is_current: true,
            last_commit: Some(CommitMeta {
                short_id: "a1b2c3d".into(),
                author: "Dev".into(),
                timestamp: NOW - 7200,
                summary: "fix: retry on 5xx".into(),
            }),
        },
        BranchInfo {
            name: "feat/checkout-rework".into(),
            ahead: 5,
            behind: 1,
            has_upstream: true,
            is_current: false,
            last_commit: Some(CommitMeta {
                short_id: "e5f6a7b".into(),
                author: "Dev".into(),
                timestamp: NOW - 86_400,
                summary: "wip: checkout".into(),
            }),
        },
    ];
    let mut web_app = snap(
        "web-app",
        Branch::Named("feat/checkout".into()),
        Some(("origin/feat", 0, 5)),
        (0, 7, 0),
        0,
        Some((NOW - 1200, "wip: cart drawer")),
        None,
    );
    web_app.remote = remote(0, CiStatus::Failing);
    let mut auth_service = snap(
        "auth-service",
        Branch::Named("main".into()),
        None,
        (0, 0, 0),
        0,
        Some((NOW - 259_200, "chore: bump deps")),
        None,
    );
    auth_service.remote = remote(1, CiStatus::Pending);
    app.set_repos(vec![
        payments,
        web_app,
        auth_service,
        snap(
            "infra",
            Branch::Detached("a1b2c3d".into()),
            None,
            (0, 0, 4),
            0,
            Some((NOW - 604_800, "build: pin ci image")),
            None,
        ),
        snap(
            "legacy-billing",
            Branch::Unborn,
            None,
            (0, 0, 0),
            0,
            None,
            Some("could not read .git (permission denied)"),
        ),
    ]);
    with_overview(app)
}

/// The built-in sample fleet with its launchpad: what `cohors demo` shows.
fn fleet_app() -> App {
    let mut app = App::new(
        vec!["~/code".to_string()],
        "~/.config/cohors/config.toml".to_string(),
    );
    crate::tui::seed_demo(&mut app, NOW);
    app.scanned_at = Some(NOW - 120);
    app
}

/// The buffer as text, one line per row.
fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in buf.area.top()..buf.area.bottom() {
        for x in buf.area.left()..buf.area.right() {
            if let Some(cell) = buf.cell((x, y)) {
                out.push_str(cell.symbol());
            }
        }
        out.push('\n');
    }
    // Normalize the version so a bump doesn't churn every snapshot — these
    // tests assert layout, not the version number.
    out.replace(&format!("v{}", env!("CARGO_PKG_VERSION")), "vX.Y.Z")
}

/// Draw `app` with an explicit colour setting and return the terminal.
fn draw(app: &App, w: u16, h: u16, color: bool) -> Terminal<TestBackend> {
    let theme = Theme::new(app.icons, app.dark, color);
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| render_with(f, app, NOW, &theme)).unwrap();
    terminal
}

/// The screen in colour (colour itself is not part of the text).
fn render_to_string(app: &App, w: u16, h: u16) -> String {
    buffer_text(&draw(app, w, h, true))
}

/// The screen as a `NO_COLOR` terminal shows it: the plain-text glyph tier.
fn render_plain(app: &App, w: u16, h: u16) -> String {
    buffer_text(&draw(app, w, h, false))
}

// ── The overview ─────────────────────────────────────────────────────────────

/// The screen cohors opens on, with the sample fleet.
#[test]
fn snapshot_overview() {
    insta::assert_snapshot!(render_to_string(&fleet_app(), 120, 45));
}

/// The size the design was drawn at: 119 columns by 53 rows.
#[test]
fn snapshot_overview_designed_size() {
    insta::assert_snapshot!(render_to_string(&fleet_app(), 119, 53));
}

/// A classic 80 by 24 terminal: one-line header, the figures as one line, and
/// all five cards still present.
#[test]
fn snapshot_overview_80x24() {
    insta::assert_snapshot!(render_to_string(&fleet_app(), 80, 24));
}

/// Narrow and tall: cards 3 and 4 stack, and columns drop from the tables.
#[test]
fn snapshot_overview_narrow() {
    insta::assert_snapshot!(render_to_string(&fleet_app(), 60, 50));
}

/// Too narrow for six figure cards in a row but tall: they wrap three by two.
#[test]
fn snapshot_overview_figure_grid() {
    insta::assert_snapshot!(render_to_string(&fleet_app(), 96, 60));
}

/// No colour and the plain-text glyph tier: severity is a character, the
/// focused card has a heavier frame, and nothing outside ASCII is drawn.
#[test]
fn snapshot_overview_no_color_ascii() {
    let app = fleet_app();
    let screen = render_plain(&app, 120, 45);
    assert!(screen.is_ascii(), "the plain tier must be pure ASCII");
    insta::assert_snapshot!(screen);
}

/// An empty fleet says what is missing and how to add a root.
#[test]
fn snapshot_overview_empty() {
    let app = with_overview(App::new(
        vec!["~/projects".to_string()],
        "~/.config/cohors/config.toml".to_string(),
    ));
    insta::assert_snapshot!(render_to_string(&app, 120, 45));
}

/// Empty fleet, but repositories were detected elsewhere: the one-key rescue.
#[test]
fn snapshot_overview_empty_picker() {
    let mut app = App::new(
        vec!["~/projects".to_string()],
        "~/.config/cohors/config.toml".to_string(),
    );
    app.suggested_roots = vec!["~/code".to_string(), "~/work".to_string()];
    insta::assert_snapshot!(render_to_string(&app, 100, 30));
}

/// The first moments of a run: scanning, nothing to show yet.
#[test]
fn snapshot_overview_scanning() {
    let mut app = App::new(vec!["~/projects".to_string()], "cfg".to_string());
    app.scanning = true;
    insta::assert_snapshot!(render_to_string(&app, 100, 20));
}

/// Repositories but no launchpad files: the tasks card explains what would
/// make tasks appear, and the open-tasks figure says there is no launchpad.
#[test]
fn snapshot_overview_without_launchpad() {
    insta::assert_snapshot!(render_to_string(&small_app(), 120, 45));
}

/// Snapshots painted from the cache before the overview has been computed:
/// the table already lists the repositories, the cards say they are waiting.
#[test]
fn snapshot_overview_before_it_is_computed() {
    let mut app = small_app();
    app.overview = None;
    insta::assert_snapshot!(render_to_string(&app, 120, 40));
}

/// Each card focused once. The cursor mark in the gutter shows which card has
/// the focus (the accent border is colour, so it is asserted separately).
#[test]
fn snapshot_focus_do_next() {
    let mut app = fleet_app();
    app.focus = Card::DoNext;
    app.cursors.next = 1;
    insta::assert_snapshot!(render_to_string(&app, 120, 45));
}

#[test]
fn snapshot_focus_at_risk() {
    let mut app = fleet_app();
    app.focus = Card::AtRisk;
    app.cursors.risk = 1;
    insta::assert_snapshot!(render_to_string(&app, 120, 45));
}

#[test]
fn snapshot_focus_tasks() {
    let mut app = fleet_app();
    app.focus = Card::Tasks;
    app.cursors.tasks = 1;
    insta::assert_snapshot!(render_to_string(&app, 120, 45));
}

#[test]
fn snapshot_focus_commits() {
    let mut app = fleet_app();
    app.focus = Card::Commits;
    app.cursors.commits = 2;
    insta::assert_snapshot!(render_to_string(&app, 120, 45));
}

/// The table focused, with the cursor far enough down that it has scrolled.
#[test]
fn snapshot_focus_repos() {
    let mut app = fleet_app();
    app.focus = Card::Repos;
    app.selected = app.visible_len() - 1;
    insta::assert_snapshot!(render_to_string(&app, 120, 45));
}

/// Marked repositories show a dot in the gutter and a count in the title.
#[test]
fn snapshot_repos_marked() {
    let mut app = small_app();
    app.focus = Card::Repos;
    app.selection.insert(RepoId("payments".to_string()));
    app.selection.insert(RepoId("web-app".to_string()));
    insta::assert_snapshot!(render_to_string(&app, 120, 40));
}

/// Typing a filter narrows the table and shows the query in its title.
#[test]
fn snapshot_repos_filtered() {
    let mut app = small_app();
    app.focus = Card::Repos;
    app.mode = Mode::Filter;
    app.filter = "pay".to_string();
    insta::assert_snapshot!(render_to_string(&app, 120, 40));
}

/// A filter that matches nothing says so, and how to clear it.
#[test]
fn snapshot_repos_filter_matches_nothing() {
    let mut app = small_app();
    app.focus = Card::Repos;
    app.filter = "zzz".to_string();
    insta::assert_snapshot!(render_to_string(&app, 120, 40));
}

/// A "Do next" step that needs a person narrows the table to its repositories.
#[test]
fn snapshot_repos_narrowed_to_a_step() {
    let mut app = fleet_app();
    app.focus = Card::DoNext;
    let step = app
        .actions()
        .iter()
        .position(|action| action.verb.is_none())
        .expect("the sample fleet has a step that needs a look");
    app.cursors.next = step;
    app.on_key(ratatui::crossterm::event::KeyEvent::from(
        ratatui::crossterm::event::KeyCode::Enter,
    ));
    assert_eq!(app.focus, Card::Repos);
    insta::assert_snapshot!(render_to_string(&app, 120, 45));
}

/// A filter that was just opened: the title shows the empty prompt and the
/// key bar says what typing does.
#[test]
fn snapshot_repos_filter_just_opened() {
    let mut app = small_app();
    app.focus = Card::Repos;
    app.mode = Mode::Filter;
    insta::assert_snapshot!(render_to_string(&app, 120, 40));
}

/// `:dirty` keeps only the repositories that need attention, and the title
/// says the table is narrowed.
#[test]
fn snapshot_repos_needs_attention_only() {
    let mut app = small_app();
    app.focus = Card::Repos;
    app.dirty_only = true;
    insta::assert_snapshot!(render_to_string(&app, 120, 40));
}

/// A sort other than the default is named in the title.
#[test]
fn snapshot_repos_sorted_by_name() {
    let mut app = small_app();
    app.focus = Card::Repos;
    app.sort = cohors_core::SortMode::Name;
    insta::assert_snapshot!(render_to_string(&app, 120, 40));
}

/// While an action runs on a repository its gutter shows a spinner.
#[test]
fn snapshot_repos_busy() {
    let mut app = small_app();
    app.focus = Card::Repos;
    app.busy.insert(RepoId("payments".to_string()));
    app.busy.insert(RepoId("web-app".to_string()));
    app.status = Some("fetching 2 repos…".to_string());
    insta::assert_snapshot!(render_to_string(&app, 120, 40));
}

/// Scrolled to the middle of a table that does not fit: the last line counts
/// what is hidden both below and above.
#[test]
fn snapshot_repos_scrolled_to_the_middle() {
    let mut app = fleet_app();
    app.focus = Card::Repos;
    // Walk down one row at a time, as the arrow key would, drawing each frame
    // so the table scrolls only when the cursor reaches its edge.
    for _ in 0..7 {
        app.selected += 1;
        render_to_string(&app, 120, 45);
    }
    let screen = render_to_string(&app, 120, 45);
    assert!(screen.contains("more") && screen.contains("above"));
    insta::assert_snapshot!(screen);
}

/// The plain-text tier marks a repository for a bulk action with `*`.
#[test]
fn snapshot_repos_marked_no_color_ascii() {
    let mut app = small_app();
    app.focus = Card::Repos;
    app.selection.insert(RepoId("payments".to_string()));
    let screen = render_plain(&app, 120, 40);
    assert!(screen.is_ascii(), "the plain tier must be pure ASCII");
    insta::assert_snapshot!(screen);
}

/// An action's result is shown at the right end of the key bar.
#[test]
fn snapshot_toast() {
    let mut app = small_app();
    app.status = Some("pushed 3 repos".to_string());
    insta::assert_snapshot!(render_to_string(&app, 120, 40));
}

/// Below the size floor the dashboard is replaced by a short message.
#[test]
fn snapshot_too_small() {
    insta::assert_snapshot!(render_to_string(&fleet_app(), 40, 10));
}

// ── The detail view ──────────────────────────────────────────────────────────

/// The detail view of the sample fleet with a repository that has tasks
/// selected, so all three cards have something to show.
fn detail_app() -> App {
    let mut app = fleet_app();
    app.screen = Screen::Repos;
    app.focus = Card::Repos;
    app.selected = app
        .view()
        .iter()
        .position(|row| app.repos[row.index].name == "payments")
        .expect("the sample fleet has a payments repository");
    app
}

#[test]
fn snapshot_detail_view() {
    insta::assert_snapshot!(render_to_string(&detail_app(), 120, 45));
}

/// The size the design was drawn at.
#[test]
fn snapshot_detail_view_designed_size() {
    insta::assert_snapshot!(render_to_string(&detail_app(), 119, 30));
}

/// One of the three cards focused: it gets the frame, the list keeps its
/// cursor.
#[test]
fn snapshot_detail_view_tasks_focused() {
    let mut app = detail_app();
    app.pane = Pane::Tasks;
    insta::assert_snapshot!(render_to_string(&app, 120, 45));
}

/// A narrow screen drops list columns and shows two of the three cards.
#[test]
fn snapshot_detail_view_narrow() {
    insta::assert_snapshot!(render_to_string(&detail_app(), 80, 30));
}

/// The cursor on the last repository: the list has scrolled with it, and the
/// note at the bottom counts the entries above.
#[test]
fn snapshot_detail_view_scrolled_to_the_end() {
    let mut app = detail_app();
    app.selected = app.visible_len() - 1;
    insta::assert_snapshot!(render_to_string(&app, 120, 30));
}

/// A screen too short for the three cards keeps the list and the key bar.
#[test]
fn snapshot_detail_view_short() {
    insta::assert_snapshot!(render_to_string(&detail_app(), 100, 16));
}

/// The unreadable repository: no state to show, only why it could not be read.
#[test]
fn snapshot_detail_view_unreadable() {
    let mut app = detail_app();
    app.selected = app
        .view()
        .iter()
        .position(|row| app.repos[row.index].has_error())
        .expect("the sample fleet has an unreadable repository");
    insta::assert_snapshot!(render_to_string(&app, 120, 45));
}

#[test]
fn snapshot_detail_view_no_color_ascii() {
    let screen = render_plain(&detail_app(), 120, 45);
    assert!(screen.is_ascii(), "the plain tier must be pure ASCII");
    insta::assert_snapshot!(screen);
}

// ── Geometry: the approved frames, measured ──────────────────────────────────
//
// The approved design is a text file drawn at 119 columns. These tests pin the
// column each element starts at to the column it has in that file, so a layout
// change that drifts from the design fails here rather than being noticed (or
// not) by eye in a snapshot.

/// The screen as rows of characters, so a column number means a terminal
/// column (not a byte offset).
fn rows_of(screen: &str) -> Vec<Vec<char>> {
    screen.lines().map(|line| line.chars().collect()).collect()
}

/// The first row that contains every one of `words`.
fn row_with<'a>(rows: &'a [Vec<char>], words: &[&str]) -> &'a Vec<char> {
    rows.iter()
        .find(|row| {
            let text: String = row.iter().collect();
            words.iter().all(|word| text.contains(word))
        })
        .unwrap_or_else(|| panic!("no row with {words:?}"))
}

/// The column `word` starts at in `row`.
fn column_of(row: &[char], word: &str) -> usize {
    let text: String = row.iter().collect();
    let byte = text
        .find(word)
        .unwrap_or_else(|| panic!("{word:?} not in {text:?}"));
    text[..byte].chars().count()
}

/// Every column in `row` holding one of the card-frame characters.
fn frame_columns(row: &[char]) -> Vec<usize> {
    row.iter()
        .enumerate()
        .filter(|(_, c)| ['│', '╭', '╮', '╰', '╯'].contains(c))
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn the_overview_at_119_columns_matches_the_approved_frame() {
    let rows = rows_of(&render_to_string(&fleet_app(), 119, 53));

    // Header: the mark, the wordmark, and the right-hand block.
    let top = row_with(&rows, &["cohors", "your git fleet"]);
    assert_eq!(column_of(top, "▜"), 2);
    assert_eq!(column_of(top, "cohors"), 14);
    assert_eq!(column_of(top, "commits,"), 85);
    assert_eq!(column_of(row_with(&rows, &["scanned"]), "scanned"), 78);
    assert_eq!(column_of(&rows[2], "▅"), 78); // the sparkline's first bar

    // Six figure cards, 19 columns each, one column apart.
    let figures = row_with(&rows, &["Not backed up", "Commits 7d"]);
    assert_eq!(
        frame_columns(figures),
        [0, 18, 20, 38, 40, 58, 60, 78, 80, 98, 100, 118]
    );

    // Card 1: the severity block (six columns, the word one column in), then
    // the key, then the title.
    let step = row_with(&rows, &["Push 4 commits"]);
    assert_eq!(column_of(step, "RISK"), 3);
    assert_eq!(column_of(step, "P"), 10);
    assert_eq!(column_of(step, "Push"), 13);

    // Card 2: block, name, branch, reasons; the age and the step sit at the right.
    let risk = row_with(&rows, &["design-system", "1 stash"]);
    assert_eq!(column_of(risk, "WARN"), 3);
    assert_eq!(column_of(risk, "design-system"), 9);
    assert_eq!(column_of(risk, "main"), 31);
    assert_eq!(column_of(risk, "1 commit"), 46);
    assert_eq!(frame_columns(risk), [0, 118]);

    // Cards 3 and 4 side by side: 50 columns, a gap, then 68.
    let tasks = row_with(&rows, &["repository", "open", "high", "focus"]);
    assert_eq!(frame_columns(tasks), [0, 49, 51, 118]);
    assert_eq!(column_of(tasks, "open"), 28);
    assert_eq!(column_of(tasks, "high"), 34);
    assert_eq!(column_of(tasks, "focus"), 40);
    // The commit bars start 22 columns into card 4.
    assert_eq!(column_of(tasks, "█"), 75);

    // Card 5: the table's columns.
    let header = row_with(&rows, &["repository", "upstream", "last commit"]);
    // The severity block takes the first seven columns of every row, so the
    // name starts at 9; the commit subject gives up the room.
    for (label, column) in [
        ("repository", 9),
        ("branch", 33),
        ("upstream", 48),
        ("stg", 62),
        ("mod", 67),
        ("new", 72),
        ("stash", 77),
        ("tasks", 84),
        ("age", 92),
        ("last commit", 97),
    ] {
        assert_eq!(column_of(header, label), column, "{label}");
    }

    // The key bar: two columns in, hints four apart.
    let keys = rows.last().unwrap();
    assert_eq!(column_of(keys, "1-5"), 2);
    assert_eq!(column_of(keys, "tab"), 15);
}

#[test]
fn the_detail_view_at_119_columns_matches_the_approved_frame() {
    let rows = rows_of(&render_to_string(&detail_app(), 119, 40));

    let header = row_with(&rows, &["repository", "fetched", "age"]);
    // As in the overview, the severity block leads every row, so the name
    // starts at 9.
    for (label, column) in [
        ("repository", 9),
        ("branch", 39),
        // The branch column is one wider than designed: the fleet has a
        // detached head whose label needs it, and the row had a column spare.
        ("upstream", 60),
        ("stg", 74),
        ("mod", 79),
        ("new", 84),
        ("stash", 89),
        ("tasks", 96),
        ("fetched", 104),
        ("age", 114),
    ] {
        assert_eq!(column_of(header, label), column, "{label}");
    }

    // One line per repository. The path is said once, in the card about the
    // selected repository, not under every name in the list.
    let with_path = rows
        .iter()
        .filter(|row| row.iter().collect::<String>().contains("~/code/"))
        .count();
    assert_eq!(with_path, 1);

    // Three cards of 39 columns, one column apart.
    let cards = row_with(&rows, &["staged", "last commit"]);
    assert_eq!(frame_columns(cards), [0, 38, 40, 78, 80, 118]);
    let state = row_with(&rows, &["upstream", "origin/main"]);
    assert_eq!(column_of(state, "upstream"), 2);
    assert_eq!(column_of(state, "origin/main"), 14);
}

// ── Style: what a text snapshot cannot show ──────────────────────────────────

/// The cell at the top-left corner of the first card whose title contains
/// `title`.
fn corner_of<'a>(terminal: &'a Terminal<TestBackend>, title: &str) -> &'a ratatui::buffer::Cell {
    let buf = terminal.backend().buffer();
    let text = buffer_text(terminal);
    let (y, line) = text
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(title))
        .unwrap_or_else(|| panic!("no card titled {title:?}"));
    // Byte offset → column: count the characters before the title, then step
    // back to the corner that starts this card.
    let col = line[..line.find(title).unwrap()].chars().count();
    let corner = line
        .chars()
        .take(col)
        .collect::<Vec<_>>()
        .iter()
        .rposition(|c| ['╭', '╔', '+', '#'].contains(c))
        .expect("a corner before the title");
    buf.cell((corner as u16, y as u16)).unwrap()
}

/// The cell where `word` starts on the first screen row containing every one
/// of `on_row_with`.
fn cell_at<'a>(
    terminal: &'a Terminal<TestBackend>,
    on_row_with: &[&str],
    word: &str,
) -> &'a ratatui::buffer::Cell {
    let text = buffer_text(terminal);
    let (y, line) = text
        .lines()
        .enumerate()
        .find(|(_, line)| on_row_with.iter().all(|w| line.contains(w)))
        .unwrap_or_else(|| panic!("no row with {on_row_with:?}"));
    let x = line[..line.find(word).unwrap()].chars().count();
    terminal
        .backend()
        .buffer()
        .cell((x as u16, y as u16))
        .unwrap()
}

/// Severity blocks are filled for risk and warning and quiet otherwise, in
/// the muted tones of the neutral theme, and fall back to reverse video when
/// there is no colour.
#[test]
fn severity_blocks_are_filled_only_where_work_could_be_lost() {
    use ratatui::style::{Color, Modifier};
    let brick = Color::Rgb(0xD9, 0x77, 0x6E);
    let ochre = Color::Rgb(0xCF, 0xA2, 0x55);
    let mut app = fleet_app();
    app.dark = true;
    let colour = draw(&app, 119, 60, true);

    // Do next, Work at risk and the repository table all lead with the block.
    assert_eq!(cell_at(&colour, &["Push 4 commits"], "RISK").bg, brick);
    assert_eq!(
        cell_at(&colour, &["design-system", "1 stash"], "WARN").bg,
        ochre
    );
    assert_eq!(cell_at(&colour, &["analytics", "perf:"], "RISK").bg, brick);
    // A note is not filled: it must not compete with real trouble.
    let note = cell_at(&colour, &["cli-tools", "docs:"], "NOTE");
    assert_eq!(note.bg, Color::Reset);
    assert!(note.modifier.contains(Modifier::DIM));
    // A clean repository has no block at all.
    let clean = buffer_text(&colour);
    let row = clean
        .lines()
        .find(|l| l.contains("auth-service") && l.contains("chore:"))
        .unwrap();
    assert!(
        !["RISK", "WARN", "NOTE", "INFO"]
            .iter()
            .any(|w| row.contains(w))
    );

    // The figures that need attention are filled tiles; the rest are neutral.
    assert_eq!(
        cell_at(&colour, &["Not backed up", "Commits 7d"], "Not backed up").bg,
        brick
    );
    // Stashes turn ochre only once one has gone stale; fresh ones are neutral.
    let stale = app.overview.as_ref().unwrap().figures.stale_stash_repos > 0;
    assert_eq!(
        cell_at(&colour, &["Not backed up", "Commits 7d"], "Stashes").bg,
        if stale { ochre } else { Color::Reset }
    );
    assert_eq!(
        cell_at(&colour, &["Not backed up", "Commits 7d"], "Uncommitted").bg,
        Color::Reset
    );

    // A light terminal gets the deeper tones.
    app.dark = false;
    let light = draw(&app, 119, 60, true);
    assert_eq!(
        cell_at(&light, &["Push 4 commits"], "RISK").bg,
        Color::Rgb(0xB4, 0x48, 0x3F)
    );

    // Without colour the block is reverse video, and the word still says it.
    let plain = draw(&app, 119, 60, false);
    let block = cell_at(&plain, &["Push 4 commits"], "RISK");
    assert_eq!(block.bg, Color::Reset);
    assert!(block.modifier.contains(Modifier::REVERSED));
}

#[test]
fn the_focused_card_has_its_sections_colour_on_its_frame_and_the_others_do_not() {
    use ratatui::style::Color;
    let mut app = fleet_app();
    app.dark = true;
    app.focus = Card::AtRisk;
    let terminal = draw(&app, 120, 45, true);
    // Work at risk is brick; an unfocused frame is dim, with no colour.
    assert_eq!(
        corner_of(&terminal, "Work at risk").fg,
        Color::Rgb(0xD9, 0x77, 0x6E)
    );
    assert_eq!(corner_of(&terminal, "Do next").fg, Color::Reset);
    assert_eq!(corner_of(&terminal, "Repositories").fg, Color::Reset);
    // Every card's title is in its section's colour, focused or not.
    assert_eq!(
        cell_at(&terminal, &["Do next"], "Do next").fg,
        SPIDER_PURPLE
    );
}

// ── A background that is not known ───────────────────────────────────────────

/// The screen on a terminal that would not say what its background is.
fn draw_unknown_ground(app: &App, w: u16, h: u16) -> Terminal<TestBackend> {
    let theme = Theme::new(app.icons, true, true).with_unknown_ground();
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| render_with(f, app, NOW, &theme)).unwrap();
    terminal
}

/// When the terminal's background is not known, nothing is painted that is
/// only right on one kind of background. The only fills left are the ones
/// that bring their own text colour: severity blocks, the figure tiles that
/// need attention, and the cursor row.
#[test]
fn an_unknown_background_gets_no_surface_that_was_guessed() {
    use ratatui::style::{Color, Modifier};
    let mut app = fleet_app();
    app.focus = Card::Repos;
    let terminal = draw_unknown_ground(&app, 119, 60);
    let buf = terminal.backend().buffer();

    // Cards are line frames, exactly as with a known background and no panels.
    assert_eq!(
        buffer_text(&terminal),
        buffer_text(&draw(&app, 119, 60, true))
    );
    // Every filled cell names its own text colour.
    for cell in buf.content() {
        if cell.bg != Color::Reset {
            assert_ne!(
                cell.fg,
                Color::Reset,
                "{:?} on {:?}",
                cell.symbol(),
                cell.bg
            );
        }
    }
    // The colour grading is still there: titles in their section's colour.
    assert_eq!(
        cell_at(&terminal, &["Do next"], "Do next").fg,
        SPIDER_PURPLE
    );
    assert_eq!(
        cell_at(&terminal, &["Repositories", "12"], "Repositories").fg,
        Color::Rgb(0x5F, 0xAE, 0xA6)
    );
    // The cursor row is a solid colour with dark, undimmed text all across,
    // dim cells included (the commit subject is normally dim).
    let subject = cell_at(&terminal, &["analytics", "perf:"], "perf:");
    assert_eq!(subject.bg, SPIDER_PURPLE);
    assert_eq!(subject.fg, Color::Rgb(0x1C, 0x1B, 0x1A));
    assert!(!subject.modifier.contains(Modifier::DIM));

    // An overlay is a frame of lines too, with no filled surface.
    app.mode = Mode::Help;
    let help = draw_unknown_ground(&app, 119, 60);
    assert!(buffer_text(&help).contains('╔'));
}

/// With a known background the panel is a shade off THAT background, not a
/// fixed grey: lighter than a dark one, darker than a light one.
#[test]
fn a_panels_surface_is_a_shade_off_the_real_background() {
    use ratatui::style::Color;
    let brightness = |colour: Color| match colour {
        Color::Rgb(r, g, b) => u32::from(r) + u32::from(g) + u32::from(b),
        other => panic!("not a 24-bit colour: {other:?}"),
    };
    for (ground, dark) in [((0x00, 0x2B, 0x36), true), ((0xFD, 0xF6, 0xE3), false)] {
        let mut theme = Theme::new(cohors_config::IconMode::Unicode, dark, true).with_panels();
        theme.ground = Some(Color::Rgb(ground.0, ground.1, ground.2));
        let surface = theme.surface().unwrap();
        let (base, shade) = (brightness(theme.ground()), brightness(surface));
        if dark {
            assert!(shade > base && shade - base < 90, "{surface:?}");
        } else {
            assert!(shade < base && base - shade < 90, "{surface:?}");
        }
        // The hue of the background carries into the panel (a teal terminal
        // gets a teal-tinted panel, not a grey one).
        if let (Color::Rgb(_, g, b), true) = (surface, dark) {
            assert!(b > g, "{surface:?}");
        }
    }
}

#[test]
fn without_colour_the_focused_card_has_a_heavier_frame() {
    let mut app = fleet_app();
    app.icons = cohors_config::IconMode::Unicode;
    app.focus = Card::AtRisk;
    let terminal = draw(&app, 120, 45, false);
    assert_eq!(corner_of(&terminal, "Work at risk").symbol(), "╔");
    assert_eq!(corner_of(&terminal, "Do next").symbol(), "╭");
    // And no cell on the screen carries a colour.
    let buf = terminal.backend().buffer();
    assert!(
        buf.content()
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
    );
}

#[test]
fn the_cursor_row_is_washed_only_in_the_focused_card() {
    let mut app = fleet_app();
    app.focus = Card::DoNext;
    let terminal = draw(&app, 120, 45, true);
    let buf = terminal.backend().buffer();
    let wash = Theme::new(app.icons, false, true).selection().bg.unwrap();
    let washed_rows: Vec<u16> = (0..buf.area.height)
        .filter(|&y| (0..buf.area.width).any(|x| buf.cell((x, y)).unwrap().bg == wash))
        .collect();
    assert_eq!(
        washed_rows.len(),
        1,
        "exactly one cursor row: {washed_rows:?}"
    );
}

// ── Robustness ───────────────────────────────────────────────────────────────

/// A fleet built to break layouts: very long and non-ASCII names, control
/// characters in text that comes from repositories, an unreadable repository,
/// huge counts, and a repository with nothing known about it.
fn hostile_app() -> App {
    let mut app = App::new(
        vec!["/a/very/long/root/path/that/goes/on/and/on/for/a/while/projects".to_string()],
        "cfg".to_string(),
    );
    let long = "an-extremely-long-repository-name-that-no-column-could-ever-hold-in-full";
    let mut huge = snap(
        long,
        Branch::Named("feature/an/equally/long/branch/name/with/many/segments".into()),
        Some(("origin/main", 4_000_000, 3_000_000)),
        (u32::MAX / 4, u32::MAX / 4, u32::MAX / 4),
        9_999,
        Some((
            NOW - 30,
            "subject with \u{1b}[31mescapes\u{7} and a\ttab\nand newline",
        )),
        None,
    );
    huge.activity = vec![255; 60];
    let mut wide = snap(
        "日本語のリポジトリ名🚀",
        Branch::Detached("deadbee".into()),
        None,
        (0, 0, 1),
        0,
        Some((NOW + 500, "commit dated in the future")),
        None,
    );
    wide.remote_url = Some("git@example.com:x/y.git".to_string());
    wide.last_fetch = Some(NOW + 10_000);
    let mut bare = snap("", Branch::Unborn, None, (0, 0, 0), 0, None, None);
    bare.path = None;
    app.set_repos(vec![
        huge,
        wide,
        bare,
        snap(
            "broken",
            Branch::Unborn,
            None,
            (0, 0, 0),
            0,
            None,
            Some("fatal: \u{1b}[2Jnot a git repository (or any of the parent directories)"),
        ),
    ]);
    with_overview(app)
}

/// No terminal size, focus, screen or fleet may panic. This draws every size
/// from 1×1 upward for the sample fleet, an empty fleet and a hostile one.
#[test]
fn no_size_or_fleet_panics() {
    let empty = with_overview(App::new(Vec::new(), String::new()));
    let fleets = [fleet_app(), empty, hostile_app()];
    let sizes: Vec<(u16, u16)> = (1..=131)
        .step_by(13)
        .flat_map(|w| (1..=61).step_by(6).map(move |h| (w, h)))
        .chain([(48, 12), (47, 12), (48, 11), (80, 24), (200, 100), (300, 5)])
        .collect();
    for mut app in fleets {
        for (screen, focus, pane) in [
            (Screen::Overview, Card::DoNext, Pane::List),
            (Screen::Overview, Card::Commits, Pane::List),
            (Screen::Overview, Card::Repos, Pane::List),
            (Screen::Repos, Card::Repos, Pane::List),
            (Screen::Repos, Card::Repos, Pane::Tasks),
        ] {
            app.screen = screen;
            app.focus = focus;
            app.pane = pane;
            app.selected = app.visible_len().saturating_sub(1);
            for &(w, h) in &sizes {
                for color in [true, false] {
                    draw(&app, w, h, color);
                }
            }
        }
    }
}

/// Escape sequences in a commit subject or an error message are stripped
/// before they reach the screen.
#[test]
fn control_characters_from_repositories_are_not_drawn() {
    let mut app = hostile_app();
    app.focus = Card::Repos;
    for screen in [Screen::Overview, Screen::Repos] {
        app.screen = screen;
        let text = render_to_string(&app, 160, 50);
        assert!(!text.contains('\u{1b}') && !text.contains('\u{7}'));
    }
}

/// With colour off and ASCII data, both screens draw nothing outside ASCII in
/// any state: loaded, waiting for the overview, scanning, empty, with a status
/// message, or too small.
#[test]
fn the_plain_tier_is_ascii_in_every_state() {
    let empty = || App::new(vec!["~/projects".to_string()], "cfg".to_string());
    let mut waiting = small_app();
    waiting.overview = None;
    let mut scanning = empty();
    scanning.scanning = true;
    let mut rescue = empty();
    rescue.suggested_roots = vec!["~/code".to_string()];
    let mut busy = small_app();
    busy.busy.insert(RepoId("payments".to_string()));
    // Status messages are written with typographic punctuation.
    busy.status = Some("fetching 2 repos…  ·  1 skipped — conflict".to_string());
    let mut filtering = small_app();
    filtering.mode = Mode::Filter;
    filtering.filter = "zzz".to_string();

    let states = [
        ("loaded", fleet_app()),
        ("waiting", waiting),
        ("scanning", scanning),
        ("empty", with_overview(empty())),
        ("rescue", rescue),
        ("busy", busy),
        ("filtering", filtering),
    ];
    for (name, mut app) in states {
        for screen in [Screen::Overview, Screen::Repos] {
            app.screen = screen;
            for (w, h) in [(120, 45), (80, 24), (60, 50), (40, 10)] {
                let text = render_plain(&app, w, h);
                let stray: String = text.chars().filter(|c| !c.is_ascii()).collect();
                assert!(stray.is_empty(), "{name} {screen:?} {w}x{h}: {stray:?}");
            }
        }
    }
}

// ── Overlays (drawn on top of the overview) ──────────────────────────────────

/// The "Open with…" picker lists detected openers with the default marked.
#[test]
fn snapshot_open_with() {
    use crate::app::{OpenWith, Opener};
    let mut app = small_app();
    app.mode = Mode::OpenWith;
    let openers = vec![
        Opener::Editor {
            command: "code".to_string(),
            label: "VS Code".to_string(),
        },
        Opener::Editor {
            command: "nvim".to_string(),
            label: "Neovim".to_string(),
        },
        Opener::Reveal,
        Opener::Lazygit,
    ];
    app.open_with = Some(OpenWith::new(openers, Some("code".to_string())));
    insta::assert_snapshot!(render_to_string(&app, 100, 24));
}

/// When PATH has no editor CLI, the picker shows a note explaining why and
/// still offers Reveal / lazygit.
#[test]
fn snapshot_open_with_no_editor() {
    use crate::app::{OpenWith, Opener};
    let mut app = small_app();
    app.mode = Mode::OpenWith;
    let openers = vec![Opener::Reveal, Opener::Lazygit];
    app.open_with = Some(OpenWith::new(openers, None));
    insta::assert_snapshot!(render_to_string(&app, 100, 24));
}

/// The "push which branch?" picker lists a repo's local branches (current marked
/// and pre-selected) with each branch's push state — reached from `P` on a
/// multi-branch repo.
#[test]
fn snapshot_push_branch() {
    use crate::app::PushPick;
    let mut app = small_app();
    app.mode = Mode::PushBranch;
    let payments = app.repos.iter().find(|r| r.name == "payments").unwrap();
    app.push_pick = Some(PushPick {
        id: payments.id.clone(),
        name: payments.name.clone(),
        branches: payments.branches.clone(),
        cursor: 0,
    });
    insta::assert_snapshot!(render_to_string(&app, 100, 24));
}

/// Command mode shows a `:` input line and the cheat sheet of verbs.
#[test]
fn snapshot_command_mode() {
    let mut app = small_app();
    app.mode = Mode::Command;
    app.command_line = "sort name".to_string();
    insta::assert_snapshot!(render_to_string(&app, 100, 24));
}

/// Shell mode (opened with `!`) collapses the sheet to just the shell row.
#[test]
fn snapshot_command_mode_shell() {
    let mut app = small_app();
    app.mode = Mode::Command;
    app.command_line = "!git checkout main".to_string();
    insta::assert_snapshot!(render_to_string(&app, 100, 24));
}

/// The full drill-in renders its sections (commits/changes/branches).
#[test]
fn snapshot_detail() {
    use crate::app::DetailView;
    let mut app = small_app();
    app.mode = Mode::Detail;
    let id = app.repos[0].id.clone();
    let mut dv = DetailView::new(id, "payments".to_string());
    dv.detail = Some(cohors_core::demo::detail(NOW));
    app.detail = Some(dv);
    insta::assert_snapshot!(render_to_string(&app, 100, 28));
}

/// The drill-in with GitHub data: the PR + contributor sections populated.
#[test]
fn snapshot_detail_with_remote() {
    use crate::app::DetailView;
    use cohors_core::{Contributor, PullRequest, RemoteDetail};
    let mut app = small_app();
    app.mode = Mode::Detail;
    let id = app.repos[0].id.clone();
    let mut dv = DetailView::new(id, "payments".to_string());
    dv.detail = Some(cohors_core::demo::detail(NOW));
    dv.remote = Some(RemoteDetail {
        prs: vec![
            PullRequest {
                number: 142,
                title: "fix: retry charge on 5xx".to_string(),
                author: "maya".to_string(),
                draft: false,
                branch: "fix/retry".to_string(),
                url: String::new(),
                requested_reviewers: vec!["sam".to_string()],
            },
            PullRequest {
                number: 147,
                title: "wip: webhook signatures".to_string(),
                author: "sam".to_string(),
                draft: true,
                branch: "spike/webhooks".to_string(),
                url: String::new(),
                requested_reviewers: Vec::new(),
            },
        ],
        contributors: vec![
            Contributor {
                login: "maya".to_string(),
                contributions: 128,
            },
            Contributor {
                login: "sam".to_string(),
                contributions: 64,
            },
        ],
        open_issues: 7,
        latest_release: Some("v1.4.0".to_string()),
        assigned_issues: 2,
        default_branch_protected: Some(true),
        latest_run: Some(CiStatus::Passing),
    });
    app.detail = Some(dv);
    insta::assert_snapshot!(render_to_string(&app, 100, 28));
}

#[test]
fn snapshot_confirm_stash() {
    use crate::app::{ConfirmAction, Pending};
    let mut app = small_app();
    app.selection.insert(RepoId("payments".to_string()));
    app.selection.insert(RepoId("web-app".to_string()));
    app.mode = Mode::Confirm;
    app.confirm = Some(Pending {
        prompt: "Stash changes in 2 repos?".to_string(),
        action: ConfirmAction::BulkStash(vec![
            RepoId("payments".to_string()),
            RepoId("web-app".to_string()),
        ]),
    });
    insta::assert_snapshot!(render_to_string(&app, 100, 24));
}

#[test]
fn snapshot_command_run_running() {
    use crate::app::{CommandRun, RunResult};
    let mut app = small_app();
    app.mode = Mode::CommandRun;
    app.run = Some(CommandRun::new(
        1,
        "git status -s".to_string(),
        vec![
            RunResult {
                id: RepoId("payments".to_string()),
                name: "payments".to_string(),
                state: RunState::Done {
                    code: 0,
                    stdout: " M src/lib.rs\n?? notes.txt\n".to_string(),
                    stderr: String::new(),
                },
            },
            RunResult {
                id: RepoId("web-app".to_string()),
                name: "web-app".to_string(),
                state: RunState::Running,
            },
            RunResult {
                id: RepoId("auth-service".to_string()),
                name: "auth-service".to_string(),
                state: RunState::Running,
            },
        ],
    ));
    insta::assert_snapshot!(render_to_string(&app, 100, 24));
}

#[test]
fn snapshot_command_run_finished() {
    use crate::app::{CommandRun, RunResult};
    let mut app = small_app();
    app.mode = Mode::CommandRun;
    let mut run = CommandRun::new(
        2,
        "cargo test".to_string(),
        vec![
            RunResult {
                id: RepoId("payments".to_string()),
                name: "payments".to_string(),
                state: RunState::Done {
                    code: 0,
                    stdout: "test result: ok. 12 passed\n".to_string(),
                    stderr: String::new(),
                },
            },
            RunResult {
                id: RepoId("web-app".to_string()),
                name: "web-app".to_string(),
                state: RunState::Done {
                    code: 1,
                    stdout: "running 3 tests\n".to_string(),
                    stderr: "test cart::total failed\nerror: 1 test failed".to_string(),
                },
            },
        ],
    );
    run.focus = 1; // the failing repo, so its stderr shows
    app.run = Some(run);
    insta::assert_snapshot!(render_to_string(&app, 100, 24));
}

/// The help overlay: the marks and the whole keymap.
#[test]
fn snapshot_help() {
    let mut app = small_app();
    app.mode = Mode::Help;
    // Tall enough that the whole help fits, so the box collapses to its
    // content (no trailing whitespace) rather than being capped + clipped.
    insta::assert_snapshot!(render_to_string(&app, 120, 50));
}

/// Every key the help overlay promises has to fit on a 45-row terminal too.
#[test]
fn the_help_fits_a_45_row_terminal() {
    let mut app = small_app();
    app.mode = Mode::Help;
    let screen = render_to_string(&app, 120, 45);
    assert!(screen.contains("config:"), "the last help line is cut off");
}

#[test]
fn snapshot_standup() {
    use crate::app::StandupView;
    use cohors_core::{StandupCommit, StandupWindow};
    let mut app = small_app();
    app.mode = Mode::Standup;
    app.standup_window = StandupWindow::Week;
    // payments (most active) sorts first; its commits overflow the pane,
    // so the scrollbar renders.
    let mut commits = Vec::new();
    for i in 0..24 {
        commits.push(StandupCommit {
            repo: "payments".into(),
            short_id: format!("aa{i:05}"),
            summary: format!("feat: payments work item {i}"),
            timestamp: NOW - (i as i64) * 3600,
        });
    }
    for i in 0..5 {
        commits.push(StandupCommit {
            repo: "web-app".into(),
            short_id: format!("bb{i:05}"),
            summary: format!("fix: web-app bug {i}"),
            timestamp: NOW - (i as i64) * 7200,
        });
    }
    let mut view = StandupView::new(commits);
    // Focus the commits with the cursor deep in the list, so the highlight
    // and contextual scroll (the list follows the cursor) are exercised.
    view.commits_focused = true;
    view.commit_cursor = 20;
    app.standup = Some(view);
    insta::assert_snapshot!(render_to_string(&app, 100, 24));
}

/// On a narrow terminal the standup's two panes stack (Repos box on top,
/// commits box below) instead of sitting side-by-side.
#[test]
fn snapshot_standup_narrow_stacked() {
    use crate::app::StandupView;
    use cohors_core::{StandupCommit, StandupWindow};
    let mut app = small_app();
    app.mode = Mode::Standup;
    app.standup_window = StandupWindow::Week;
    let mut commits = Vec::new();
    for i in 0..8 {
        commits.push(StandupCommit {
            repo: "payments".into(),
            short_id: format!("aa{i:05}"),
            summary: format!("feat: payments work item {i}"),
            timestamp: NOW - (i as i64) * 3600,
        });
    }
    for i in 0..3 {
        commits.push(StandupCommit {
            repo: "web-app".into(),
            short_id: format!("bb{i:05}"),
            summary: format!("fix: web-app bug {i}"),
            timestamp: NOW - (i as i64) * 7200,
        });
    }
    app.standup = Some(StandupView::new(commits));
    insta::assert_snapshot!(render_to_string(&app, 50, 24));
}

#[test]
fn snapshot_standup_collecting() {
    let mut app = small_app();
    app.mode = Mode::Standup;
    app.standup = None;
    insta::assert_snapshot!(render_to_string(&app, 100, 24));
}

#[test]
fn snapshot_launchpad() {
    let mut app = small_app();
    app.mode = Mode::Launchpad;
    let (entries, board) = cohors_core::launchpad::demo();
    app.launchpad = Some(crate::app::LaunchpadView::new(entries, board));
    insta::assert_snapshot!(render_to_string(&app, 110, 34));
}

#[test]
fn snapshot_launchpad_loading() {
    let mut app = small_app();
    app.mode = Mode::Launchpad;
    app.launchpad = None;
    insta::assert_snapshot!(render_to_string(&app, 100, 24));
}

/// Enter on a repository in the tasks card opens the launchpad at that
/// repository's block, not at the top.
#[test]
fn the_launchpad_can_open_on_a_repository() {
    let (entries, board) = cohors_core::launchpad::demo();
    let with_tasks: Vec<String> = entries
        .iter()
        .filter(|e| !e.open_tasks().is_empty())
        .map(|e| e.name.clone())
        .collect();
    let view = crate::app::LaunchpadView::new(entries, board);
    let lines: Vec<usize> = with_tasks
        .iter()
        .map(|name| launchpad_line_of(&view, name).expect("a block for every repo with tasks"))
        .collect();
    // Blocks come one after another, further down each time.
    assert!(lines.windows(2).all(|pair| pair[0] < pair[1]), "{lines:?}");
    assert_eq!(launchpad_line_of(&view, "no-such-repository"), None);
}

/// The header leads with the root holding the most repos, and ties (or no
/// matching repos) keep the first configured root.
#[test]
fn header_roots_lead_with_the_busiest_root() {
    let repo_at = |name: &str, path: &str| {
        let mut r = snap(
            name,
            Branch::Named("main".into()),
            None,
            (0, 0, 0),
            0,
            None,
            None,
        );
        r.path = Some(camino::Utf8PathBuf::from(path));
        r
    };

    let mut app = App::new(
        vec!["/srv/projects".to_string(), "/srv/work".to_string()],
        "cfg".to_string(),
    );
    // More repos under the second root → it leads; the other is "+1 more".
    app.repos = vec![
        repo_at("a", "/srv/projects/a"),
        repo_at("b", "/srv/work/b"),
        repo_at("c", "/srv/work/c"),
    ];
    assert_eq!(overview::header_roots(&app), "/srv/work (+1 more)");

    // A tie (here, no repo under either root) keeps the first configured root.
    app.repos = vec![repo_at("x", "/elsewhere/x")];
    assert_eq!(overview::header_roots(&app), "/srv/projects (+1 more)");
}

// ── Panels: the look with colour ─────────────────────────────────────────────

/// The screen with cards as filled panels, which is what a colour terminal
/// gets (the other tests draw line frames so they can measure them).
fn draw_panels(app: &App, w: u16, h: u16) -> Terminal<TestBackend> {
    let theme = Theme::new(app.icons, app.dark, true).with_panels();
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| render_with(f, app, NOW, &theme)).unwrap();
    terminal
}

/// Panels change what cards are made of and nothing else: every word is where
/// the line-framed layout puts it, no frame lines are left, each card's title
/// sits on a band in its own colour, and the focused card's band is the loud one.
#[test]
fn panels_keep_the_layout_and_colour_each_section() {
    use ratatui::style::Color;
    let mut app = fleet_app();
    app.dark = true;
    let panels = draw_panels(&app, 119, 53);
    let lines = draw(&app, 119, 53, true);

    // Same text in the same cells, once the frames are taken out of both.
    let strip = |text: String| -> String {
        text.chars()
            .map(|c| {
                if "╭╮╰╯│─▄▀".contains(c) {
                    ' '
                } else {
                    c
                }
            })
            .collect()
    };
    assert_eq!(strip(buffer_text(&panels)), strip(buffer_text(&lines)));
    assert!(!buffer_text(&panels).contains('│'));

    // The five title bands are five different colours.
    let band = |title: &[&str], word: &str| cell_at(&panels, title, word).bg;
    let bands = [
        band(&["Do next"], "Do next"),
        band(&["Work at risk", "6"], "Work at risk"),
        band(&["Open tasks", "Commits, last"], "Open tasks"),
        band(&["Open tasks", "Commits, last"], "Commits, last"),
        band(&["Repositories", "12"], "Repositories"),
    ];
    for (i, colour) in bands.iter().enumerate() {
        assert!(matches!(colour, Color::Rgb(..)), "band {i} is not filled");
        assert!(!bands[..i].contains(colour), "band {i} repeats a colour");
    }

    // "Do next" has the focus when the dashboard opens: its band is the
    // section's full colour, and moving the focus hands that to another card.
    let purple = Color::Rgb(0xB5, 0x5C, 0xA4);
    assert_eq!(bands[0], purple);
    app.focus = Card::AtRisk;
    let moved = draw_panels(&app, 119, 53);
    assert_ne!(cell_at(&moved, &["Do next"], "Do next").bg, purple);
    assert_eq!(
        cell_at(&moved, &["Work at risk", "6"], "Work at risk").bg,
        Color::Rgb(0xD9, 0x77, 0x6E)
    );
}

/// An overlay is a panel too, with its title on the colour of the section it
/// belongs to (the launchpad is the tasks card's sage).
#[test]
fn an_overlay_is_a_panel_in_its_sections_colour() {
    use ratatui::style::Color;
    let mut app = fleet_app();
    app.dark = true;
    app.mode = Mode::Help;
    let help = draw_panels(&app, 119, 53);
    let text = buffer_text(&help);
    assert!(!text.contains('║') && !text.contains('╔'), "{text}");
    assert_eq!(
        cell_at(&help, &["Help"], "Help").bg,
        Color::Rgb(0xB5, 0x5C, 0xA4)
    );
}

/// A card lists its keys only while it has the focus, because that is the
/// only time they work.
#[test]
fn a_card_lists_its_keys_only_while_it_has_the_focus() {
    let mut app = fleet_app();
    let text = render_to_string(&app, 119, 53);
    assert!(!text.contains("as markdown"));
    assert!(!text.contains("pull   P"));
    // The keys that work everywhere are on the key bar.
    let keys = text.lines().last().unwrap().to_string();
    assert!(keys.contains("standup") && keys.contains("tasks"), "{keys}");

    app.focus = Card::Commits;
    assert!(render_to_string(&app, 119, 53).contains("as markdown"));
    app.focus = Card::Repos;
    assert!(render_to_string(&app, 119, 53).contains("pull   P"));
}

#[test]
fn every_update_state_shows_in_the_header() {
    use cohors_core::update::UpdateState;
    let states = [
        UpdateState::Available {
            version: "0.7.1".into(),
            command: "brew upgrade cohors".into(),
        },
        UpdateState::Installing {
            version: "0.7.1".into(),
        },
        UpdateState::Ready {
            version: "0.7.1".into(),
        },
        UpdateState::Failed {
            version: "0.7.1".into(),
            command: "pip install -U cohors".into(),
            reason: "no matching distribution".into(),
        },
    ];
    for state in states {
        let mut app = small_app();
        app.update = Some(state.clone());
        // Both headers: the card on a tall screen, one line on a short one.
        for (height, header_rows) in [(40, 5), (14, 1)] {
            let text = render_to_string(&app, 170, height);
            let header: String = text.lines().take(header_rows).collect();
            assert!(
                header.contains(&state.notice()),
                "{state:?} not in the header at height {height}:\n{text}"
            );
            // The rest of the dashboard still draws.
            assert!(
                text.contains("Do next"),
                "{state:?} blanked the dashboard at height {height}:\n{text}"
            );
        }
    }
}
