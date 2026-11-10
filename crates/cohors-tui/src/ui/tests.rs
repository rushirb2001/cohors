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

#[test]
fn the_focused_card_has_the_accent_border_and_the_others_do_not() {
    let mut app = fleet_app();
    app.focus = Card::AtRisk;
    let terminal = draw(&app, 120, 45, true);
    assert_eq!(corner_of(&terminal, "Work at risk").fg, SPIDER_PURPLE);
    assert_ne!(corner_of(&terminal, "Do next").fg, SPIDER_PURPLE);
    assert_ne!(corner_of(&terminal, "Repositories").fg, SPIDER_PURPLE);
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
