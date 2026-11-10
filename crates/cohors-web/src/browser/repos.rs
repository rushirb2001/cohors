//! The repository table (shared by the Overview and Repositories pages) and
//! the Repositories page itself: filter box, saved views, sortable columns,
//! multi-select with an action bar. A click on a row opens the drawer.

use std::cmp::Ordering;

use cohors_core::RepoLine;
use cohors_core::attention::STALE_FETCH_SECS;
use leptos::prelude::*;

use super::Ctx;
use super::actions::{self, ids_selector};
use super::ui::{self, Icon};

// ── Views: which repositories the table shows ────────────────────────────────

/// A named subset of the fleet.
#[derive(Clone, PartialEq, Eq)]
pub enum RepoView {
    All,
    AtRisk,
    NotBackedUp,
    HasStash,
    Uncommitted,
    StaleFetch,
    HasTasks,
    Clean,
    /// The exact repositories a "Do next" action named.
    Custom {
        label: String,
        ids: Vec<String>,
    },
}

impl RepoView {
    /// The views pinned in the sidebar.
    pub const SAVED: [RepoView; 5] = [
        RepoView::AtRisk,
        RepoView::NotBackedUp,
        RepoView::HasStash,
        RepoView::Uncommitted,
        RepoView::StaleFetch,
    ];
    /// The quick filter above the Overview's table.
    const QUICK: [RepoView; 4] = [
        RepoView::All,
        RepoView::AtRisk,
        RepoView::HasTasks,
        RepoView::Clean,
    ];
    /// Every named view, for the Repositories page.
    const EVERY: [RepoView; 8] = [
        RepoView::All,
        RepoView::AtRisk,
        RepoView::NotBackedUp,
        RepoView::HasStash,
        RepoView::Uncommitted,
        RepoView::StaleFetch,
        RepoView::HasTasks,
        RepoView::Clean,
    ];

    pub fn label(&self) -> String {
        match self {
            RepoView::All => "All",
            RepoView::AtRisk => "At risk",
            RepoView::NotBackedUp => "Not backed up",
            RepoView::HasStash => "Has stash",
            RepoView::Uncommitted => "Uncommitted",
            RepoView::StaleFetch => "Stale fetch",
            RepoView::HasTasks => "Has tasks",
            RepoView::Clean => "Clean",
            RepoView::Custom { label, .. } => label,
        }
        .to_string()
    }

    /// The view's name in the URL hash.
    pub fn slug(&self) -> &'static str {
        match self {
            RepoView::All => "all",
            RepoView::AtRisk => "at-risk",
            RepoView::NotBackedUp => "not-backed-up",
            RepoView::HasStash => "has-stash",
            RepoView::Uncommitted => "uncommitted",
            RepoView::StaleFetch => "stale-fetch",
            RepoView::HasTasks => "has-tasks",
            RepoView::Clean => "clean",
            RepoView::Custom { .. } => "selection",
        }
    }

    /// The named view for a slug. An empty slug is everything.
    pub fn from_slug(slug: &str) -> Option<RepoView> {
        if slug.is_empty() {
            return Some(RepoView::All);
        }
        RepoView::EVERY.into_iter().find(|v| v.slug() == slug)
    }

    /// Whether `line` belongs to this view. These read the model's own fields
    /// and mirror the rules in `cohors_core::attention::assess`, so each
    /// count matches the figure of the same name.
    pub fn matches(&self, line: &RepoLine, now: i64) -> bool {
        let readable = line.error.is_none();
        match self {
            RepoView::All => true,
            RepoView::AtRisk => line.at_risk,
            // Commits the remote does not have, or a branch never pushed.
            RepoView::NotBackedUp => readable && (line.ahead > 0 || never_pushed(line)),
            RepoView::HasStash => line.stashes > 0,
            RepoView::Uncommitted => line.changed() > 0,
            RepoView::StaleFetch => {
                readable
                    && line.has_remote
                    && line.fetched.is_some_and(|at| now - at > STALE_FETCH_SECS)
            }
            RepoView::HasTasks => line.tasks_open > 0,
            RepoView::Clean => line.reasons.is_empty(),
            RepoView::Custom { ids, .. } => ids.contains(&line.id),
        }
    }
}

/// A named branch that has a remote to go to but no upstream yet. A detached
/// HEAD (`@abc123 (detached)`) or an empty repository (`(no commits)`) has
/// nothing to publish.
fn never_pushed(line: &RepoLine) -> bool {
    let named = !line.branch.starts_with('@') && !line.branch.starts_with('(');
    line.has_remote && line.upstream.is_none() && named
}

