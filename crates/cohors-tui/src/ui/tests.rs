use super::*;
use crate::app::{App, Mode};
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

fn demo_app() -> App {
    let mut app = App::new(
        vec!["~/projects".to_string(), "~/work".to_string()],
        "~/.config/cohors/config.toml".to_string(),
    );
    // A few repos carry GitHub remote info so the Remote cloud is exercised
    // in passing/failing/pending states; the rest stay local ("—").
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
    // Per-branch CI: `main` passes, the feature branch fails — each branch row
    // shows its own checks, since CI is keyed by ref, not the worktree.
    payments.remote.as_mut().unwrap().ci_by_branch = std::collections::HashMap::from([
        ("main".to_string(), CiStatus::Passing),
        ("feat/checkout-rework".to_string(), CiStatus::Failing),
    ]);
    // A multi-branch repo: the dashboard lists each branch, with the open-PR count
    // attached to the branch the PRs target (both target `main`).
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
    app
}

fn render_to_string(app: &App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| render(f, app, NOW)).unwrap();
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
    // Normalize the version so a bump doesn't churn every header snapshot —
    // these tests assert layout, not the version number.
    out.replace(&format!("v{}", env!("CARGO_PKG_VERSION")), "vX.Y.Z")
}

#[test]
fn snapshot_list() {
    let app = demo_app();
    // Tall enough to show the whole fleet, including the multi-branch `payments`
    // repo expanded into its per-branch lines.
    insta::assert_snapshot!(render_to_string(&app, 100, 28));
}

/// The medium width tier: too narrow for the Activity sparkline, but the modern
/// split Sync / PRs / CI columns survive (it does NOT cliff to the fused set).
#[test]
fn snapshot_list_medium() {
    let app = demo_app();
    insta::assert_snapshot!(render_to_string(&app, 88, 32));
}

/// The narrow tier: no room for PRs or the CI word, but CI survives as a
/// colour-coded square in its own column (never fused away).
#[test]
fn snapshot_list_narrow() {
    let app = demo_app();
    insta::assert_snapshot!(render_to_string(&app, 72, 32));
}

/// On a narrow terminal the header collapses to the spider + lede + the
/// directory only — no taglines, divider, or info column.
#[test]
fn snapshot_header_compact() {
    let app = demo_app();
    insta::assert_snapshot!(render_to_string(&app, 60, 16));
}

/// With `h` pressed, the hint boxes collapse to just the toggle divider,
/// which now reads "unhide".
#[test]
fn snapshot_hints_hidden() {
    let mut app = demo_app();
    app.hints_hidden = true;
    insta::assert_snapshot!(render_to_string(&app, 100, 20));
}

/// Action/scan feedback shows as a self-dismissing toast (bottom-right, above
/// the footer), not in the header.
#[test]
fn snapshot_toast() {
    let mut app = demo_app();
    app.status = Some("pushed 3 repos".to_string());
    insta::assert_snapshot!(render_to_string(&app, 100, 20));
}

/// The "Open with…" picker lists detected openers with the default marked.
#[test]
fn snapshot_open_with() {
    use crate::app::{OpenWith, Opener};
    let mut app = demo_app();
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
    insta::assert_snapshot!(render_to_string(&app, 100, 20));
}

/// When PATH has no editor CLI, the picker shows a note explaining why and
/// still offers Reveal / lazygit.
#[test]
fn snapshot_open_with_no_editor() {
    use crate::app::{OpenWith, Opener};
    let mut app = demo_app();
    app.mode = Mode::OpenWith;
    let openers = vec![Opener::Reveal, Opener::Lazygit];
    app.open_with = Some(OpenWith::new(openers, None));
    insta::assert_snapshot!(render_to_string(&app, 100, 20));
}

/// The "push which branch?" picker lists a repo's local branches (current marked
/// and pre-selected) with each branch's push state — reached from `P` on a
/// multi-branch repo.
#[test]
fn snapshot_push_branch() {
    use crate::app::PushPick;
    let mut app = demo_app();
    app.mode = Mode::PushBranch;
    let payments = app.repos.iter().find(|r| r.name == "payments").unwrap();
    app.push_pick = Some(PushPick {
        id: payments.id.clone(),
        name: payments.name.clone(),
        branches: payments.branches.clone(),
        cursor: 0,
    });
    insta::assert_snapshot!(render_to_string(&app, 100, 20));
}

