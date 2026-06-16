//! cohors web — the fleet dashboard in the browser, driven by the pure
//! `cohors-core` crate compiled to WebAssembly.
//!
//! v0.5: slice 1 rendered the built-in demo fleet; slice 2 adds *real GitHub
//! data* — paste a personal-access token and it fetches your repositories over
//! the browser's `fetch` (see [`github`]) and renders them with the exact same
//! `compute_view` / `assess` logic the TUI runs (ADR-002). The demo fleet stays
//! as the zero-setup fallback. Proper OAuth (no pasted token) is slice 3.

mod github;

use std::sync::Arc;

use cohors_core::{
    Branch, CiStatus, FleetSummary, RepoSnapshot, Severity, SortMode, ViewParams, assess,
    compute_view, demo, fleet_summary, group_commits, time,
};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

/// Fixed clock for the *demo* fleet, so its relative ages stay sensible.
const DEMO_NOW: i64 = 1_700_000_000;

/// The browser's real wall clock (Unix seconds) — used for live GitHub data.
fn real_now() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

/// What the dashboard is currently showing.
#[derive(Clone)]
enum Fleet {
    Demo(Arc<Vec<RepoSnapshot>>),
    Loading,
    Loaded(Arc<Vec<RepoSnapshot>>),
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let fleet = RwSignal::new(Fleet::Loading);
    let notice = RwSignal::new(None::<String>);

    // Control signals live here so they persist as the fleet changes.
    let filter = RwSignal::new(String::new());
    let sort = RwSignal::new(SortMode::DirtyFirst);
    let dirty_only = RwSignal::new(false);
    let selected = RwSignal::new(None::<String>);

    // Load real repos through the local proxy (which injects the machine's GitHub
    // token — no token in the browser). On any failure, fall back to the demo
    // fleet with a short note explaining why.
    let load = move || {
        notice.set(None);
        selected.set(None);
        fleet.set(Fleet::Loading);
        spawn_local(async move {
            match github::fetch_repos().await {
                Ok(repos) if !repos.is_empty() => fleet.set(Fleet::Loaded(Arc::new(repos))),
                Ok(_) => {
                    notice.set(Some("No repositories found on this account.".to_string()));
                    fleet.set(Fleet::Loaded(Arc::new(Vec::new())));
                }
                Err(e) => {
                    notice.set(Some(format!("{e} — showing the demo fleet.")));
                    fleet.set(Fleet::Demo(Arc::new(demo::fleet(DEMO_NOW))));
                }
            }
        });
    };

    let show_demo = move |_| {
        notice.set(Some("Showing the demo fleet.".to_string()));
        selected.set(None);
        fleet.set(Fleet::Demo(Arc::new(demo::fleet(DEMO_NOW))));
    };
    let reload = move |_| load();

    // Fetch on startup — no setup, no token entry.
    load();

    view! {
        <div class="app">
            <header class="topbar">
                <div class="mark">
                    <span>"▜▒▟███▙▒▛"</span>
                    <span>"▟██▌█▐██▙"</span>
                    <span>"▀▐▖▀█▀▗▌▀"</span>
                </div>
                <div class="lede">
                    <div class="title">
                        <span class="brand">"cohors"</span>
                        <span class="pill">"web"</span>
                    </div>
                    <div class="tag">"All your git repositories at a glance"</div>
                </div>
                <div class="conn">
                    {move || match fleet.get() {
                        Fleet::Loaded(_) => view! {
                            <span class="src">"● GitHub"</span>
                            <button class="ghost" on:click=reload>"reload"</button>
                            <button class="ghost" on:click=show_demo>"demo"</button>
                        }
                        .into_any(),
                        Fleet::Demo(_) => view! {
                            <span class="src demo">"demo"</span>
                            <button class="ghost" on:click=reload>"connect GitHub"</button>
                        }
                        .into_any(),
                        Fleet::Loading => view! { <span class="dim">"loading…"</span> }.into_any(),
                    }}
                </div>
            </header>

            {move || notice.get().map(|n| view! { <div class="banner">{n}</div> })}

            {move || match fleet.get() {
                Fleet::Demo(s) => dashboard(s, DEMO_NOW, filter, sort, dirty_only, selected, true).into_any(),
                Fleet::Loaded(s) => dashboard(s, real_now(), filter, sort, dirty_only, selected, false).into_any(),
                Fleet::Loading => view! {
                    <div class="state">"Fetching your repositories from GitHub…"</div>
                }
                .into_any(),
            }}
        </div>
    }
}