// ── Sorting ──────────────────────────────────────────────────────────────────

/// A sortable column.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    LastCommit,
    Upstream,
    Changes,
    Stash,
    Tasks,
}

/// The column the table is sorted by, and which way.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub key: SortKey,
    pub descending: bool,
}

impl SortKey {
    /// The direction a first click sorts in: names A to Z, everything else
    /// largest or newest first.
    fn starts_descending(self) -> bool {
        self != SortKey::Name
    }

    fn compare(self, a: &RepoLine, b: &RepoLine) -> Ordering {
        match self {
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::LastCommit => commit_time(a).cmp(&commit_time(b)),
            SortKey::Upstream => sync_rank(a).cmp(&sync_rank(b)),
            SortKey::Changes => a.changed().cmp(&b.changed()),
            SortKey::Stash => a.stashes.cmp(&b.stashes),
            SortKey::Tasks => a.tasks_open.cmp(&b.tasks_open),
        }
    }
}

fn commit_time(line: &RepoLine) -> i64 {
    line.commit.as_ref().map_or(0, |c| c.timestamp)
}

/// How far a repository is from its remote, for sorting: unreadable first,
/// then never pushed, then by commits ahead and behind, then level, then
/// repositories with no remote at all.
fn sync_rank(line: &RepoLine) -> (u8, u32) {
    if line.error.is_some() {
        (4, 0)
    } else if !line.has_remote {
        (0, 0)
    } else if line.upstream.is_none() {
        (3, 0)
    } else if line.ahead + line.behind > 0 {
        (2, line.ahead + line.behind)
    } else {
        (1, 0)
    }
}

/// Click a column header: sort by it, then reverse, then back to the model's
/// own order (most urgent first).
fn cycle_sort(cx: Ctx, key: SortKey) {
    let next = match cx.sort.get_untracked() {
        Some(sort) if sort.key == key && sort.descending == key.starts_descending() => Some(Sort {
            key,
            descending: !sort.descending,
        }),
        Some(sort) if sort.key == key => None,
        _ => Some(Sort {
            key,
            descending: key.starts_descending(),
        }),
    };
    cx.sort.set(next);
}

// ── Filtering ────────────────────────────────────────────────────────────────

/// Whether `line` contains every word typed into the filter box, in its
/// name, branch, upstream, path, groups or reasons.
fn matches_text(line: &RepoLine, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let mut haystack = format!("{} {}", line.name, line.branch).to_lowercase();
    for part in line
        .upstream
        .iter()
        .chain(line.path.iter())
        .chain(line.groups.iter())
        .chain(line.reasons.iter())
    {
        haystack.push(' ');
        haystack.push_str(&part.to_lowercase());
    }
    words.iter().all(|word| haystack.contains(word.as_str()))
}

/// The lines the table shows: the view, then the text filter, then the sort.
fn visible_lines(cx: Ctx) -> Vec<RepoLine> {
    let view = cx.view.get();
    let words: Vec<String> = cx
        .filter
        .get()
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let mut lines: Vec<RepoLine> = cx.overview.with(|o| {
        o.repos
            .iter()
            .filter(|line| view.matches(line, o.now) && matches_text(line, &words))
            .cloned()
            .collect()
    });
    // A stable sort, so ties keep the model's order.
    if let Some(sort) = cx.sort.get() {
        lines.sort_by(|a, b| {
            let order = sort.key.compare(a, b);
            if sort.descending {
                order.reverse()
            } else {
                order
            }
        });
    }
    lines
}

// ── The table ────────────────────────────────────────────────────────────────

