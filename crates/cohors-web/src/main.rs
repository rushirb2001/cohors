//! cohors web — the fleet dashboard in the browser, driven by `cohors-core`
//! compiled to WebAssembly.
//!
//! The web app is *the same tool* as the TUI/CLI/MCP, in a different surface:
//! `cohors web` (the native server) scans the repos under `--root`/config, builds
//! `cohors-core` snapshots (local worktree status, ahead/behind, stash — and the
//! *why-it-needs-you* judgment from `assess`), enriches them with remote CI/PRs,
//! and serves them as JSON (see [`api`]). This page deserializes those same
//! models and renders them through the same `assess`/`compute_view`/`sort` logic
//! the TUI uses — local first, remote folded in. The demo fleet is the fallback
//! when there's nothing to scan.

mod api;

use cohors_core::{
    Branch, CiStatus, RepoSnapshot, Severity, SortMode, ViewParams, assess, compute_view, demo,
    fleet_summary, time,
};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

/// Fixed clock for the *demo* fleet, so its relative ages stay sensible.
const DEMO_NOW: i64 = 1_700_000_000;

/// The browser's real wall clock (Unix seconds) — used for live (scanned) data.
fn real_now() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Loading,
    Live,
    Demo,
}

/// The drill-in detail's load state for the selected repo.
#[derive(Clone)]
enum DetailState {
    Idle,
    Loading,
    Loaded(api::RepoDetailResponse),
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
    let dirty_only = RwSignal::new(false);
    let sort = RwSignal::new(SortMode::DirtyFirst);
    let selected = RwSignal::new(None::<String>);
    let roots = RwSignal::new(Vec::<String>::new());
    // True while the remote-enrichment pass is in flight, so rows that *could* be
    // GitHub (have a remote URL, no `remote` yet) show a spinner instead of "—".
    let enriching = RwSignal::new(false);