/// The dashboard body for a given fleet + clock: the attention summary, the live
/// filter/sort/dirty-only controls, the fleet table, and the detail / standup
/// aside. `is_demo` only chooses the aside's default panel (the standup is demo
/// data).
fn dashboard(
    snaps: Arc<Vec<RepoSnapshot>>,
    now: i64,
    filter: RwSignal<String>,
    sort: RwSignal<SortMode>,
    dirty_only: RwSignal<bool>,
    selected: RwSignal<Option<String>>,
    is_demo: bool,
) -> impl IntoView {
    let summary = fleet_summary(&snaps, now);

    let view_of = {
        let snaps = snaps.clone();
        move || {
            let q = filter.get();
            let params = ViewParams {
                sort: sort.get(),
                dirty_only: dirty_only.get(),
                query: &q,
            };
            compute_view(&snaps, &params)
                .into_iter()
                .map(|vr| vr.index)
                .collect::<Vec<_>>()
        }
    };
    let body = {
        let snaps = snaps.clone();
        let view_of = view_of.clone();
        move || {
            view_of()
                .into_iter()
                .map(|i| repo_row(&snaps[i], selected, now))
                .collect::<Vec<_>>()
        }
    };
    let visible_count = {
        let view_of = view_of.clone();
        move || view_of().len()
    };
    let aside = {
        let snaps = snaps.clone();
        move || {
            selected
                .get()
                .and_then(|id| snaps.iter().find(|s| s.id.0 == id).cloned())
                .map(|s| detail_panel(&s, now).into_any())
                .unwrap_or_else(|| {
                    if is_demo {
                        standup_panel(now).into_any()
                    } else {
                        hint_panel().into_any()
                    }
                })
        }
    };

    view! {
        <>
            <section class="attention">{summary_chips(&summary)}</section>
            <section class="controls">
                <input
                    class="filter"
                    placeholder="filter repos — name, branch, fuzzy…"
                    prop:value=move || filter.get()
                    on:input=move |ev| filter.set(event_target_value(&ev))
                />
                <div class="sorts">
                    {sort_button(sort, SortMode::DirtyFirst, "dirty-first")}
                    {sort_button(sort, SortMode::Recent, "recent")}
                    {sort_button(sort, SortMode::Name, "name")}
                    {sort_button(sort, SortMode::AheadBehind, "ahead/behind")}
                </div>
                <button
                    class="toggle"
                    class:on=move || dirty_only.get()
                    on:click=move |_| dirty_only.update(|d| *d = !*d)
                >
                    "dirty only"
                </button>
            </section>
            <div class="grid">
                <section class="card fleet-wrap">
                    <div class="card-title">
                        "Repositories "
                        <span class="dim">{move || format!("({})", visible_count())}</span>
                        <span class="dim hint">"  ·  click a row for detail"</span>
                    </div>
                    <div class="scroll">
                        <table class="fleet">
                            <thead>
                                <tr>
                                    <th>"Repo"</th>
                                    <th>"Branch"</th>
                                    <th>"Sync"</th>
                                    <th>"Changes"</th>
                                    <th>"Stash"</th>
                                    <th>"PRs"</th>
                                    <th>"CI"</th>
                                    <th>"Last"</th>
                                    <th>"Status"</th>
                                </tr>
                            </thead>
                            <tbody>{body}</tbody>
                        </table>
                    </div>
                </section>
                <aside class="side">{aside}</aside>
            </div>
        </>
    }
}