/// The repository table with its tools. `full` is the Repositories page: every
/// saved view with its count, and the view kept in the URL. Otherwise it is
/// the Overview's shorter quick filter.
pub fn repo_table(cx: Ctx, full: bool) -> impl IntoView {
    let lines = Memo::new(move |_| visible_lines(cx));

    let views = if full {
        RepoView::EVERY.to_vec()
    } else {
        RepoView::QUICK.to_vec()
    };
    let segments = views
        .into_iter()
        .map(|view| {
            let pressed = view.clone();
            let counted = view.clone();
            let opened = view.clone();
            let count = move || {
                cx.overview
                    .with(|o| o.repos.iter().filter(|r| counted.matches(r, o.now)).count())
            };
            view! {
                <button
                    aria-pressed=move || {
                        if cx.view.with(|v| *v == pressed) { "true" } else { "false" }
                    }
                    on:click=move |_| {
                        if full {
                            cx.open_view(opened.clone());
                        } else {
                            cx.view.set(opened.clone());
                        }
                    }
                >
                    <span>{view.label()}</span>
                    <span class="count">{count}</span>
                </button>
            }
        })
        .collect::<Vec<_>>();

    // A "Do next" list is not one of the named views: show it as a chip that
    // can be dismissed.
    let custom = move || {
        let RepoView::Custom { label, .. } = cx.view.get() else {
            return None;
        };
        Some(view! {
            <span class="chip">
                <span>{label}</span>
                <button
                    class="icon-btn"
                    aria-label="Show all repositories"
                    on:click=move |_| {
                        if full {
                            cx.open_view(RepoView::All);
                        } else {
                            cx.view.set(RepoView::All);
                        }
                    }
                >
                    {ui::icon(Icon::Close)}
                </button>
            </span>
        })
    };

    let showing = move || {
        let shown = lines.with(Vec::len);
        let total = cx.overview.with(|o| o.figures.repos);
        if shown == total {
            ui::plural(total, "repository", "repositories")
        } else {
            format!("{shown} of {total} repositories")
        }
    };
    let order = move || match cx.sort.get() {
        None => view! { <span class="count">"Most urgent first"</span> }.into_any(),
        Some(_) => view! {
            <button class="btn small quiet" on:click=move |_| cx.sort.set(None)>
                "Back to most urgent first"
            </button>
        }
        .into_any(),
    };

    let all_ticked = move || {
        let marked = cx.marked.get();
        lines.with(|l| !l.is_empty() && l.iter().all(|line| marked.contains(&line.id)))
    };
    let toggle_all = move |_| {
        let ids: Vec<String> = lines.with_untracked(|l| l.iter().map(|x| x.id.clone()).collect());
        cx.marked.update(|marked| {
            if ids.iter().all(|id| marked.contains(id)) {
                for id in &ids {
                    marked.remove(id);
                }
            } else {
                marked.extend(ids);
            }
        });
    };

    let body = move || {
        let now = cx.overview.with(|o| o.now);
        let rows = lines.get();
        if rows.is_empty() {
            return view! {
                <tr>
                    <td colspan="8" class="wrap">
                        <div class="empty">
                            "No repository matches this view and filter."
                        </div>
                    </td>
                </tr>
            }
            .into_any();
        }
        rows.into_iter()
            .map(|line| repo_row(cx, line, now))
            .collect::<Vec<_>>()
            .into_any()
    };

    view! {
        <div class="tools">
            <input
                class="search"
                type="search"
                placeholder="Filter by name, branch, path or reason"
                aria-label="Filter repositories"
                prop:value=move || cx.filter.get()
                on:input=move |ev| cx.filter.set(event_target_value(&ev))
            />
            {custom}
            <span class="count">{showing}</span>
            <span class="grow"></span>
            {order}
        </div>
        <div class="tools">
            <div class="seg" role="group" aria-label="Views">
                {segments}
            </div>
        </div>
        {selection_bar(cx)}
        <div class="block">
            <table class="repos">
                <colgroup>
                    <col style="width:34px" />
                    <col style="width:20%" />
                    <col />
                    <col style="width:134px" />
                    <col style="width:184px" />
                    <col style="width:62px" />
                    <col style="width:62px" />
                    <col class="hide-md" style="width:96px" />
                </colgroup>
                <thead>
                    <tr>
                        <th class="check">
                            <input
                                type="checkbox"
                                aria-label="Select every repository shown"
                                prop:checked=all_ticked
                                on:change=toggle_all
                            />
                        </th>
                        {sort_header(cx, "Repository", SortKey::Name, false)}
                        {sort_header(cx, "Last commit", SortKey::LastCommit, false)}
                        {sort_header(cx, "Upstream", SortKey::Upstream, false)}
                        {sort_header(cx, "Working tree", SortKey::Changes, false)}
                        {sort_header(cx, "Stash", SortKey::Stash, true)}
                        {sort_header(cx, "Tasks", SortKey::Tasks, true)}
                        <th class="hide-md">"12 weeks"</th>
                    </tr>
                </thead>
                <tbody>{body}</tbody>
            </table>
        </div>
    }
}