    // Load the local scan (fast — local status only), then re-request enriched so
    // remote CI/PRs fold in without blocking first paint. With nothing to scan
    // (or on error), fall back to the demo fleet with a note.
    let load = move || {
        notice.set(None);
        selected.set(None);
        enriching.set(false);
        mode.set(Mode::Loading);
        spawn_local(async move {
            match api::fetch_repos().await {
                Ok(list) if !list.is_empty() => {
                    repos.set(list);
                    mode.set(Mode::Live);
                    // Second pass: enrich with remote signals (one server call;
                    // the server fans out and caches). Merge by id so the local
                    // rows stay put and only `remote` fills in.
                    enriching.set(true);
                    spawn_local(async move {
                        if let Ok(enriched) = api::fetch_enriched().await {
                            repos.update(|v| {
                                for e in enriched {
                                    if let Some(s) = v.iter_mut().find(|s| s.id.0 == e.id.0) {
                                        s.remote = e.remote;
                                    }
                                }
                            });
                        }
                        enriching.set(false);
                    });
                }
                Ok(_) => {
                    notice.set(Some(
                        "No git repositories found to scan — showing the demo fleet.".to_string(),
                    ));
                    repos.set(demo::fleet(DEMO_NOW));
                    mode.set(Mode::Demo);
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

    // Pull the session metadata (which folder is being scanned), and — when the
    // server was started with `--watch` — poll for a fresh scan so the page tracks
    // the folder live (re-scanning is cheap; remote is server-cached). Skips while
    // showing the demo fleet.
    spawn_local(async move {
        let meta = api::fetch_meta().await;
        roots.set(meta.roots);
        if !meta.watch {
            return;
        }
        loop {
            gloo_timers::future::TimeoutFuture::new(5_000).await;
            if mode.get_untracked() != Mode::Live {
                continue;
            }
            if let Ok(list) = api::fetch_enriched().await
                && !list.is_empty()
                && mode.get_untracked() == Mode::Live
            {
                repos.set(list);
            }
        }
    });

    // Drill-in: when a repo is selected in a live scan, fetch its detail — local
    // recent commits / changed files / branches / stashes, plus remote PRs /
    // contributors / issues / release — on demand.
    let detail = RwSignal::new(DetailState::Idle);
    Effect::new(move |_| {
        let Some(id) = selected.get() else {
            detail.set(DetailState::Idle);
            return;
        };
        if mode.get_untracked() != Mode::Live {
            detail.set(DetailState::Idle);
            return;
        }
        let snap = repos.get_untracked().into_iter().find(|s| s.id.0 == id);
        let Some(snap) = snap else {
            detail.set(DetailState::Idle);
            return;
        };
        let Some(path) = snap.path.as_ref().map(|p| p.to_string()) else {
            detail.set(DetailState::Idle);
            return;
        };
        let url = snap.remote_url.clone();
        detail.set(DetailState::Loading);
        spawn_local(async move {
            let d = api::fetch_detail(&path, url.as_deref()).await;
            // Ignore a stale result if the selection moved on.
            if selected.get_untracked().as_deref() == Some(id.as_str()) {
                detail.set(DetailState::Loaded(d));
            }
        });
    });

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
                    <div class="tag">
                        {move || {
                            let r = roots.get();
                            if r.is_empty() {
                                "All your repos in one place — local status + remote".to_string()
                            } else {
                                format!("scanning {}", r.join(", "))
                            }
                        }}
                    </div>
                </div>
                <div class="conn">
                    {move || match mode.get() {
                        Mode::Live => view! {
                            <span class="src">"● local scan"</span>
                            <button class="ghost" on:click=reload>"rescan"</button>
                            <button class="ghost" on:click=show_demo>"demo"</button>
                        }
                        .into_any(),
                        Mode::Demo => view! {
                            <span class="src demo">"demo"</span>
                            <button class="ghost" on:click=reload>"rescan"</button>
                        }
                        .into_any(),
                        Mode::Loading => view! { <span class="dim">"scanning…"</span> }.into_any(),
                    }}
                </div>
            </header>

            {move || notice.get().map(|n| view! { <div class="banner">{n}</div> })}

            {move || match mode.get() {
                Mode::Loading => {
                    view! { <div class="state">"Scanning your repositories…"</div> }.into_any()
                }
                Mode::Demo => {
                    dashboard(repos, DEMO_NOW, filter, dirty_only, sort, selected, detail, enriching)
                        .into_any()
                }
                Mode::Live => dashboard(
                    repos,
                    real_now(),
                    filter,
                    dirty_only,
                    sort,
                    selected,
                    detail,
                    enriching,
                )
                .into_any(),
            }}
        </div>
    }
}

/// The dashboard body: the attention summary, filter/sort controls, the fleet
/// table (the TUI's columns), and the per-repo detail aside. Reads `repos`
/// reactively so the remote-enrichment pass updates the rows live.
#[allow(clippy::too_many_arguments)]
fn dashboard(
    repos: RwSignal<Vec<RepoSnapshot>>,
    now: i64,
    filter: RwSignal<String>,
    dirty_only: RwSignal<bool>,
    sort: RwSignal<SortMode>,
    selected: RwSignal<Option<String>>,
    detail: RwSignal<DetailState>,
    enriching: RwSignal<bool>,
) -> impl IntoView {
    let summary = move || summary_chips(&repos.get(), now);
    let visible = move || {
        let r = repos.get();
        let params = ViewParams {
            sort: sort.get(),
            dirty_only: dirty_only.get(),
            query: &filter.get(),
        };
        compute_view(&r, &params)
            .into_iter()
            .map(|row| row.index)
            .collect::<Vec<_>>()
    };
    let body = move || {
        let r = repos.get();
        let busy = enriching.get();
        visible()
            .into_iter()
            .map(|i| repo_row(&r[i], selected, now, busy))
            .collect::<Vec<_>>()
    };
    let count = move || visible().len();
    let aside = move || {
        selected
            .get()
            .and_then(|id| repos.get().into_iter().find(|s| s.id.0 == id))
            .map(|s| detail_panel(&s, now, detail).into_any())
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
                <button
                    class="sort"
                    class:active=move || dirty_only.get()
                    on:click=move |_| dirty_only.update(|d| *d = !*d)
                >
                    "needs attention"
                </button>
                <div class="sorts">
                    {sort_button(sort, SortMode::DirtyFirst, "attention")}
                    {sort_button(sort, SortMode::Recent, "recent")}
                    {sort_button(sort, SortMode::Name, "name")}
                    {sort_button(sort, SortMode::AheadBehind, "sync")}
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

/// The CSS color class for a severity — mirrors the TUI's `severity_style`.
fn severity_class(sev: Severity) -> &'static str {
    match sev {
        Severity::Ok => "ok",
        Severity::Info => "dim",
        Severity::Notice => "ahead",
        Severity::Warn => "warn",
        Severity::Risk => "risk",
    }
}

/// The attention summary strip, from `cohors-core`'s `fleet_summary` — the same
/// counts the TUI's header shows.
fn summary_chips(repos: &[RepoSnapshot], now: i64) -> impl IntoView + use<> {
    let s = fleet_summary(repos, now);
    let enriching = repos.iter().filter(|r| r.remote.is_none()).count();

    let mut chips: Vec<(String, &'static str)> =
        vec![(format!("{} repositories", s.total), "accent")];
    if s.needs_attention > 0 {
        chips.push((format!("{} need attention", s.needs_attention), "warn"));
    }
    if s.errors > 0 {
        chips.push((format!("{} unreadable", s.errors), "risk"));
    }
    if s.unpushed > 0 {
        let label = if s.unpushed_aging > 0 {
            format!("{} unpushed · {} aging", s.unpushed, s.unpushed_aging)
        } else {
            format!("{} unpushed", s.unpushed)
        };
        chips.push((label, "ahead"));
    }
    if s.behind > 0 {
        chips.push((format!("{} behind", s.behind), "notice"));
    }
    if s.dirty > 0 {
        chips.push((format!("{} dirty", s.dirty), "modified"));
    }
    if s.stash > 0 {
        chips.push((format!("{} stashed", s.stash), "dim"));
    }
    if s.needs_attention == 0 && s.errors == 0 {
        chips.push(("all clear".to_string(), "ok"));
    }
    if enriching == repos.len() && !repos.is_empty() {
        chips.push(("enriching remote…".to_string(), "dim"));
    }
    chips
        .into_iter()
        .map(|(text, cls)| view! { <span class=format!("chip {cls}")>{text}</span> })
        .collect::<Vec<_>>()
}

/// One fleet row, mirroring the TUI's dock columns. Clicking it selects the repo.
/// `busy` is true while remote enrichment is in flight (drives the PRs/CI spinner).
fn repo_row(
    s: &RepoSnapshot,
    selected: RwSignal<Option<String>>,
    now: i64,
    busy: bool,
) -> impl IntoView + use<> {
    let id = s.id.0.clone();
    let id_click = id.clone();
    let is_sel = move || selected.get().as_deref() == Some(id.as_str());

    // A broken repo: red name, the reason in the Status column. The data cells are
    // genuinely unknowable, so they're left blank rather than filled with noise.
    if let Some(reason) = &s.error {
        let name = s.name.clone();
        let reason = reason.clone();
        return view! {
            <tr class:selected=is_sel on:click=move |_| selected.set(Some(id_click.clone()))>
                <td class="name error">{name}</td>
                <td></td>
                <td></td>
                <td></td>
                <td></td>
                <td></td>
                <td></td>
                <td class="last"></td>
                <td class="status risk">{reason}</td>
            </tr>
        }
        .into_any();
    }

    let a = assess(s, now);
    let name_class = match a.severity {
        Severity::Ok | Severity::Info => "name dim",
        Severity::Warn | Severity::Risk => "name strong",
        Severity::Notice => "name",
    };
    let name = s.name.clone();

    view! {
        <tr class:selected=is_sel on:click=move |_| selected.set(Some(id_click.clone()))>
            <td class=name_class>{name}</td>
            <td>{branch_cell(s)}</td>
            <td>{sync_cell(s)}</td>
            <td>{changes_cell(s)}</td>
            <td>{stash_cell(s)}</td>
            <td>{prs_cell(s, busy)}</td>
            <td>{ci_cell(s, busy)}</td>
            <td class="last">{last_cell(s, now)}</td>
            <td class="status">{status_cell(&a)}</td>
        </tr>
    }
    .into_any()
}

// ── Default states ───────────────────────────────────────────────────────────
//
// The web never shows the terminal's cryptic `·`/`—`: every empty/default cell
// is a plain, tooltipped *word* appropriate to its column ("clean", "synced",
// "none", "local", "never", "up to date"), plus the braille dot-spinner for
// in-progress states (enriching, CI running).

/// A faint, tooltipped default word for an empty/neutral cell.
fn word(text: &'static str, tip: &'static str) -> AnyView {
    view! { <span class="state" title=tip>{text}</span> }.into_any()
}

/// The braille dot-spinner for in-progress states (loading / CI running).
fn spinner(tip: &'static str) -> AnyView {
    view! { <span class="spin" title=tip></span> }.into_any()
}

/// The Branch cell: branch name (long names truncated, full name on hover),
/// `@sha` for detached, "unborn" for a fresh repo.
fn branch_cell(s: &RepoSnapshot) -> impl IntoView + use<> {
    match &s.branch {
        Branch::Named(b) => {
            let full = b.clone();
            view! { <span class="branch" title=full>{ellipsize(b, 22)}</span> }.into_any()
        }
        Branch::Detached(id) => {
            let short: String = id.chars().take(7).collect();
            view! { <span class="detached">{format!("@{short}")}</span> }.into_any()
        }
        Branch::Unborn => view! { <span class="dim">"unborn"</span> }.into_any(),
    }
}

/// The Sync cell: `↑2 ↓5` ahead/behind arrows, "synced" when even, "local" when
/// the branch has no upstream.
fn sync_cell(s: &RepoSnapshot) -> impl IntoView + use<> {
    match &s.upstream {
        None => word("local", "no upstream — local branch"),
        Some(up) if up.ahead == 0 && up.behind == 0 => word("synced", "in sync with upstream"),
        Some(up) => {
            let ahead = (up.ahead > 0).then(|| {
                view! { <span class="ahead" title="commits to push">{format!("↑{}", up.ahead)}</span> }
            });
            let sep = (up.ahead > 0 && up.behind > 0).then(|| view! { <span>" "</span> });
            let behind = (up.behind > 0).then(|| {
                view! { <span class="behind" title="commits to pull">{format!("↓{}", up.behind)}</span> }
            });
            view! { <span>{ahead}{sep}{behind}</span> }.into_any()
        }
    }
}

/// The Changes cell: the count of uncommitted changes next to a pencil icon
/// (green when all staged, amber when there's unstaged work), or "clean".
fn changes_cell(s: &RepoSnapshot) -> impl IntoView + use<> {
    let w = &s.worktree;
    let total = w.staged + w.modified + w.untracked;
    if total == 0 {
        word("clean", "clean working tree")
    } else {
        let cls = format!("count {}", if w.modified > 0 || w.untracked > 0 { "modified" } else { "staged" });
        let tip = format!(
            "{} uncommitted change{} — {} staged · {} modified · {} untracked",
            total,
            if total == 1 { "" } else { "s" },
            w.staged,
            w.modified,
            w.untracked
        );
        view! {
            <span class=cls title=tip>
                {total.to_string()}
                {edit_icon()}
            </span>
        }
        .into_any()
    }
}

/// A small pencil icon (inherits the cell's color via `currentColor`) — the
/// visual shorthand for "uncommitted changes".
fn edit_icon() -> impl IntoView {
    view! {
        <svg
            class="ico-edit"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
        >
            <path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z" />
            <path d="m15 5 4 4" />
        </svg>
    }
}

/// The Stash cell: the stash count (amber), or "none".
fn stash_cell(s: &RepoSnapshot) -> impl IntoView + use<> {
    if s.stash_count > 0 {
        let tip = format!("{} stash entr{}", s.stash_count, if s.stash_count == 1 { "y" } else { "ies" });
        view! { <span class="warn" title=tip>{s.stash_count.to_string()}</span> }.into_any()
    } else {
        word("none", "no stashes")
    }
}

/// The PRs cell: open-PR count, "none" on a remote with none, "local" off-remote.
/// While remote enrichment is in flight, a GitHub repo not yet filled spins.
fn prs_cell(s: &RepoSnapshot, busy: bool) -> impl IntoView + use<> {
    match &s.remote {
        None if busy && s.remote_url.is_some() => spinner("checking pull requests…"),
        None => word("local", "no remote data"),
        Some(r) if r.open_prs == 0 => word("none", "no open pull requests"),
        Some(r) => {
            let tip = format!("{} open pull request{}", r.open_prs, if r.open_prs == 1 { "" } else { "s" });
            view! { <span class="ahead" title=tip>{r.open_prs.to_string()}</span> }.into_any()
        }
    }
}

/// The CI cell: "passing" / "failing" / a spinner+"pending" for a running build,
/// "none" for a remote with no CI, "local" off-remote (or a spinner mid-enrich).
fn ci_cell(s: &RepoSnapshot, busy: bool) -> impl IntoView + use<> {
    match &s.remote {
        None if busy && s.remote_url.is_some() => spinner("checking CI…"),
        None => word("local", "no remote data"),
        Some(r) => match r.ci {
            CiStatus::Passing => {
                view! { <span class="ok" title="CI passing">"passing"</span> }.into_any()
            }
            CiStatus::Failing => {
                view! { <span class="risk" title="CI failing">"failing"</span> }.into_any()
            }
            CiStatus::Pending => view! {
                <span class="ci-pending" title="CI running">
                    <span class="spin"></span>
                    <span class="warn">"pending"</span>
                </span>
            }
            .into_any(),
            CiStatus::None => word("none", "no CI configured"),
        },
    }
}

/// The Last cell: the last commit's age only (mirroring the TUI dock — the commit
/// subject lives in the detail aside, keeping the table compact). "never" if none.
fn last_cell(s: &RepoSnapshot, now: i64) -> impl IntoView + use<> {
    match &s.last_commit {
        Some(c) => {
            let age = time::relative(c.timestamp, now);
            let summary = c.summary.clone();
            view! { <span class="age" title=summary>{age}</span> }.into_any()
        }
        None => word("never", "no commits"),
    }
}

/// Truncate `s` to at most `max` characters, adding an ellipsis when cut.
fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The Status cell: the primary attention reason, colored by severity (the same
/// signal that drives the attention sort) — a calm "✓ up to date" when nothing
/// needs you.
fn status_cell(a: &cohors_core::Assessment) -> impl IntoView + use<> {
    match &a.primary {
        Some(r) => {
            let cls = severity_class(r.severity());
            view! { <span class=cls>{r.label()}</span> }.into_any()
        }
        None => word("up to date", "nothing needs attention"),
    }
}

/// The per-repo detail aside: the local facts + every reason it wants attention,
/// then the on-demand drill-in (recent commits, changed files, remote PRs, etc).
fn detail_panel(
    s: &RepoSnapshot,
    now: i64,
    detail: RwSignal<DetailState>,
) -> impl IntoView + use<> {
    let name = s.name.clone();
    let a = assess(s, now);

    // Every reason it wants attention (most urgent first), each severity-colored.
    let reasons: Vec<_> = a
        .reasons
        .iter()
        .map(|r| {
            let cls = severity_class(r.severity());
            let label = r.label();
            view! { <li class=cls>{label}</li> }
        })
        .collect();
    let reasons_block = (!a.reasons.is_empty()).then(|| {
        view! {
            <div class="sec">
                <div class="sec-h">"Needs attention"</div>
                <ul class="reasons">{reasons}</ul>
            </div>
        }
    });

    // Local facts pulled straight from the snapshot.
    let branch = s.branch.label();
    let sync = sync_text(s);
    let changes = changes_text(s);
    let stash = if s.stash_count > 0 { format!("{} stashed", s.stash_count) } else { "none".to_string() };
    let (ci_label, ci_cls) = ci_text(s);
    let prs = prs_text(s);
    let last = s
        .last_commit
        .as_ref()
        .map(|c| format!("{} ago · {}", time::relative(c.timestamp, now), c.summary))
        .unwrap_or_else(|| "never".to_string());
    let link = s.remote_url.clone();

    view! {
        <div class="card detail">
            <div class="card-title">{name}<span class="dim">{format!("  ·  {branch}")}</span></div>
            <div class="scroll">
                <dl class="facts">
                    <dt>"Sync"</dt><dd>{sync}</dd>
                    <dt>"Changes"</dt><dd>{changes}</dd>
                    <dt>"Stash"</dt><dd>{stash}</dd>
                    <dt>"CI"</dt><dd class=ci_cls>{ci_label}</dd>
                    <dt>"PRs"</dt><dd>{prs}</dd>
                    <dt>"Last"</dt><dd>{last}</dd>
                </dl>
                {reasons_block}
                {move || rich_block(detail.get(), now)}
                {link.map(|url| {
                    let shown = url.clone();
                    view! {
                        <div class="sec">
                            <div class="sec-h">"Remote source"</div>
                            <div class="link">
                                <a href=url target="_blank" rel="noreferrer">{shown}</a>
                            </div>
                        </div>
                    }
                })}
            </div>
        </div>
    }
}

/// The on-demand drill-in under the facts: local recent commits / changed files,
/// then remote PRs / contributors / issues / release. `Loading` shows the dots
/// spinner; `Idle` (demo) renders nothing.
fn rich_block(state: DetailState, now: i64) -> AnyView {
    match state {
        DetailState::Idle => ().into_any(),
        DetailState::Loading => view! {
            <div class="rich-loading"><span class="spin"></span>" loading detail…"</div>
        }
        .into_any(),
        DetailState::Loaded(d) => rich_sections(d, now).into_any(),
    }
}

/// Render the loaded drill-in. Empty sections are omitted.
fn rich_sections(d: api::RepoDetailResponse, now: i64) -> impl IntoView {
    let commits = (!d.local.recent_commits.is_empty()).then(|| {
        let rows = d
            .local
            .recent_commits
            .clone()
            .into_iter()
            .map(|c| {
                let age = time::relative(c.timestamp, now);
                view! {
                    <li>
                        <span class="sha">{c.short_id}</span>
                        <span class="msg">{c.summary}</span>
                        <span class="age">{format!("{} · {age} ago", c.author)}</span>
                    </li>
                }
            })
            .collect::<Vec<_>>();
        view! {
            <div class="sec">
                <div class="sec-h">"Recent commits"</div>
                <ul class="rows">{rows}</ul>
            </div>
        }
    });

    let changed = (!d.local.changed_files.is_empty()).then(|| {
        let rows = d
            .local
            .changed_files
            .clone()
            .into_iter()
            .map(|f| {
                view! {
                    <li>
                        <span class="sha">{f.status}</span>
                        <span class="msg">{f.path}</span>
                    </li>
                }
            })
            .collect::<Vec<_>>();
        view! {
            <div class="sec">
                <div class="sec-h">"Working tree"</div>
                <ul class="rows">{rows}</ul>
            </div>
        }
    });

    let remote = d.remote.map(|r| {
        let stats = format!(
            "{} open issue{}{}",
            r.open_issues,
            if r.open_issues == 1 { "" } else { "s" },
            r.latest_release
                .as_ref()
                .map(|t| format!("  ·  release {t}"))
                .unwrap_or_default()
        );
        let prs = (!r.prs.is_empty()).then(|| {
            let rows = r
                .prs
                .clone()
                .into_iter()
                .map(|p| {
                    let draft = p.draft.then(|| view! { <span class="badge">"draft"</span> });
                    view! {
                        <li>
                            <a class="sha" href=p.url target="_blank" rel="noreferrer">
                                {format!("#{}", p.number)}
                            </a>
                            <span class="msg">{p.title}</span>
                            {draft}
                            <span class="age">{p.author}</span>
                        </li>
                    }
                })
                .collect::<Vec<_>>();
            view! {
                <div class="sec">
                    <div class="sec-h">"Open PRs"</div>
                    <ul class="rows">{rows}</ul>
                </div>
            }
        });
        let contribs = (!r.contributors.is_empty()).then(|| {
            let rows = r
                .contributors
                .clone()
                .into_iter()
                .map(|c| {
                    view! {
                        <li>
                            <span class="msg">{c.login}</span>
                            <span class="age">{format!("{} commits", c.contributions)}</span>
                        </li>
                    }
                })
                .collect::<Vec<_>>();
            view! {
                <div class="sec">
                    <div class="sec-h">"Top contributors"</div>
                    <ul class="rows">{rows}</ul>
                </div>
            }
        });
        view! {
            <div class="sec">
                <div class="sec-h">"Remote"</div>
                <div class="stats">{stats}</div>
            </div>
            {prs}
            {contribs}
        }
    });

    view! { <div class="rich">{commits}{changed}{remote}</div> }
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
fn sort_button(sort: RwSignal<SortMode>, key: SortMode, label: &'static str) -> impl IntoView {
    view! {
        <button class="sort" class:active=move || sort.get() == key on:click=move |_| sort.set(key)>
            {label}
        </button>
    }
}

// ── Text variants of the cells, for the detail facts list ────────────────────

fn sync_text(s: &RepoSnapshot) -> String {
    match &s.upstream {
        None => "no upstream".to_string(),
        Some(up) if up.ahead == 0 && up.behind == 0 => "in sync".to_string(),
        Some(up) => {
            let mut parts = Vec::new();
            if up.ahead > 0 {
                parts.push(format!("↑{} ahead", up.ahead));
            }
            if up.behind > 0 {
                parts.push(format!("↓{} behind", up.behind));
            }
            parts.join(" · ")
        }
    }
}

fn changes_text(s: &RepoSnapshot) -> String {
    let w = &s.worktree;
    if w.staged + w.modified + w.untracked == 0 {
        return "clean".to_string();
    }
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
}

fn ci_text(s: &RepoSnapshot) -> (&'static str, &'static str) {
    match &s.remote {
        None => ("local", "dim"),
        Some(r) => match r.ci {
            CiStatus::Passing => ("passing", "ok"),
            CiStatus::Failing => ("failing", "risk"),
            CiStatus::Pending => ("pending", "warn"),
            CiStatus::None => ("no CI", "dim"),
        },
    }
}

fn prs_text(s: &RepoSnapshot) -> String {
    match &s.remote {
        None => "local".to_string(),
        Some(r) if r.open_prs == 0 => "none".to_string(),
        Some(r) => format!("{} open", r.open_prs),
    }
}