/// On a compact terminal the footer's group boxes collapse to single columns
/// (so the keys never clip) instead of falling back to a different layout.
/// Even at an ultra-narrow width the stacked footer shows every hint (one per
/// line if it must) — no lossy essentials fallback.
#[test]
fn snapshot_footer_ultra_narrow() {
    let app = demo_app();
    insta::assert_snapshot!(render_to_string(&app, 30, 30));
}

#[test]
fn snapshot_footer_compact() {
    let app = demo_app();
    insta::assert_snapshot!(render_to_string(&app, 56, 22));
}

/// On a tall terminal the docked context pane appears below the list and
/// shows the selected repo at a glance: why it wants attention, its
/// changes/sync, and the last commit.
#[test]
fn snapshot_dock_idle() {
    let mut app = demo_app();
    app.selected = 1; // skip the unreadable repo that sorts to the top
    insta::assert_snapshot!(render_to_string(&app, 120, 34));
}

/// ASCII icon mode (also the `NO_COLOR` fallback): the synced state renders as
/// a steady "ok" word instead of a colour-only, blinking `●`, and a marked repo
/// uses a `*` gutter instead of `●` — the multi-user portability path.
#[test]
fn snapshot_ascii_synced() {
    let mut app = App::new(vec!["(demo)".to_string()], "(demo)".to_string());
    app.icons = cohors_config::IconMode::Ascii;
    app.set_repos(vec![snap(
        "in-sync",
        Branch::Named("main".to_string()),
        Some(("origin/main", 0, 0)),
        (0, 0, 0),
        0,
        Some((NOW - 3600, "chore: nothing to do")),
        None,
    )]);
    app.selection.insert(app.repos[0].id.clone()); // marked → `*` gutter
    insta::assert_snapshot!(render_to_string(&app, 100, 34));
}

/// While a bulk action runs, the dock shows the in-flight repos with spinners
/// instead of the cursor's repo.
#[test]
fn snapshot_dock_action() {
    let mut app = demo_app();
    app.busy.insert(app.repos[1].id.clone());
    app.busy.insert(app.repos[2].id.clone());
    insta::assert_snapshot!(render_to_string(&app, 100, 34));
}

/// A long commit message wraps across the dock's remaining lines instead of
/// being truncated with an ellipsis.
#[test]
fn snapshot_dock_wraps_long_commit() {
    let mut app = App::new(vec!["(demo)".to_string()], "(demo)".to_string());
    app.set_repos(vec![snap(
        "load-tester",
        Branch::Named("main".to_string()),
        Some(("origin/main", 1, 0)),
        (0, 0, 0),
        0,
        Some((
            NOW - 7 * 86_400,
            "chore: add load-test harnesses (mock origin, HTTP load driver, quota-RPC pgbench) + ignore results and refresh the CI perf matrix",
        )),
        None,
    )]);
    insta::assert_snapshot!(render_to_string(&app, 100, 34));
}

/// On a window too short for the whole fleet, the list shows a "… N more ↓"
/// affordance on its bottom border.
#[test]
fn snapshot_repos_scroll_affordance() {
    let mut app = App::new(vec!["(demo)".to_string()], "(demo)".to_string());
    app.set_repos(cohors_core::demo::fleet(NOW));
    app.hints_hidden = true; // give the list room so some rows show and some overflow
    // Short height so the 12-repo fleet overflows the viewport.
    insta::assert_snapshot!(render_to_string(&app, 100, 18));
}

/// Scrolled to the end of the list, the "more" hint flips to the top
/// (`↑ N more`), since the hidden repos are now above.
#[test]
fn snapshot_repos_scroll_at_bottom() {
    let mut app = App::new(vec!["(demo)".to_string()], "(demo)".to_string());
    app.set_repos(cohors_core::demo::fleet(NOW));
    app.hints_hidden = true;
    app.selected = app.visible_len() - 1; // cursor at the last repo → scrolled down
    insta::assert_snapshot!(render_to_string(&app, 100, 18));
}