/// A column header that sorts the table when clicked.
fn sort_header(cx: Ctx, label: &'static str, key: SortKey, right: bool) -> impl IntoView {
    let current = move || cx.sort.get().filter(|s| s.key == key);
    view! {
        <th
            class:r=right
            aria-sort=move || {
                current().map(|s| if s.descending { "descending" } else { "ascending" })
            }
        >
            <button
                class="sort"
                class:on=move || current().is_some()
                class:desc=move || current().is_some_and(|s| s.descending)
                title="Sort by this column"
                on:click=move |_| cycle_sort(cx, key)
            >
                <span>{label}</span>
                {ui::icon(Icon::Arrow)}
            </button>
        </th>
    }
}

/// The bar of actions for the ticked repositories. One button per registry
/// verb (`cohors_web::ACTION_VERBS`, kept in step with the registry by a
/// server test).
fn selection_bar(cx: Ctx) -> impl IntoView {
    move || {
        let count = cx.marked.with(|m| m.len());
        (count > 0).then(|| {
            let buttons = cohors_web::ACTION_VERBS
                .iter()
                .map(|&verb| {
                    actions::verb_button(cx, verb, ui::capitalise(verb), "small", None, move || {
                        ids_selector(&marked_ids(cx))
                    })
                })
                .collect::<Vec<_>>();
            view! {
                <div class="selbar" role="region" aria-label="Actions for the selected repositories">
                    <b>{format!("{count} selected")}</b>
                    {buttons}
                    {actions::read_only_note(cx)}
                    <span class="grow"></span>
                    <button
                        class="btn small quiet"
                        on:click=move |_| cx.marked.update(|m| m.clear())
                    >
                        "Clear selection"
                    </button>
                </div>
            }
        })
    }
}

/// The ticked repository ids, sorted so the request is the same every time.
pub fn marked_ids(cx: Ctx) -> Vec<String> {
    let mut ids: Vec<String> = cx.marked.get_untracked().into_iter().collect();
    ids.sort();
    ids
}

/// One repository: two lines per cell, the main fact above its qualifier.
fn repo_row(cx: Ctx, line: RepoLine, now: i64) -> impl IntoView {
    let id = line.id.clone();
    let is_open = {
        let id = id.clone();
        move || cx.selected.with(|s| s.as_deref() == Some(id.as_str()))
    };
    let open = {
        let id = id.clone();
        move || cx.selected.set(Some(id.clone()))
    };
    let open_on_enter = {
        let open = open.clone();
        move |ev: leptos::ev::KeyboardEvent| {
            if ev.key() == "Enter" {
                open();
            }
        }
    };
    let ticked = {
        let id = id.clone();
        move || cx.marked.with(|m| m.contains(&id))
    };
    let tick = {
        let id = id.clone();
        move |ev| {
            let on = event_target_checked(&ev);
            cx.marked.update(|m| {
                if on {
                    m.insert(id.clone());
                } else {
                    m.remove(&id);
                }
            });
        }
    };

    let commit = match &line.commit {
        Some(c) => view! {
            <div class="clip" title=c.summary.clone()>{c.summary.clone()}</div>
            <div class="sub clip">
                {ui::icon(Icon::Commit)}
                {format!("{}, {}", c.short_id, ui::ago(c.timestamp, now))}
            </div>
        }
        .into_any(),
        None => view! {
            <div class="dim">"No commits"</div>
            <div class="sub">"\u{a0}"</div>
        }
        .into_any(),
    };

    let sync = sync_summary(&line, now);
    let tree = tree_summary(&line);
    let tasks = if line.configured {
        big_number(line.tasks_open as u32).into_any()
    } else {
        // Not zero tasks: there is no launchpad file to hold any.
        view! { <span class="big zero" title="No launchpad file">"–"</span> }.into_any()
    };

    view! {
        <tr
            class="open-row"
            class:sel=is_open
            tabindex="0"
            on:click=move |_| open()
            on:keydown=open_on_enter
        >
            <td class="check">
                <input
                    type="checkbox"
                    aria-label=format!("Select {}", line.name)
                    prop:checked=ticked
                    // A tick must not also open the drawer.
                    on:click=|ev| ev.stop_propagation()
                    on:change=tick
                />
            </td>
            <td>
                <div class="clip" title=line.name.clone()>
                    {ui::severity_mark(line.severity)}
                    <span class="name">{line.name.clone()}</span>
                </div>
                <div class="sub clip" title=line.branch.clone()>
                    {ui::icon(Icon::Branch)}
                    {line.branch.clone()}
                </div>
            </td>
            <td>{commit}</td>
            <td>
                <div class=format!("clip {}", sync.class)>{sync.main}</div>
                <div class="sub clip">{sync.sub}</div>
            </td>
            <td>
                <div class=format!("clip {}", tree.class)>{tree.main}</div>
                <div class="sub clip">{tree.sub}</div>
            </td>
            <td class="r">{big_number(line.stashes)}</td>
            <td class="r">{tasks}</td>
            <td class="hide-md">{mini_bars(&line.activity)}</td>
        </tr>
    }
}