/// The attention summary: one chip per fleet-wide count, coloured by urgency.
fn summary_chips(s: &FleetSummary) -> impl IntoView + use<> {
    let mut chips: Vec<(String, &'static str)> = Vec::new();
    chips.push((
        format!("{} of {} need attention", s.needs_attention, s.total),
        "accent",
    ));
    if s.unpushed > 0 {
        let aging = if s.unpushed_aging > 0 {
            format!(" ({} aging)", s.unpushed_aging)
        } else {
            String::new()
        };
        chips.push((format!("{} unpushed{aging}", s.unpushed), "risk"));
    }
    if s.behind > 0 {
        chips.push((format!("{} behind", s.behind), "warn"));
    }
    if s.dirty > 0 {
        chips.push((format!("{} dirty", s.dirty), "modified"));
    }
    if s.stash > 0 {
        chips.push((format!("{} stashed", s.stash), "dim"));
    }
    if s.errors > 0 {
        chips.push((format!("{} unreadable", s.errors), "error"));
    }
    chips
        .into_iter()
        .map(|(text, cls)| view! { <span class=format!("chip {cls}")>{text}</span> })
        .collect::<Vec<_>>()
}

/// One sort button, highlighted when it's the active mode.
fn sort_button(sort: RwSignal<SortMode>, mode: SortMode, label: &'static str) -> impl IntoView {
    view! {
        <button
            class="sort"
            class:active=move || sort.get() == mode
            on:click=move |_| sort.set(mode)
        >
            {label}
        </button>
    }
}

/// One fleet row — every cell built from `cohors-core` data + judgments. Clicking
/// it selects the repo (driving the detail aside).
fn repo_row(s: &RepoSnapshot, selected: RwSignal<Option<String>>, now: i64) -> impl IntoView + use<> {
    let a = assess(s, now);
    let id = s.id.0.clone();
    let id_click = id.clone();
    let is_sel = move || selected.get().as_deref() == Some(id.as_str());

    let name = s.name.clone();
    let name_class = if s.error.is_some() {
        "name error"
    } else if matches!(a.severity, Severity::Ok | Severity::Info) {
        "name clean"
    } else {
        "name"
    };

    let branch = match &s.branch {
        Branch::Named(b) => b.clone(),
        Branch::Detached(id) => format!("@{}", id.chars().take(7).collect::<String>()),
        Branch::Unborn => "unborn".to_string(),
    };

    let (sync, sync_class) = match &s.upstream {
        None => ("—".to_string(), "dim"),
        Some(up) if up.ahead == 0 && up.behind == 0 => ("·".to_string(), "dim"),
        Some(up) => {
            let mut p = String::new();
            if up.ahead > 0 {
                p.push_str(&format!("↑{}", up.ahead));
            }
            if up.behind > 0 {
                if !p.is_empty() {
                    p.push(' ');
                }
                p.push_str(&format!("↓{}", up.behind));
            }
            (p, if up.behind > 0 { "behind" } else { "ahead" })
        }
    };

    let w = &s.worktree;
    let total = w.staged + w.modified + w.untracked;
    let (changes, changes_class) = if s.error.is_some() || total == 0 {
        ("·".to_string(), "dim")
    } else if w.modified > 0 || w.untracked > 0 {
        (total.to_string(), "modified")
    } else {
        (total.to_string(), "staged")
    };

    let (stash, stash_class) = if s.stash_count == 0 {
        ("·".to_string(), "dim")
    } else {
        (s.stash_count.to_string(), "warn")
    };

    let (prs, prs_class) = match &s.remote {
        None => ("—".to_string(), "dim"),
        Some(r) if r.open_prs == 0 => ("·".to_string(), "dim"),
        Some(r) => (r.open_prs.to_string(), "ahead"),
    };

    let (ci, ci_class) = match &s.remote {
        None => ("—", "dim"),
        Some(r) => match r.ci {
            CiStatus::Passing => ("passing", "staged"),
            CiStatus::Failing => ("failing", "risk"),
            CiStatus::Pending => ("pending", "warn"),
            CiStatus::None => ("·", "dim"),
        },
    };

    let age = s
        .last_commit
        .as_ref()
        .map(|c| time::relative(c.timestamp, now))
        .unwrap_or_else(|| "—".to_string());

    let last = s
        .last_commit
        .as_ref()
        .map(|c| c.summary.clone())
        .unwrap_or_default();

    let (status, status_class) = if let Some(e) = &s.error {
        (e.clone(), "status risk".to_string())
    } else if let Some(r) = &a.primary {
        (r.label(), format!("status {}", sev_class(r.severity())))
    } else if !last.is_empty() {
        (last, "status dim".to_string())
    } else {
        ("·".to_string(), "status dim".to_string())
    };

    view! {
        <tr class:selected=is_sel on:click=move |_| selected.set(Some(id_click.clone()))>
            <td class=name_class>{name}</td>
            <td class="dim">{branch}</td>
            <td class=sync_class>{sync}</td>
            <td class=changes_class>{changes}</td>
            <td class=stash_class>{stash}</td>
            <td class=prs_class>{prs}</td>
            <td class=ci_class>{ci}</td>
            <td class="dim">{age}</td>
            <td class=status_class>{status}</td>
        </tr>
    }
}

/// The per-repo detail aside: a labelled facts card mirroring the TUI's context
/// pane.
fn detail_panel(s: &RepoSnapshot, now: i64) -> impl IntoView + use<> {
    let a = assess(s, now);
    let branch = match &s.branch {
        Branch::Named(b) => b.clone(),
        Branch::Detached(id) => format!("@{}", id.chars().take(7).collect::<String>()),
        Branch::Unborn => "unborn".to_string(),
    };
    let name = s.name.clone();

    let reasons: Vec<_> = if let Some(e) = &s.error {
        vec![view! { <li class="risk">{format!("⚠ {e}")}</li> }]
    } else if a.reasons.is_empty() {
        vec![view! { <li class="ok">{"clean — nothing needs you".to_string()}</li> }]
    } else {
        a.reasons
            .iter()
            .map(|r| {
                let cls = sev_class(r.severity());
                view! { <li class=cls>{r.label()}</li> }
            })
            .collect()
    };

    let w = &s.worktree;
    let changes = if w.staged + w.modified + w.untracked == 0 {
        "clean".to_string()
    } else {
        let mut parts = Vec::new();
        if w.staged > 0 {
            parts.push(format!("{} staged", w.staged));
        }
        if w.modified > 0 {
            parts.push(format!("{} modified", w.modified));
        }
        if w.untracked > 0 {
            parts.push(format!("{} untracked", w.untracked));
        }
        parts.join(" · ")
    };

    let stash = if s.stash_count == 0 {
        "none".to_string()
    } else {
        s.stash_count.to_string()
    };

    let upstream = match &s.upstream {
        None => "—".to_string(),
        Some(up) if up.ahead == 0 && up.behind == 0 => format!("even with {}", up.name),
        Some(up) => {
            let mut p = Vec::new();
            if up.ahead > 0 {
                p.push(format!("{} ahead", up.ahead));
            }
            if up.behind > 0 {
                p.push(format!("{} behind", up.behind));
            }
            format!("{}  ({})", p.join(" · "), up.name)
        }
    };

    let remote = match &s.remote {
        None => "—".to_string(),
        Some(r) => {
            let ci = match r.ci {
                CiStatus::Passing => "CI passing",
                CiStatus::Failing => "CI failing",
                CiStatus::Pending => "CI pending",
                CiStatus::None => "no CI",
            };
            let prs = if r.open_prs == 0 {
                "no open PRs".to_string()
            } else {
                format!(
                    "{} open PR{}",
                    r.open_prs,
                    if r.open_prs == 1 { "" } else { "s" }
                )
            };
            format!("{ci}  ·  {prs}")
        }
    };

    let last = s
        .last_commit
        .as_ref()
        .map(|c| {
            let age = time::relative(c.timestamp, now);
            if c.summary.is_empty() {
                format!("{age} ago")
            } else {
                format!("{age} ago — {}", c.summary)
            }
        })
        .unwrap_or_else(|| "—".to_string());

    let link = s.remote_url.clone();

    view! {
        <div class="card detail">
            <div class="card-title">{name}<span class="dim">{format!("  ·  {branch}")}</span></div>
            <div class="scroll">
                <ul class="reasons">{reasons}</ul>
                <dl class="facts">
                    <dt>"Changes"</dt><dd>{changes}</dd>
                    <dt>"Stash"</dt><dd>{stash}</dd>
                    <dt>"Upstream"</dt><dd>{upstream}</dd>
                    <dt>"Remote"</dt><dd>{remote}</dd>
                    <dt>"Last"</dt><dd>{last}</dd>
                </dl>
                {link.map(|url| {
                    let shown = url.clone();
                    view! {
                        <div class="link">
                            <a href=url target="_blank" rel="noreferrer">{shown}</a>
                        </div>
                    }
                })}
            </div>
        </div>
    }
}

/// The aside's default panel for real data: a quiet hint.
fn hint_panel() -> impl IntoView {
    view! {
        <div class="card">
            <div class="card-title">"Detail"</div>
            <div class="scroll"><p class="empty">"Select a repository to inspect it."</p></div>
        </div>
    }
}

/// The weekly-standup panel (demo data): commits this week, grouped by repo.
fn standup_panel(now: i64) -> impl IntoView + use<> {
    let commits = demo::standup(now);
    let groups = group_commits(&commits);
    let total = commits.len();

    let blocks: Vec<_> = groups
        .into_iter()
        .map(|(repo, cs)| {
            let items: Vec<_> = cs
                .iter()
                .map(|c| {
                    view! {
                        <li>
                            <span class="age dim">{time::relative(c.timestamp, now)}</span>
                            <span class="sha dim">{c.short_id.clone()}</span>
                            {c.summary.clone()}
                        </li>
                    }
                })
                .collect();
            view! {
                <div class="su-repo">
                    <div class="su-name">{repo}<span class="dim">{format!("  {}", items.len())}</span></div>
                    <ul class="su-list">{items}</ul>
                </div>
            }
        })
        .collect();

    view! {
        <div class="card standup">
            <div class="card-title">
                "This week "
                <span class="dim">{format!("· {total} commits")}</span>
            </div>
            <div class="scroll">{blocks}</div>
        </div>
    }
}

/// Severity → CSS class for the accent colour.
fn sev_class(s: Severity) -> &'static str {
    match s {
        Severity::Risk => "risk",
        Severity::Warn => "warn",
        Severity::Notice => "notice",
        _ => "dim",
    }
}