/// Scrolled to the middle, both hints show at once — `↑ N more` below the
/// header and `… N more ↓` at the bottom.
#[test]
fn snapshot_repos_scroll_middle() {
    let mut app = App::new(vec!["(demo)".to_string()], "(demo)".to_string());
    app.set_repos(cohors_core::demo::fleet(NOW));
    app.hints_hidden = true;
    // Far enough down that some repos are hidden above *and* below.
    app.selected = app.visible_len().saturating_sub(4);
    insta::assert_snapshot!(render_to_string(&app, 100, 18));
}

/// Command mode shows a `:` input line in the top strip.
#[test]
fn snapshot_command_mode() {
    let mut app = demo_app();
    app.mode = Mode::Command;
    app.command_line = "sort name".to_string();
    insta::assert_snapshot!(render_to_string(&app, 100, 22));
}

/// Shell mode (opened with `!`) collapses the sheet to just the shell row.
#[test]
fn snapshot_command_mode_shell() {
    let mut app = demo_app();
    app.mode = Mode::Command;
    app.command_line = "!git checkout main".to_string();
    insta::assert_snapshot!(render_to_string(&app, 100, 22));
}

/// The drill-in detail pane renders its sections (commits/changes/branches).
#[test]
fn snapshot_detail() {
    use crate::app::DetailView;
    let mut app = demo_app();
    app.mode = Mode::Detail;
    let id = app.repos[0].id.clone();
    let mut dv = DetailView::new(id, "payments".to_string());
    dv.detail = Some(cohors_core::demo::detail(NOW));
    app.detail = Some(dv);
    insta::assert_snapshot!(render_to_string(&app, 100, 28));
}

/// Detail with GitHub data: the PR + contributor sections populated.
#[test]
fn snapshot_detail_with_remote() {
    use crate::app::DetailView;
    use cohors_core::{Contributor, PullRequest, RemoteDetail};
    let mut app = demo_app();
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

/// The `cohors demo` fleet renders end-to-end (validates the demo data path).
#[test]
fn snapshot_demo_fleet() {
    let mut app = App::new(vec!["(demo)".to_string()], "(demo)".to_string());
    app.set_repos(cohors_core::demo::fleet(NOW));
    insta::assert_snapshot!(render_to_string(&app, 100, 26));
}

#[test]
fn snapshot_multiselect() {
    let mut app = demo_app();
    app.selection.insert(RepoId("payments".to_string()));
    app.selection.insert(RepoId("web-app".to_string()));
    insta::assert_snapshot!(render_to_string(&app, 100, 20));
}

#[test]
fn snapshot_confirm_stash() {
    use crate::app::{ConfirmAction, Pending};
    let mut app = demo_app();
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
    insta::assert_snapshot!(render_to_string(&app, 100, 20));
}

#[test]
fn snapshot_command_run_running() {
    use crate::app::{CommandRun, RunResult};
    let mut app = demo_app();
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
    insta::assert_snapshot!(render_to_string(&app, 100, 22));
}

#[test]
fn snapshot_command_run_finished() {
    use crate::app::{CommandRun, RunResult};
    let mut app = demo_app();
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
    insta::assert_snapshot!(render_to_string(&app, 100, 22));
}

#[test]
fn snapshot_dirty_only() {
    let mut app = demo_app();
    app.dirty_only = true;
    insta::assert_snapshot!(render_to_string(&app, 100, 20));
}

#[test]
fn snapshot_filtered() {
    let mut app = demo_app();
    app.mode = Mode::Filter;
    app.filter = "pay".to_string();
    insta::assert_snapshot!(render_to_string(&app, 100, 24));
}

/// A just-opened filter (empty query): the boxed input inside the repos panel
/// shows its muted placeholder above the column headers.
#[test]
fn snapshot_filter_empty() {
    let mut app = demo_app();
    app.mode = Mode::Filter;
    insta::assert_snapshot!(render_to_string(&app, 100, 24));
}

/// The footer hint bar washes its *inner* cells with the faint brand-purple card
/// colour, but never the box outline (styles aren't captured by the symbol
/// snapshots, so assert both directly).
#[test]
fn footer_hint_bar_wash_stays_inside_the_outline() {
    let app = demo_app();
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| render(f, &app, NOW)).unwrap();
    let buf = terminal.backend().buffer();
    let wash = Color::Rgb(0xFA, 0xF5, 0xFF);

    // The inner hints carry the wash...
    let has_wash = (0..buf.area.height)
        .any(|y| (0..buf.area.width).any(|x| buf.cell((x, y)).is_some_and(|c| c.bg == wash)));
    assert!(has_wash, "footer hints should carry the purple wash");

    // ...but the box outline stays clean — the footer spans the full width, so its
    // bottom-left corner (last row, column 0) is a border cell that must not wash.
    let corner = buf.cell((0, buf.area.height - 1)).unwrap();
    assert_ne!(corner.bg, wash, "the wash must not bleed onto the outline");
}