/// A count worth reading at a glance; zero is set back.
fn big_number(value: u32) -> impl IntoView {
    let class = if value == 0 { "big zero" } else { "big" };
    view! { <span class=class>{value}</span> }
}

/// Two lines of text for a cell, and the class that tints the first.
pub struct Summary {
    pub main: String,
    pub sub: String,
    pub class: &'static str,
}

/// How the branch stands against its upstream, said truthfully: a repository
/// with no remote says so, and so does a branch with no upstream.
pub fn sync_summary(line: &RepoLine, now: i64) -> Summary {
    let fetched = match line.fetched {
        Some(at) => format!("fetched {}", ui::ago(at, now)),
        None => "never fetched".to_string(),
    };
    if let Some(error) = &line.error {
        return Summary {
            main: "Unreadable".to_string(),
            sub: error.clone(),
            class: "bad",
        };
    }
    if !line.has_remote {
        return Summary {
            main: "No remote".to_string(),
            sub: "nowhere to push".to_string(),
            class: "",
        };
    }
    if line.upstream.is_none() {
        let (sub, class) = if never_pushed(line) {
            ("never pushed", "bad")
        } else {
            ("nothing to push", "")
        };
        return Summary {
            main: "No upstream".to_string(),
            sub: sub.to_string(),
            class,
        };
    }
    let (main, class) = match (line.ahead, line.behind) {
        (0, 0) => ("Level".to_string(), ""),
        (ahead, 0) => (format!("{ahead} ahead"), "caution"),
        (0, behind) => (format!("{behind} behind"), ""),
        (ahead, behind) => (format!("{ahead} ahead, {behind} behind"), "caution"),
    };
    Summary {
        main,
        sub: fetched,
        class,
    }
}

/// The working tree in two lines: how many files, then what kind.
pub fn tree_summary(line: &RepoLine) -> Summary {
    let changed = line.changed();
    if changed == 0 {
        return Summary {
            main: "Clean".to_string(),
            sub: "\u{a0}".to_string(),
            class: "dim",
        };
    }
    let mut kinds = Vec::new();
    for (count, word) in [
        (line.conflicted, "conflicted"),
        (line.staged, "staged"),
        (line.modified, "mod"),
        (line.untracked, "new"),
    ] {
        if count > 0 {
            kinds.push(format!("{count} {word}"));
        }
    }
    Summary {
        main: ui::plural(changed as usize, "file", "files"),
        sub: kinds.join(", "),
        class: "",
    }
}

/// Commits per week as a row of small bars, oldest week first.
pub fn mini_bars(weeks: &[u8]) -> impl IntoView + use<> {
    let most = weeks.iter().copied().max().unwrap_or(0).max(1) as f32;
    let total: u32 = weeks.iter().map(|&w| u32::from(w)).sum();
    let tip = format!(
        "{} in {}",
        ui::plural(total as usize, "commit", "commits"),
        ui::plural(weeks.len(), "week", "weeks")
    );
    let bars = weeks
        .iter()
        .map(|&week| {
            // A quiet week keeps a thin baseline so the row reads as a chart.
            let height = if week == 0 {
                6.0
            } else {
                (f32::from(week) / most * 100.0).max(12.0)
            };
            view! { <i style=format!("height:{height:.0}%")></i> }
        })
        .collect::<Vec<_>>();
    view! {
        <span class="mini" role="img" aria-label=tip.clone() title=tip>
            {bars}
        </span>
    }
}

// ── The page ─────────────────────────────────────────────────────────────────

pub fn page(cx: Ctx) -> impl IntoView {
    view! {
        <section class="head">
            <div>
                <h1>"Repositories"</h1>
                <p>{move || cx.overview.with(|o| o.headline.clone())}</p>
            </div>
            <div class="actions">
                {actions::verb_button(
                    cx,
                    "fetch",
                    "Fetch all".to_string(),
                    "",
                    Some(Icon::Refresh),
                    actions::all_selector,
                )}
            </div>
        </section>
        <section>{repo_table(cx, true)}</section>
    }
}
