//! cohors web — a GitHub fleet-health dashboard in the browser, driven by
//! `cohors-core` models compiled to WebAssembly.
//!
//! `cohors web` (the native server) proxies GitHub with your existing login, so
//! the page shows *your* repos with zero setup (see [`github`]). The browser has
//! no local working copy, so — unlike the TUI's local view — this surfaces the
//! *remote* signals that matter across a fleet: **CI status**, **open PRs**,
//! activity, each enriched live after the list loads. The demo fleet is the
//! offline fallback.

mod github;

use cohors_core::{Branch, CiStatus, RepoSnapshot, demo, time};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

/// Fixed clock for the *demo* fleet, so its relative ages stay sensible.
const DEMO_NOW: i64 = 1_700_000_000;
/// A repo with no push in this long reads as "stale".
const STALE_SECS: i64 = 90 * 24 * 60 * 60;

/// The browser's real wall clock (Unix seconds) — used for live GitHub data.
fn real_now() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Loading,
    Live,
    Demo,
}

#[derive(Clone, Copy, PartialEq)]
enum SortKey {
    Attention,
    Recent,
    Name,
    Prs,
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let repos = RwSignal::new(Vec::<RepoSnapshot>::new());
    let mode = RwSignal::new(Mode::Loading);
    let notice = RwSignal::new(None::<String>);
    let filter = RwSignal::new(String::new());
    let sort = RwSignal::new(SortKey::Attention);
    let selected = RwSignal::new(None::<String>);

    // Load the repo list through the proxy, then enrich each repo (CI + PRs)
    // progressively so rows light up as their health arrives. On failure, fall
    // back to the demo fleet with a note.
    let load = move || {
        notice.set(None);
        selected.set(None);
        mode.set(Mode::Loading);
        spawn_local(async move {
            match github::fetch_repos().await {
                Ok(list) if !list.is_empty() => {
                    let targets: Vec<(String, String)> = list
                        .iter()
                        .filter_map(|s| match &s.branch {
                            Branch::Named(b) => Some((s.id.0.clone(), b.clone())),
                            _ => None,
                        })
                        .collect();
                    repos.set(list);
                    mode.set(Mode::Live);
                    for (id, branch) in targets {
                        if let Some(info) = github::enrich(&id, &branch).await {
                            repos.update(|v| {
                                if let Some(s) = v.iter_mut().find(|s| s.id.0 == id) {
                                    s.remote = Some(info);
                                }
                            });
                        }
                    }
                }
                Ok(_) => {
                    notice.set(Some("No repositories on this account.".to_string()));
                    repos.set(Vec::new());
                    mode.set(Mode::Live);
                }
                Err(e) => {
                    notice.set(Some(format!("{e} — showing the demo fleet.")));
                    repos.set(demo::fleet(DEMO_NOW));
                    mode.set(Mode::Demo);
                }
            }
        });
    };

    let show_demo = move |_| {
        notice.set(Some("Showing the demo fleet.".to_string()));
        selected.set(None);
        repos.set(demo::fleet(DEMO_NOW));
        mode.set(Mode::Demo);
    };
    let reload = move |_| load();
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
                    <div class="tag">"Your GitHub fleet, at a glance"</div>
                </div>
                <div class="conn">
                    {move || match mode.get() {
                        Mode::Live => view! {
                            <span class="src">"● GitHub"</span>
                            <button class="ghost" on:click=reload>"reload"</button>
                            <button class="ghost" on:click=show_demo>"demo"</button>
                        }
                        .into_any(),
                        Mode::Demo => view! {
                            <span class="src demo">"demo"</span>
                            <button class="ghost" on:click=reload>"connect GitHub"</button>
                        }
                        .into_any(),
                        Mode::Loading => view! { <span class="dim">"loading…"</span> }.into_any(),
                    }}
                </div>
            </header>

            {move || notice.get().map(|n| view! { <div class="banner">{n}</div> })}

            {move || match mode.get() {
                Mode::Loading => {
                    view! { <div class="state">"Fetching your repositories from GitHub…"</div> }
                        .into_any()
                }
                Mode::Demo => dashboard(repos, DEMO_NOW, filter, sort, selected).into_any(),
                Mode::Live => dashboard(repos, real_now(), filter, sort, selected).into_any(),
            }}
        </div>
    }
}