/// In a dark terminal the cards flip to a dark wash (so the native accent colours
/// read), never the light one.
#[test]
fn footer_hint_bar_uses_the_dark_wash_in_dark_mode() {
    let mut app = demo_app();
    app.dark = true;
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| render(f, &app, NOW)).unwrap();
    let buf = terminal.backend().buffer();
    let dark = Color::Rgb(0x2E, 0x27, 0x3E);
    let light = Color::Rgb(0xFA, 0xF5, 0xFF);
    let any_bg = |want: Color| {
        (0..buf.area.height)
            .any(|y| (0..buf.area.width).any(|x| buf.cell((x, y)).is_some_and(|c| c.bg == want)))
    };
    assert!(any_bg(dark), "dark mode should use the dark purple wash");
    assert!(!any_bg(light), "dark mode must not use the light wash");
}

#[test]
fn snapshot_help() {
    let mut app = demo_app();
    app.mode = Mode::Help;
    // Tall enough that the whole help fits, so the box collapses to its
    // content (no trailing whitespace) rather than being capped + clipped.
    insta::assert_snapshot!(render_to_string(&app, 100, 64));
}

#[test]
fn snapshot_standup() {
    use crate::app::StandupView;
    use cohors_core::{StandupCommit, StandupWindow};
    let mut app = demo_app();
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
    insta::assert_snapshot!(render_to_string(&app, 100, 22));
}

/// On a narrow terminal the standup's two panes stack (Repos box on top,
/// commits box below) instead of sitting side-by-side.
#[test]
fn snapshot_standup_narrow_stacked() {
    use crate::app::StandupView;
    use cohors_core::{StandupCommit, StandupWindow};
    let mut app = demo_app();
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
    let mut app = demo_app();
    app.mode = Mode::Standup;
    app.standup = None;
    insta::assert_snapshot!(render_to_string(&app, 100, 18));
}

#[test]
fn snapshot_empty() {
    let app = App::new(vec!["~/projects".to_string()], "cfg".to_string());
    insta::assert_snapshot!(render_to_string(&app, 100, 30));
}

/// Empty fleet, but repos detected elsewhere: the first-run rescue prompt.
#[test]
fn snapshot_empty_picker() {
    let mut app = App::new(vec!["~/projects".to_string()], "cfg".to_string());
    app.suggested_roots = vec!["~/code".to_string(), "~/work".to_string()];
    insta::assert_snapshot!(render_to_string(&app, 100, 30));
}

#[test]
fn snapshot_loading() {
    let mut app = App::new(vec!["~/projects".to_string()], "cfg".to_string());
    app.scanning = true;
    insta::assert_snapshot!(render_to_string(&app, 100, 12));
}

/// The header leads with the root holding the most repos, and ties (or no
/// matching repos) keep the first configured root.
#[test]
fn header_dir_leads_with_the_busiest_root() {
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
    // More repos under the second root → it leads; the other counts as "(+1)".
    app.repos = vec![
        repo_at("a", "/srv/projects/a"),
        repo_at("b", "/srv/work/b"),
        repo_at("c", "/srv/work/c"),
    ];
    assert_eq!(header_dir(&app), "/srv/work (+1)");

    // A tie (here, no repo under either root) keeps the first configured root.
    app.repos = vec![repo_at("x", "/elsewhere/x")];
    assert_eq!(header_dir(&app), "/srv/projects (+1)");
}