/// The dashboard body: the health summary, filter/sort controls, the fleet
/// table, and the per-repo detail aside. Reads `repos` reactively so enrichment
/// updates the rows live.
fn dashboard(
    repos: RwSignal<Vec<RepoSnapshot>>,
    now: i64,
    filter: RwSignal<String>,
    sort: RwSignal<SortKey>,
    selected: RwSignal<Option<String>>,
) -> impl IntoView {
    let summary = move || summary_chips(&repos.get(), now);
    let visible = move || view_indices(&repos.get(), &filter.get(), sort.get(), now);
    let body = move || {
        let r = repos.get();
        visible()
            .into_iter()
            .map(|i| repo_row(&r[i], selected, now))
            .collect::<Vec<_>>()
    };
    let count = move || visible().len();
    let aside = move || {
        selected
            .get()
            .and_then(|id| repos.get().into_iter().find(|s| s.id.0 == id))
            .map(|s| detail_panel(&s, now).into_any())
            .unwrap_or_else(|| hint_panel().into_any())
    };

    view! {
        <>
            <section class="attention">{summary}</section>
            <section class="controls">
                <input
                    class="filter"
                    placeholder="filter repos — name…"
                    prop:value=move || filter.get()
                    on:input=move |ev| filter.set(event_target_value(&ev))
                />
                <div class="sorts">
                    {sort_button(sort, SortKey::Attention, "attention")}
                    {sort_button(sort, SortKey::Recent, "recent")}
                    {sort_button(sort, SortKey::Name, "name")}
                    {sort_button(sort, SortKey::Prs, "PRs")}
                </div>
            </section>
            <div class="grid">
                <section class="card fleet-wrap">
                    <div class="card-title">
                        "Repositories "
                        <span class="dim">{move || format!("({})", count())}</span>
                        <span class="dim hint">"  ·  click a row for detail"</span>
                    </div>
                    <div class="scroll">
                        <table class="fleet">
                            <thead>
                                <tr>
                                    <th>"Repo"</th>
                                    <th>"PRs"</th>
                                    <th>"CI"</th>
                                    <th>"Last"</th>
                                    <th>"About"</th>
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

/// Health rank for sorting/attention: failing CI is most urgent, then open PRs,
/// then pending CI, then stale; a healthy or not-yet-enriched repo is 0.
fn health_rank(s: &RepoSnapshot, now: i64) -> u8 {
    match &s.remote {
        None => 0,
        Some(r) => {
            if r.ci == CiStatus::Failing {
                5
            } else if r.open_prs > 0 {
                4
            } else if r.ci == CiStatus::Pending {
                3
            } else if s.last_commit.as_ref().is_some_and(|c| now - c.timestamp > STALE_SECS) {
                1
            } else {
                0
            }
        }
    }
}

/// The health summary chips, computed across the fleet.
fn summary_chips(repos: &[RepoSnapshot], now: i64) -> impl IntoView + use<> {
    let total = repos.len();
    let enriching = repos.iter().filter(|s| s.remote.is_none()).count();
    let failing = repos
        .iter()
        .filter(|s| s.remote.as_ref().is_some_and(|r| r.ci == CiStatus::Failing))
        .count();
    let pending = repos
        .iter()
        .filter(|s| s.remote.as_ref().is_some_and(|r| r.ci == CiStatus::Pending))
        .count();
    let prs: u32 = repos
        .iter()
        .filter_map(|s| s.remote.as_ref().map(|r| r.open_prs))
        .sum();
    let stale = repos
        .iter()
        .filter(|s| health_rank(s, now) == 1)
        .count();

    let mut chips: Vec<(String, &'static str)> = vec![(format!("{total} repositories"), "accent")];
    if failing > 0 {
        chips.push((format!("{failing} failing CI"), "risk"));
    }
    if pending > 0 {
        chips.push((format!("{pending} CI pending"), "warn"));
    }
    if prs > 0 {
        chips.push((format!("{prs} open PRs"), "notice"));
    }
    if stale > 0 {
        chips.push((format!("{stale} stale"), "dim"));
    }
    if enriching > 0 {
        chips.push((format!("enriching {enriching}…"), "dim"));
    } else if failing == 0 && pending == 0 {
        chips.push(("all green".to_string(), "ok"));
    }
    chips
        .into_iter()
        .map(|(text, cls)| view! { <span class=format!("chip {cls}")>{text}</span> })
        .collect::<Vec<_>>()
}

/// Filter (substring on name) then order by the chosen key. Returns indices into
/// `repos`.
fn view_indices(repos: &[RepoSnapshot], query: &str, sort: SortKey, now: i64) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    let mut idx: Vec<usize> = repos
        .iter()
        .enumerate()
        .filter(|(_, s)| q.is_empty() || s.name.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect();
    let recency = |s: &RepoSnapshot| s.last_commit.as_ref().map(|c| c.timestamp).unwrap_or(0);
    idx.sort_by(|&a, &b| {
        let (ra, rb) = (&repos[a], &repos[b]);
        match sort {
            SortKey::Attention => health_rank(rb, now)
                .cmp(&health_rank(ra, now))
                .then(recency(rb).cmp(&recency(ra))),
            SortKey::Recent => recency(rb).cmp(&recency(ra)),
            SortKey::Name => ra.name.to_lowercase().cmp(&rb.name.to_lowercase()),
            SortKey::Prs => {
                let pa = ra.remote.as_ref().map(|r| r.open_prs).unwrap_or(0);
                let pb = rb.remote.as_ref().map(|r| r.open_prs).unwrap_or(0);
                pb.cmp(&pa).then(recency(rb).cmp(&recency(ra)))
            }
        }
    });
    idx
}

/// One CI cell's (label, class). `·` is "still enriching"; once enriched, a repo
/// with no checks reads as "no CI" rather than an ambiguous dash.
fn ci_view(s: &RepoSnapshot) -> (&'static str, &'static str) {
    match &s.remote {
        None => ("·", "dim"),
        Some(r) => match r.ci {
            CiStatus::Passing => ("passing", "staged"),
            CiStatus::Failing => ("failing", "risk"),
            CiStatus::Pending => ("pending", "warn"),
            CiStatus::None => ("no CI", "dim"),
        },
    }
}

/// One fleet row. Clicking it selects the repo (driving the detail aside).
fn repo_row(s: &RepoSnapshot, selected: RwSignal<Option<String>>, now: i64) -> impl IntoView + use<> {
    let id = s.id.0.clone();
    let id_click = id.clone();
    let is_sel = move || selected.get().as_deref() == Some(id.as_str());

    let failing = s
        .remote
        .as_ref()
        .is_some_and(|r| r.ci == CiStatus::Failing);
    let name = s.name.clone();
    let name_class = if failing { "name risk" } else { "name" };

    let (prs, prs_class) = match &s.remote {
        None => ("·".to_string(), "dim"),
        Some(r) if r.open_prs == 0 => ("·".to_string(), "dim"),
        Some(r) => (r.open_prs.to_string(), "ahead"),
    };
    let (ci, ci_class) = ci_view(s);

    let age = s
        .last_commit
        .as_ref()
        .map(|c| time::relative(c.timestamp, now))
        .unwrap_or_else(|| "—".to_string());
    let about = s
        .last_commit
        .as_ref()
        .map(|c| c.summary.clone())
        .unwrap_or_default();

    view! {
        <tr class:selected=is_sel on:click=move |_| selected.set(Some(id_click.clone()))>
            <td class=name_class>{name}</td>
            <td class=prs_class>{prs}</td>
            <td class=ci_class>{ci}</td>
            <td class="dim">{age}</td>
            <td class="about">{about}</td>
        </tr>
    }
}

/// The per-repo detail aside: the remote facts this dashboard knows.
fn detail_panel(s: &RepoSnapshot, now: i64) -> impl IntoView + use<> {
    let branch = match &s.branch {
        Branch::Named(b) => b.clone(),
        Branch::Detached(id) => format!("@{}", id.chars().take(7).collect::<String>()),
        Branch::Unborn => "unborn".to_string(),
    };
    let name = s.name.clone();

    let (ci_label, ci_class) = match &s.remote {
        None => ("enriching…", "dim"),
        Some(r) => match r.ci {
            CiStatus::Passing => ("passing", "ok"),
            CiStatus::Failing => ("failing", "risk"),
            CiStatus::Pending => ("pending", "warn"),
            CiStatus::None => ("no CI", "dim"),
        },
    };
    let prs = match &s.remote {
        None => "…".to_string(),
        Some(r) if r.open_prs == 0 => "none".to_string(),
        Some(r) => format!(
            "{} open PR{}",
            r.open_prs,
            if r.open_prs == 1 { "" } else { "s" }
        ),
    };
    let last = s
        .last_commit
        .as_ref()
        .map(|c| {
            let age = time::relative(c.timestamp, now);
            let stale = now - c.timestamp > STALE_SECS;
            if stale {
                format!("{age} ago · stale")
            } else {
                format!("{age} ago")
            }
        })
        .unwrap_or_else(|| "—".to_string());
    let about = s
        .last_commit
        .as_ref()
        .map(|c| c.summary.clone())
        .filter(|d| !d.is_empty());
    let link = s.remote_url.clone();

    view! {
        <div class="card detail">
            <div class="card-title">{name}<span class="dim">{format!("  ·  {branch}")}</span></div>
            <div class="scroll">
                <dl class="facts">
                    <dt>"CI"</dt><dd class=ci_class>{ci_label}</dd>
                    <dt>"PRs"</dt><dd>{prs}</dd>
                    <dt>"Activity"</dt><dd>{last}</dd>
                </dl>
                {about.map(|d| view! { <p class="about-detail">{d}</p> })}
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

/// The aside's default panel (nothing selected).
fn hint_panel() -> impl IntoView {
    view! {
        <div class="card">
            <div class="card-title">"Detail"</div>
            <div class="scroll"><p class="empty">"Select a repository to inspect it."</p></div>
        </div>
    }
}

/// One sort button, highlighted when active.
fn sort_button(sort: RwSignal<SortKey>, key: SortKey, label: &'static str) -> impl IntoView {
    view! {
        <button class="sort" class:active=move || sort.get() == key on:click=move |_| sort.set(key)>
            {label}
        </button>
    }
}
