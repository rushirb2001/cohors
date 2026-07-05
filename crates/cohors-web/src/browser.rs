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

use crate::api;
use cohors_core::{
    AttentionReason, Branch, CiStatus, RepoSnapshot, Severity, SortMode, ViewParams, assess,
    compute_view, demo, fleet_summary, time,
};
use leptos::prelude::*;
use std::collections::HashSet;
use wasm_bindgen::JsCast;
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

/// Mount the Leptos app to the page body (called from the wasm `main`).
pub fn mount() {
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
    let marked = RwSignal::new(HashSet::<String>::new());
    // An action staged for the confirm modal (verb + resolved targets).
    let pending = RwSignal::new(None::<PendingAction>);
    // The completed action's per-repo outcomes, shown in the results modal.
    let result = RwSignal::new(None::<ActionResult>);
    // Which action dropdown is open: "all", "selected", or a repo id.
    let menu = RwSignal::new(None::<String>);
    // The modal's free-text argument: commit message / run command.
    let action_arg = RwSignal::new(String::new());
    // Guided tour: the current step (None = not touring), the on-screen rect of
    // the highlighted element (spotlight), and where the floating card sits (so
    // it slides between steps).
    let tour = RwSignal::new(None::<usize>);
    let tour_rect = RwSignal::new(None::<(f64, f64, f64, f64)>);
    let tour_card_pos = RwSignal::new(None::<(f64, f64)>);
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
                            // Once CI is known, bring focus to a failing build by
                            // opening its detail — unless the user already picked a
                            // repo while enrichment was in flight.
                            if selected.get_untracked().is_none() {
                                let failing = repos.get_untracked().into_iter().find(|s| {
                                    matches!(
                                        s.remote.as_ref().map(|r| r.ci),
                                        Some(CiStatus::Failing)
                                    )
                                });
                                if let Some(s) = failing {
                                    selected.set(Some(s.id.0));
                                }
                            }
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

    // First-ever visit: auto-run the tour once, after the fleet has rendered so
    // the spotlight targets exist. A localStorage flag makes it show only once.
    if !tour_seen() {
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(750).await;
            if !tour_seen() {
                tour.set(Some(0));
            }
        });
    }

    // Guided tour: when the step changes, apply its demo side-effect (select /
    // mark a repo, or conjure a modal — all safe, never a real git action) and,
    // after a tick so the DOM has settled, measure the target and float the card.
    Effect::new(move |_| {
        let Some(i) = tour.get() else {
            // Tour closed: clear everything it may have staged.
            tour_rect.set(None);
            tour_card_pos.set(None);
            pending.set(None);
            result.set(None);
            return;
        };
        let step = &TOUR[i];
        match step.act {
            TourAct::Select => {
                pending.set(None);
                result.set(None);
                if let Some(id) = first_visible_repo() {
                    marked.update(|m| m.clear());
                    selected.set(Some(id));
                }
            }
            TourAct::Mark => {
                pending.set(None);
                result.set(None);
                selected.set(None);
                if let Some(id) = first_visible_repo() {
                    marked.update(|m| {
                        m.insert(id);
                    });
                }
            }
            TourAct::ShowConfirm => {
                selected.set(None);
                result.set(None);
                let names: Vec<String> = repos
                    .get_untracked()
                    .iter()
                    .take(3)
                    .map(|r| r.name.clone())
                    .collect();
                pending.set(Some(PendingAction {
                    verb: "pull".to_string(),
                    selector: serde_json::json!({ "all": true }),
                    targets: names,
                }));
            }
            TourAct::ShowResults => {
                selected.set(None);
                pending.set(None);
                let rows: Vec<ResultRow> = repos
                    .get_untracked()
                    .iter()
                    .take(3)
                    .enumerate()
                    .map(|(n, r)| ResultRow {
                        repo: r.name.clone(),
                        state: if n == 2 { "skip" } else { "ok" },
                        message: if n == 2 {
                            "1 conflict — resolve first".to_string()
                        } else {
                            "pulled".to_string()
                        },
                        stdout: String::new(),
                        stderr: String::new(),
                    })
                    .collect();
                result.set(Some(ActionResult {
                    verb: "pull".to_string(),
                    summary: "2 ok · 1 skipped".to_string(),
                    rows,
                }));
            }
            TourAct::Clear => {
                selected.set(None);
                marked.update(|m| m.clear());
                pending.set(None);
                result.set(None);
            }
            TourAct::None => {}
        }
        let target = step.target;
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(90).await;
            let rect = target.and_then(element_rect);
            tour_rect.set(rect);
            tour_card_pos.set(Some(card_position(rect)));
        });
    });

    view! {
        <div class="app">
            {move || notice.get().map(|n| view! { <div class="banner">{n}</div> })}

            {move || match mode.get() {
                Mode::Loading => {
                    view! { <div class="state">"Scanning your repositories…"</div> }.into_any()
                }
                Mode::Demo => dashboard(
                    DashCtx {
                        repos,
                        now: DEMO_NOW,
                        filter,
                        dirty_only,
                        sort,
                        selected,
                        marked,
                        menu,
                        pending,
                        action_arg,
                        notice,
                        detail,
                        enriching,
                        mode,
                        roots,
                        tour,
                    },
                    reload,
                )
                .into_any(),
                Mode::Live => dashboard(
                    DashCtx {
                        repos,
                        now: real_now(),
                        filter,
                        dirty_only,
                        sort,
                        selected,
                        marked,
                        menu,
                        pending,
                        action_arg,
                        notice,
                        detail,
                        enriching,
                        mode,
                        roots,
                        tour,
                    },
                    reload,
                )
                .into_any(),
            }}
            {confirm_modal(pending, action_arg, notice, result)}
            {result_modal(result)}
            {tour_overlay(tour, tour_rect, tour_card_pos, selected, marked)}
        </div>
    }
}

/// The signals the dashboard reads/writes — bundled so the call site isn't a
/// wall of positional arguments.
#[derive(Clone, Copy)]
struct DashCtx {
    repos: RwSignal<Vec<RepoSnapshot>>,
    now: i64,
    filter: RwSignal<String>,
    dirty_only: RwSignal<bool>,
    sort: RwSignal<SortMode>,
    selected: RwSignal<Option<String>>,
    marked: RwSignal<HashSet<String>>,
    menu: RwSignal<Option<String>>,
    pending: RwSignal<Option<PendingAction>>,
    action_arg: RwSignal<String>,
    notice: RwSignal<Option<String>>,
    detail: RwSignal<DetailState>,
    enriching: RwSignal<bool>,
    mode: RwSignal<Mode>,
    roots: RwSignal<Vec<String>>,
    tour: RwSignal<Option<usize>>,
}

/// The dashboard body: a header-band Overview (brand + fleet + at-a-glance
/// stats), the fleet table whose column headers *are* the sort control, and the
/// per-repo detail aside. Reads `repos` reactively so the enrichment pass
/// updates the rows live.
fn dashboard(
    cx: DashCtx,
    reload: impl Fn(leptos::ev::MouseEvent) + Copy + Send + 'static,
) -> impl IntoView {
    let DashCtx {
        repos,
        now,
        filter,
        dirty_only,
        sort,
        selected,
        marked,
        menu,
        pending,
        action_arg,
        notice,
        detail,
        enriching,
        mode,
        roots,
        tour,
    } = cx;
    let summary = move || summary_chips(&repos.get(), now);
    let visible = move || {
        let r = repos.get();
        let params = ViewParams {
            sort: sort.get(),
            dirty_only: dirty_only.get(),
            query: &filter.get(),
            now,
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
            .map(|i| {
                repo_row(
                    &r[i], selected, marked, menu, pending, action_arg, notice, now, busy,
                )
            })
            .collect::<Vec<_>>()
    };
    let aside = move || {
        let list = repos.get();
        match selected
            .get()
            .and_then(|id| list.iter().find(|s| s.id.0 == id).cloned())
        {
            Some(s) => detail_panel(&s, now, detail).into_any(),
            None => hint_panel(&list, now, tour).into_any(),
        }
    };

    view! {
        <>
            // 1 · Header row — the brand mark in its own isolated card, then the
            // overview (fleet name + path, stats, scan status) beside it.
            <div class="top-row">
                <section class="card brand-card">
                    <div class="mark">
                        <span>"▜▒▟███▙▒▛"</span>
                        <span>"▟██▌█▐██▙"</span>
                        <span>"▀▐▖▀█▀▗▌▀"</span>
                    </div>
                    <div class="title">
                        <span class="brand">"cohors"</span>
                        <span class="pill">"web"</span>
                    </div>
                </section>
                <section class="card overview">
                    <div class="ov-top">
                        <div class="fleet-name">
                            <span class="fleet-label">"Your Fleet"</span>
                            {move || {
                                let r = roots.get();
                                (!r.is_empty())
                                    .then(|| {
                                        let path = r
                                            .iter()
                                            .map(|p| shorten_home(p))
                                            .collect::<Vec<_>>()
                                            .join("  ·  ");
                                        view! {
                                            <span class="fleet-path" title=r.join(", ")>
                                                {folder_icon()}
                                                {path}
                                            </span>
                                        }
                                    })
                            }}
                        </div>
                        <div class="conn">
                            {move || {
                                let demo = mode.get() == Mode::Demo;
                                view! {
                                    <span class=if demo { "src demo" } else { "src" }>
                                        {if demo { "demo fleet" } else { "● local scan" }}
                                    </span>
                                    <button class="ghost" on:click=reload>
                                        "rescan"
                                    </button>
                                }
                            }}
                        </div>
                    </div>
                    <section class="attention">{summary}</section>
                </section>
            </div>
            <div class="grid">
                // 2 · Repositories: filter + act controls in the subhead row; the
                // column headers double as the sort control (no separate buttons).
                <section class="card fleet-wrap">
                    <header class="panel-head repos-head">
                        <span class="panel-label">"Repositories"</span>
                        <input
                            class="filter"
                            placeholder="filter repos — name…"
                            prop:value=move || filter.get()
                            on:input=move |ev| filter.set(event_target_value(&ev))
                        />
                        <button
                            class="toggle"
                            class:on=move || dirty_only.get()
                            on:click=move |_| dirty_only.update(|d| *d = !*d)
                            title="Show only repos that need attention"
                        >
                            "needs attention"
                        </button>
                        <div class="act-controls">
                            {move || {
                                let m = marked.get();
                                (!m.is_empty())
                                    .then(|| {
                                        let ids: Vec<String> = m.iter().cloned().collect();
                                        act_dropdown(
                                            format!("selected · {}", ids.len()),
                                            "selected".to_string(),
                                            serde_json::json!({ "ids": ids }),
                                            menu,
                                            pending,
                                            action_arg,
                                            notice,
                                        )
                                    })
                            }}
                            {act_dropdown(
                                "act on all".to_string(),
                                "all".to_string(),
                                serde_json::json!({ "all": true }),
                                menu,
                                pending,
                                action_arg,
                                notice,
                            )}
                        </div>
                    </header>
                    <div class="scroll">
                        <table class="fleet">
                            <thead>
                                <tr>
                                    <th class="mk">"Mark"</th>
                                    {sort_th("Repo", SortMode::Name, sort)}
                                    {sort_th("Sync", SortMode::AheadBehind, sort)}
                                    <th>"Stash"</th>
                                    <th>"PRs"</th>
                                    <th>"CI"</th>
                                    {sort_th("Last", SortMode::Recent, sort)}
                                    <th>"Activity"</th>
                                    {sort_th("Status", SortMode::DirtyFirst, sort)}
                                    <th class="act">"Git"</th>
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

/// A sortable column header: clicking it sets that sort mode; the active column
/// shows a caret. This *is* the sort control now — there are no separate sort
/// buttons. `Sync`→ahead/behind, `Last`→recency, `Repo`→name, `Status`→attention.
fn sort_th(label: &'static str, mode: SortMode, sort: RwSignal<SortMode>) -> impl IntoView {
    let active = move || sort.get() == mode;
    view! {
        <th
            class="sortable"
            class:sorted=active
            on:click=move |_| sort.set(mode)
            title="Sort by this column"
        >
            {label}
            // A faint ⇅ marks every sortable column; the active one shows a solid
            // ↓ in the accent colour, so the affordance is always visible.
            <span class="caret">{move || if active() { " ↓" } else { " ⇅" }}</span>
        </th>
    }
}

/// A CSS tint class for the detail header, from the repo's CI state: red for a
/// failing build, green for passing, amber while pending, grey when there's no CI
/// (or the remote hasn't been enriched yet).
fn ci_tint(s: &RepoSnapshot) -> &'static str {
    match s.remote.as_ref().map(|r| r.ci) {
        Some(CiStatus::Failing) => "ci-fail",
        Some(CiStatus::Passing) => "ci-pass",
        Some(CiStatus::Pending) => "ci-pending",
        _ => "ci-none",
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

/// One action staged for the confirm modal: the verb, its resolved target
/// selector, and the repo names it will hit (from a server-side dry run).
#[derive(Clone)]
struct PendingAction {
    verb: String,
    selector: serde_json::Value,
    targets: Vec<String>,
}

/// Kick off an action: dry-run it server-side to resolve the exact target set,
/// then stage the confirm modal. Every action — one repo, the selection, or the
/// fleet — passes through here, so the user always sees the list of
/// repositories in focus before anything runs.
fn start_action(
    verb: String,
    selector: serde_json::Value,
    pending: RwSignal<Option<PendingAction>>,
    arg: RwSignal<String>,
    notice: RwSignal<Option<String>>,
) {
    let mut preview =
        serde_json::json!({ "verb": verb, "selector": selector.clone(), "dry_run": true });
    // commit/run demand their argument even for a dry run; the real value is
    // typed into the modal afterwards.
    if verb == "commit" {
        preview["message"] = serde_json::json!("(pending)");
    }
    if verb == "run" {
        preview["command"] = serde_json::json!("(pending)");
    }
    spawn_local(async move {
        match api::post_action(preview).await {
            Ok(v) => {
                if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
                    notice.set(Some(e.to_string()));
                    return;
                }
                let targets: Vec<String> = v
                    .get("repos")
                    .and_then(|r| r.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|r| r.get("name").and_then(|n| n.as_str()))
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                if targets.is_empty() {
                    notice.set(Some(format!("{verb}: no matching repos")));
                    return;
                }
                arg.set(String::new());
                pending.set(Some(PendingAction {
                    verb,
                    selector,
                    targets,
                }));
            }
            Err(e) => notice.set(Some(e)),
        }
    });
}

/// A dropdown of every registry verb (write parity with the TUI keymap and MCP
/// catalog — enforced by the server-side parity test). `key` says which menu
/// this is in the shared `menu` signal: "all", "selected", or a repo id.
#[allow(clippy::too_many_arguments)]
fn act_dropdown(
    label: String,
    key: String,
    selector: serde_json::Value,
    menu: RwSignal<Option<String>>,
    pending: RwSignal<Option<PendingAction>>,
    arg: RwSignal<String>,
    notice: RwSignal<Option<String>>,
) -> impl IntoView {
    let key_open = key.clone();
    let key_toggle = key;
    // The menu is positioned `fixed` at the trigger so it escapes the table's
    // overflow clipping (a row's menu was getting cut off by the scroll box).
    let pos = RwSignal::new((0.0_f64, 0.0_f64));
    view! {
        <span class="dd">
            <button
                class="ghost dd-btn"
                on:click=move |ev| {
                    ev.stop_propagation();
                    if let Some(el) = ev
                        .current_target()
                        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                    {
                        let r = el.get_bounding_client_rect();
                        let vw = viewport().0;
                        pos.set((r.left().min(vw - 172.0).max(8.0), r.bottom() + 4.0));
                    }
                    menu.update(|m| {
                        *m = if m.as_deref() == Some(key_toggle.as_str()) {
                            None
                        } else {
                            Some(key_toggle.clone())
                        };
                    });
                }
            >
                {label}
                " ▾"
            </button>
            {move || {
                (menu.get().as_deref() == Some(key_open.as_str()))
                    .then(|| {
                        let sel = selector.clone();
                        let (l, t) = pos.get();
                        view! {
                            <div class="dd-menu" style=format!("left:{l}px;top:{t}px")>
                                {cohors_web::ACTION_VERBS
                                    .iter()
                                    .map(|v| {
                                        let verb = *v;
                                        let sel = sel.clone();
                                        view! {
                                            <button
                                                class="dd-item"
                                                on:click=move |_| {
                                                    menu.set(None);
                                                    start_action(
                                                        verb.to_string(),
                                                        sel.clone(),
                                                        pending,
                                                        arg,
                                                        notice,
                                                    );
                                                }
                                            >
                                                {verb}
                                            </button>
                                        }
                                    })
                                    .collect::<Vec<_>>()}
                            </div>
                        }
                    })
            }}
        </span>
    }
}

/// The confirm modal: names the verb, lists every repository in focus, takes
/// the commit message / run command when the verb needs one, and only acts on
/// Confirm. Clicking the backdrop (or Cancel) discards the staged action.
fn confirm_modal(
    pending: RwSignal<Option<PendingAction>>,
    arg: RwSignal<String>,
    notice: RwSignal<Option<String>>,
    result: RwSignal<Option<ActionResult>>,
) -> impl IntoView {
    move || {
        pending.get().map(|p| {
            let needs_text = p.verb == "commit" || p.verb == "run";
            let placeholder = if p.verb == "run" {
                "shell command"
            } else {
                "commit message"
            };
            let title = format!(
                "{} — {} repositor{}",
                p.verb,
                p.targets.len(),
                if p.targets.len() == 1 { "y" } else { "ies" },
            );
            let go = move |_| {
                let Some(p) = pending.get_untracked() else {
                    return;
                };
                let text = arg.get_untracked().trim().to_string();
                let needs_text = p.verb == "commit" || p.verb == "run";
                if needs_text && text.is_empty() {
                    notice.set(Some(format!(
                        "{} needs a {}",
                        p.verb,
                        if p.verb == "run" {
                            "command"
                        } else {
                            "message"
                        },
                    )));
                    return;
                }
                pending.set(None);
                let mut body = serde_json::json!({
                    "verb": p.verb, "selector": p.selector, "confirm": true
                });
                if p.verb == "commit" {
                    body["message"] = serde_json::json!(text);
                }
                if p.verb == "run" {
                    body["command"] = serde_json::json!(text);
                }
                let verb = p.verb.clone();
                // While it runs, a quiet banner; the outcome opens the results modal.
                notice.set(Some(format!("running {verb}…")));
                spawn_local(async move {
                    let resp = api::post_action(body).await;
                    match parse_result(&verb, resp) {
                        Ok(r) => {
                            notice.set(None);
                            result.set(Some(r));
                        }
                        // A gate/transport error has no per-repo rows — surface it
                        // as the banner, exactly as before.
                        Err(msg) => notice.set(Some(msg)),
                    }
                });
            };
            view! {
                <div class="overlay" on:click=move |_| pending.set(None)>
                    <div class="modal" on:click=|ev| ev.stop_propagation()>
                        <h3>{title}</h3>
                        <ul class="targets">
                            {p
                                .targets
                                .iter()
                                .map(|t| view! { <li>{t.clone()}</li> })
                                .collect::<Vec<_>>()}
                        </ul>
                        {needs_text
                            .then(|| {
                                view! {
                                    <input
                                        class="action-arg"
                                        type="text"
                                        placeholder=placeholder
                                        prop:value=move || arg.get()
                                        on:input=move |ev| arg.set(event_target_value(&ev))
                                    />
                                }
                            })}
                        <div class="modal-actions">
                            <button class="confirm" on:click=go>
                                "Confirm"
                            </button>
                            <button class="ghost" on:click=move |_| pending.set(None)>
                                "Cancel"
                            </button>
                        </div>
                    </div>
                </div>
            }
        })
    }
}

/// A completed action's outcome, for the results modal. `summary` is the
/// headline (ok / skipped / failed counts); `rows` are the per-repo lines.
#[derive(Clone)]
struct ActionResult {
    verb: String,
    summary: String,
    rows: Vec<ResultRow>,
}

/// One repo's outcome in the results modal.
#[derive(Clone)]
struct ResultRow {
    /// Short repo name (the directory name, not the full path id).
    repo: String,
    /// "ok" | "skip" | "fail" — drives the colour and the marker.
    state: &'static str,
    /// The one-line outcome: a git status message, or `run`'s "exit N".
    message: String,
    /// `run` command output (empty for git verbs).
    stdout: String,
    stderr: String,
}

/// The marker glyph for a result state (matches the TUI's ✓/⊘/✗).
fn mark_glyph(state: &str) -> &'static str {
    match state {
        "ok" => "✓",
        "skip" => "⊘",
        _ => "✗",
    }
}

/// Parse an action response into an [`ActionResult`]. Handles both the git-verb
/// shape (`results:[{repo,ok,message,skipped}]`) and the `run` shape
/// (`results:[{repo,ok,exit_code,stdout,stderr,timed_out}]`). Returns `Err` with
/// the server's message for a gate/transport error (no per-repo rows to show).
fn parse_result(
    verb: &str,
    resp: Result<serde_json::Value, String>,
) -> Result<ActionResult, String> {
    let v = resp?;
    if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
        return Err(e.to_string());
    }
    let raw = v
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let is_run = verb == "run";
    let rows: Vec<ResultRow> = raw
        .iter()
        .map(|r| {
            // The `repo` field is the id — for scanned repos that's the full
            // path, so show just the final component (the repo's own name).
            let id = r.get("repo").and_then(|x| x.as_str()).unwrap_or("?");
            let repo = id.rsplit(['/', '\\']).next().unwrap_or(id).to_string();
            let ok = r.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
            let skipped = r.get("skipped").and_then(|x| x.as_bool()).unwrap_or(false);
            let state = if skipped {
                "skip"
            } else if ok {
                "ok"
            } else {
                "fail"
            };
            if is_run {
                let code = r.get("exit_code").and_then(|x| x.as_i64()).unwrap_or(-1);
                let timed_out = r
                    .get("timed_out")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                let message = if timed_out {
                    "timed out".to_string()
                } else {
                    format!("exit {code}")
                };
                ResultRow {
                    repo,
                    state,
                    message,
                    stdout: r
                        .get("stdout")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    stderr: r
                        .get("stderr")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                }
            } else {
                ResultRow {
                    repo,
                    state,
                    message: r
                        .get("message")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    stdout: String::new(),
                    stderr: String::new(),
                }
            }
        })
        .collect();

    let ok = rows.iter().filter(|r| r.state == "ok").count();
    let skipped = rows.iter().filter(|r| r.state == "skip").count();
    let failed = rows.iter().filter(|r| r.state == "fail").count();
    let mut summary = format!("{ok} ok");
    if skipped > 0 {
        summary.push_str(&format!(" · {skipped} skipped"));
    }
    if failed > 0 {
        summary.push_str(&format!(" · {failed} failed"));
    }
    Ok(ActionResult {
        verb: verb.to_string(),
        summary,
        rows,
    })
}

/// The results modal. Git verbs get a simple per-repo outcome list. `run`
/// mirrors the TUI: a master-detail — a compact repo list on the left (marker +
/// name + exit) and the focused repo's captured output on the right.
fn result_modal(result: RwSignal<Option<ActionResult>>) -> impl IntoView {
    // Which repo's output the run pane shows; reset whenever a new result opens.
    let focus = RwSignal::new(0usize);
    Effect::new(move |_| {
        let _ = result.get();
        focus.set(0);
    });
    move || {
        result.get().map(|r| {
            let title = format!("{} — {}", r.verb, r.summary);
            let is_run = r.verb == "run";
            let modal_class = if is_run {
                "modal results run"
            } else {
                "modal results"
            };
            let body = if is_run {
                run_result_body(r.rows.clone(), focus).into_any()
            } else {
                list_result_body(&r.rows).into_any()
            };
            view! {
                <div class="overlay" on:click=move |_| result.set(None)>
                    <div class=modal_class on:click=|ev| ev.stop_propagation()>
                        <h3>{title}</h3>
                        {body}
                        <div class="modal-actions">
                            <button class="confirm" on:click=move |_| result.set(None)>
                                "Done"
                            </button>
                        </div>
                    </div>
                </div>
            }
        })
    }
}

/// Git-verb results: a flat list of `marker name — message` rows.
fn list_result_body(rows: &[ResultRow]) -> impl IntoView {
    let items = rows
        .iter()
        .map(|row| {
            view! {
                <li class=format!("res-row {}", row.state)>
                    <span class="res-mark">{mark_glyph(row.state)}</span>
                    <span class="res-repo">{row.repo.clone()}</span>
                    <span class="res-msg">{row.message.clone()}</span>
                </li>
            }
        })
        .collect::<Vec<_>>();
    view! { <ul class="res-list">{items}</ul> }
}

/// `run` results: the TUI's master-detail. The left list selects a repo; the
/// right pane shows that repo's exit line, stdout, and a stderr section.
fn run_result_body(rows: Vec<ResultRow>, focus: RwSignal<usize>) -> impl IntoView {
    let rows_out = rows.clone();
    let list = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let state = row.state;
            let repo = row.repo.clone();
            let message = row.message.clone();
            view! {
                <li
                    class=format!("run-item {state}")
                    class:focused=move || focus.get() == i
                    on:click=move |_| focus.set(i)
                >
                    <span class="res-mark">{mark_glyph(state)}</span>
                    <span class="res-repo">{repo}</span>
                    <span class="run-exit">{message}</span>
                </li>
            }
        })
        .collect::<Vec<_>>();

    let output = move || {
        let n = rows_out.len();
        if n == 0 {
            return ().into_any();
        }
        let row = rows_out[focus.get().min(n - 1)].clone();
        let exit_class = if row.state == "ok" { "ok" } else { "risk" };
        let stdout = (!row.stdout.is_empty())
            .then(|| view! { <pre class="run-stream">{row.stdout.clone()}</pre> });
        let stderr = (!row.stderr.is_empty()).then(|| {
            view! {
                <div class="run-stderr-h">"── stderr ──"</div>
                <pre class="run-stream err">{row.stderr.clone()}</pre>
            }
        });
        let empty = (row.stdout.is_empty() && row.stderr.is_empty())
            .then(|| view! { <p class="run-empty">"(no output)"</p> });
        view! {
            <div class=format!("run-head {exit_class}")>{row.message.clone()}</div>
            {stdout}
            {stderr}
            {empty}
        }
        .into_any()
    };

    view! {
        <div class="run-panes">
            <ul class="run-list">{list}</ul>
            <div class="run-out-pane">{output}</div>
        </div>
    }
}

/// One fleet row, mirroring the TUI's dock columns. Clicking it selects the repo.
/// `busy` is true while remote enrichment is in flight (drives the PRs/CI spinner).
#[allow(clippy::too_many_arguments)]
fn repo_row(
    s: &RepoSnapshot,
    selected: RwSignal<Option<String>>,
    marked: RwSignal<HashSet<String>>,
    menu: RwSignal<Option<String>>,
    pending: RwSignal<Option<PendingAction>>,
    action_arg: RwSignal<String>,
    notice: RwSignal<Option<String>>,
    now: i64,
    busy: bool,
) -> impl IntoView + use<> {
    let id = s.id.0.clone();
    let id_click = id.clone();
    // The mark checkbox: toggles this repo in the action bar's target set
    // without moving the drill-in selection (the click doesn't bubble).
    let id_mark = id.clone();
    let id_tog = id.clone();
    let id_act = id.clone();
    let id_data = id.clone();
    let is_sel = move || selected.get().as_deref() == Some(id.as_str());
    let mark_td = view! {
        <td
            class="mk"
            on:click=move |ev| {
                ev.stop_propagation();
                marked
                    .update(|m| {
                        if !m.remove(id_tog.as_str()) {
                            m.insert(id_tog.clone());
                        }
                    });
            }
        >
            <input
                type="checkbox"
                prop:checked=move || marked.get().contains(id_mark.as_str())
            />
        </td>
    };

    // A broken repo: red name, the reason in the Status column. The data cells are
    // genuinely unknowable, so they're left blank rather than filled with noise.
    if let Some(reason) = &s.error {
        let name = s.name.clone();
        let reason = reason.clone();
        return view! {
            <tr class:selected=is_sel on:click=move |_| selected.set(Some(id_click.clone()))>
                {mark_td}
                <td class="repo error">{name}</td>
                <td></td>
                <td></td>
                <td></td>
                <td></td>
                <td class="last"></td>
                <td class="spark-cell"></td>
                <td class="status risk">{reason}</td>
                <td class="act"></td>
            </tr>
        }
        .into_any();
    }

    let a = assess(s, now);
    let name_class = match a.severity {
        Severity::Ok | Severity::Info => "repo dim",
        Severity::Warn | Severity::Risk => "repo strong",
        Severity::Notice => "repo",
    };

    view! {
        <tr
            data-repo=id_data
            class:selected=is_sel
            on:click=move |_| selected.set(Some(id_click.clone()))
        >
            {mark_td}
            <td class=name_class>{repo_cell(s)}</td>
            <td>{sync_cell(s)}</td>
            <td>{stash_cell(s)}</td>
            <td>{prs_cell(s, busy)}</td>
            <td>{ci_cell(s, busy)}</td>
            <td class="last">{last_cell(s, now)}</td>
            <td class="spark-cell">{activity_cell(s)}</td>
            <td class="status">{status_cell(&a)}</td>
            <td class="act" on:click=|ev| ev.stop_propagation()>
                {act_dropdown(
                    "act".to_string(),
                    id_act.clone(),
                    serde_json::json!({ "ids": [id_act.clone()] }),
                    menu,
                    pending,
                    action_arg,
                    notice,
                )}
            </td>
            <td class="spacer"></td>
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
    view! { <span class="muted" title=tip>{text}</span> }.into_any()
}

/// The braille dot-spinner for in-progress states (loading / CI running).
fn spinner(tip: &'static str) -> AnyView {
    view! { <span class="spin" title=tip></span> }.into_any()
}

/// The fused Repo cell: the repo name followed by a dim `@branch` (e.g.
/// `intern_challenge @main`), `@sha` for detached, `unborn` for a fresh repo.
fn repo_cell(s: &RepoSnapshot) -> impl IntoView + use<> {
    let name = s.name.clone();
    let branch = match &s.branch {
        Branch::Named(b) => format!("@{b}"),
        Branch::Detached(id) => format!("@{}", id.chars().take(7).collect::<String>()),
        Branch::Unborn => "unborn".to_string(),
    };
    let tip = format!("{} {branch}", s.name);
    view! {
        <span class="rname" title=tip.clone()>{name}</span>
        <span class="rbranch" title=tip>{format!(" {branch}")}</span>
    }
}

/// The Sync cell: a green cloud-check when in sync, a faint cloud-slash for a
/// local-only branch, or `↑2 ↓5` ahead/behind arrows.
fn sync_cell(s: &RepoSnapshot) -> impl IntoView + use<> {
    match &s.upstream {
        None => iconw(cloud_off_icon(), "muted", "no upstream — local branch"),
        Some(up) if up.ahead == 0 && up.behind == 0 => {
            iconw(cloud_icon(), "ok", "in sync with upstream")
        }
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

// ── Icon set ─────────────────────────────────────────────────────────────────
//
// Small stroked SVG icons (not emoji), inheriting the cell's color via
// `currentColor`, so a state reads as a glyph at a glance and the table stays
// compact. Each is paired with a `title` tooltip at the call site.

/// `<svg class="ic">` wrapper around a path string — keeps the icon fns tiny.
/// Explicit `width`/`height` so the glyph is always text-sized, not the SVG
/// default; color comes from the cell via `currentColor`.
macro_rules! icon {
    ($($body:tt)*) => {
        view! {
            <svg
                class="ic"
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
            >
                $($body)*
            </svg>
        }
    };
}

/// Pencil — uncommitted changes.
fn edit_icon() -> impl IntoView {
    icon! {
        <path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z" />
        <path d="m15 5 4 4" />
    }
}

/// Check — a cleared/good state (clean tree, no stashes, up to date).
fn check_icon() -> impl IntoView {
    icon! { <path d="M20 6 9 17l-5-5" /> }
}

/// Cloud — the branch is in sync with its upstream (shown green; ahead/behind use
/// arrows instead, so a cloud only ever means "synced").
fn cloud_icon() -> impl IntoView {
    icon! { <path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z" /> }
}

/// Cloud-off (slashed) — no upstream (the branch isn't tracking a remote).
fn cloud_off_icon() -> impl IntoView {
    icon! {
        <path d="m2 2 20 20" />
        <path d="M5.782 5.782A7 7 0 0 0 9 19h8.5a4.5 4.5 0 0 0 1.307-.193" />
        <path d="M21.532 16.5A4.5 4.5 0 0 0 17.5 10h-1.79A7.008 7.008 0 0 0 10 5.07" />
    }
}

/// Git pull-request — open PRs.
fn pr_icon() -> impl IntoView {
    icon! {
        <circle cx="6" cy="6" r="3" />
        <circle cx="18" cy="18" r="3" />
        <path d="M13 6h3a2 2 0 0 1 2 2v7" />
        <line x1="6" x2="6" y1="9" y2="21" />
    }
}

/// Archive box — stashes.
fn stash_icon() -> impl IntoView {
    icon! {
        <rect width="20" height="5" x="2" y="3" rx="1" />
        <path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8" />
        <path d="M10 12h4" />
    }
}

/// Git branch — detached HEAD.
fn branch_icon() -> impl IntoView {
    icon! {
        <line x1="6" x2="6" y1="3" y2="15" />
        <circle cx="18" cy="6" r="3" />
        <circle cx="6" cy="18" r="3" />
        <path d="M18 9a9 9 0 0 1-9 9" />
    }
}

/// Alert triangle — an unreadable repo.
fn alert_icon() -> impl IntoView {
    icon! {
        <path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3z" />
        <path d="M12 9v4" />
        <path d="M12 17h.01" />
    }
}

/// Clock — last activity.
fn clock_icon() -> impl IntoView {
    icon! {
        <circle cx="12" cy="12" r="10" />
        <polyline points="12 6 12 12 16 14" />
    }
}

/// Activity pulse — recent commit cadence (the sparkline's label).
fn activity_icon() -> impl IntoView {
    icon! { <path d="M3 12h4l3 8 4-16 3 8h4" /> }
}

/// Folder — the scanned fleet root in the header.
fn folder_icon() -> impl IntoView {
    icon! {
        <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" />
    }
}

/// Tidy an absolute path for display: a leading `/Users/<name>` or `/home/<name>`
/// becomes `~`, the way a shell would show it. Anything else is left as-is.
fn shorten_home(p: &str) -> String {
    for prefix in ["/Users/", "/home/"] {
        if let Some(rest) = p.strip_prefix(prefix) {
            return match rest.split_once('/') {
                Some((_, tail)) => format!("~/{tail}"),
                None => "~".to_string(),
            };
        }
    }
    p.to_string()
}

/// Wrap an icon in a tooltipped, colored span. `class` carries the color.
fn iconw<V: IntoView>(icon: V, class: &'static str, tip: &'static str) -> AnyView {
    view! { <span class=class title=tip>{icon}</span> }.into_any()
}

/// A count next to its column icon (e.g. `3 ✎`), colored by `color`.
fn count_icon<V: IntoView>(n: u32, icon: V, color: &'static str, tip: String) -> AnyView {
    view! { <span class=format!("count {color}") title=tip>{n.to_string()}{icon}</span> }.into_any()
}

/// The Stash cell: a box icon + count (amber), or a faint box when there are none.
fn stash_cell(s: &RepoSnapshot) -> impl IntoView + use<> {
    if s.stash_count > 0 {
        let tip = format!(
            "{} stash entr{}",
            s.stash_count,
            if s.stash_count == 1 { "y" } else { "ies" }
        );
        count_icon(s.stash_count, stash_icon(), "warn", tip)
    } else {
        iconw(stash_icon(), "muted", "no stashes")
    }
}

/// The PRs cell: a pull-request icon + count (open PRs), a faint PR icon when
/// there are none / off-remote, a spinner while enrichment is in flight.
fn prs_cell(s: &RepoSnapshot, busy: bool) -> impl IntoView + use<> {
    match &s.remote {
        None if busy && s.remote_url.is_some() => spinner("checking pull requests…"),
        None => iconw(pr_icon(), "muted", "no remote data"),
        Some(r) if r.open_prs == 0 => iconw(pr_icon(), "muted", "no open pull requests"),
        Some(r) => {
            let tip = format!(
                "{} open pull request{}",
                r.open_prs,
                if r.open_prs == 1 { "" } else { "s" }
            );
            count_icon(r.open_prs, pr_icon(), "pr", tip)
        }
    }
}

/// The CI cell: "passing" / "failing" as text, a spinner+"pending" for a running
/// build, "no CI" for a remote with no CI, "local" off-remote (spinner mid-enrich).
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
            CiStatus::None => word("no CI", "no CI configured"),
        },
    }
}

/// The Activity cell: a compact bar sparkline of the snapshot's weekly commit
/// counts (oldest → newest), scaled to the busiest week. A muted dash when the
/// data is unknown (old caches, or repos read without the commit walk).
fn activity_cell(s: &RepoSnapshot) -> impl IntoView + use<> {
    sparkline(&s.activity)
}

/// A tiny inline-SVG bar sparkline over `weeks` (one count per week, oldest first).
/// Heights are scaled to the busiest week; a zero week renders a faint baseline
/// tick so the cadence still reads. Empty/all-zero data → a muted dash.
fn sparkline(weeks: &[u8]) -> AnyView {
    let total: u32 = weeks.iter().map(|&c| c as u32).sum();
    if weeks.is_empty() || total == 0 {
        // A plain word, like the other empty cells ("clean", "never", "local"),
        // rather than a cryptic dash.
        return word("quiet", "no commits in the last 12 weeks");
    }
    // Geometry in SVG user units; the CSS sizes the rendered glyph. `viewBox`
    // makes it scale cleanly, so these are just relative proportions.
    let max = weeks.iter().copied().max().unwrap_or(1).max(1) as f32;
    const BAR_W: f32 = 3.0;
    const GAP: f32 = 1.0;
    const H: f32 = 14.0;
    let bars = weeks
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            // Busiest week fills the height; an empty week keeps a 1u baseline.
            let bh = ((c as f32 / max) * H).max(1.0);
            let x = i as f32 * (BAR_W + GAP);
            let y = H - bh;
            let faint = (c == 0).then_some("faint");
            view! {
                <rect
                    class=faint
                    x=x.to_string()
                    y=y.to_string()
                    width=BAR_W.to_string()
                    height=bh.to_string()
                    rx="0.5"
                />
            }
        })
        .collect::<Vec<_>>();
    let total_w = weeks.len() as f32 * (BAR_W + GAP);
    let tip = format!(
        "{total} commit{} over {} week{}",
        if total == 1 { "" } else { "s" },
        weeks.len(),
        if weeks.len() == 1 { "" } else { "s" },
    );
    view! {
        <svg
            class="spark"
            viewBox=format!("0 0 {total_w} {H}")
            preserveAspectRatio="xMidYMid meet"
            fill="currentColor"
            aria-hidden="true"
        >
            <title>{tip}</title>
            {bars}
        </svg>
    }
    .into_any()
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

/// The Status cell: the primary attention reason. Wide layouts show the full
/// sentence (the same text the TUI shows — "12 uncommitted changes", "branch
/// never pushed"); narrow layouts collapse to the compact icon + count. Both
/// forms are rendered and CSS picks one by viewport width.
fn status_cell(a: &cohors_core::Assessment) -> impl IntoView + use<> {
    match &a.primary {
        Some(r) => {
            let cls = format!("st {}", severity_class(r.severity()));
            let tip = r.label();
            let full = tip.clone();
            view! {
                <span class=cls title=tip>
                    <span class="st-full">{full}</span>
                    <span class="st-compact">{reason_body(r)}</span>
                </span>
            }
            .into_any()
        }
        None => view! {
            <span class="st ok" title="up to date — nothing needs attention">
                <span class="st-full muted">"up to date"</span>
                <span class="st-compact">{check_icon()}</span>
            </span>
        }
        .into_any(),
    }
}

/// The icon + count for one attention reason (colour comes from the parent's
/// severity class via `currentColor`).
fn reason_body(r: &AttentionReason) -> AnyView {
    use AttentionReason::*;
    match r {
        Unpushed { commits, .. } => view! { <span>{format!("↑{commits}")}</span> }.into_any(),
        Behind { commits } => view! { <span>{format!("↓{commits}")}</span> }.into_any(),
        Diverged { ahead, behind } => {
            view! { <span>{format!("↑{ahead} ↓{behind}")}</span> }.into_any()
        }
        Uncommitted {
            staged,
            modified,
            untracked,
        } => {
            let n = staged + modified + untracked;
            view! { {edit_icon()}<span>{n.to_string()}</span> }.into_any()
        }
        Stash { count, .. } => view! { {stash_icon()}<span>{count.to_string()}</span> }.into_any(),
        Conflicted { files } => view! { <span>{format!("✖{files}")}</span> }.into_any(),
        Operation(op) => view! { <span>{format!("{}…", op.label())}</span> }.into_any(),
        StaleRemote { days } => view! { <span>{format!("{days}d stale")}</span> }.into_any(),
        Detached => view! { {branch_icon()} }.into_any(),
        // Branch never pushed: it lives only on this disk, so a cloud-off glyph —
        // the same "not backed up to a remote" idea the Sync cell uses.
        Unpublished => view! { {cloud_off_icon()} }.into_any(),
        Unreadable => view! { {alert_icon()} }.into_any(),
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
    // "Needs attention" and the working tree are fused into one section: the
    // reasons (from the snapshot, shown immediately) followed by the concrete
    // changed files + diff behind them (from the async drill-in). The files are
    // exactly what "N uncommitted changes" refers to, so they belong together.
    let attention_block = (!a.reasons.is_empty()).then(|| {
        view! {
            <details class="sec" open>
                <summary class="sec-h">"Needs attention"</summary>
                <ul class="reasons">{reasons}</ul>
                {move || working_tree_block(detail.get())}
            </details>
        }
    });

    // Local facts pulled straight from the snapshot. Sync + CI live in the header
    // now, so they're not repeated here.
    let branch = s.branch.label();
    let changes = changes_text(s);
    let stash = if s.stash_count > 0 {
        format!("{} stashed", s.stash_count)
    } else {
        "none".to_string()
    };
    let prs = prs_text(s);
    let last = s
        .last_commit
        .as_ref()
        .map(|c| format!("{} ago · {}", time::relative(c.timestamp, now), c.summary))
        .unwrap_or_else(|| "never".to_string());
    let link = s.remote_url.clone();

    view! {
        <div class="card detail">
            <div class=format!("card-title detail-head {}", ci_tint(s))>
                <span class="dt-title">
                    {name}<span class="dim">{format!("  ·  {branch}")}</span>
                </span>
                // At-a-glance status, inline in the header: sync + CI (a running
                // build animates via the spinner, same as the table).
                <span class="dt-badges">
                    {sync_cell(s)}
                    {ci_cell(s, false)}
                </span>
            </div>
            <div class="scroll">
                <div class="facts">
                    <div class="fact">
                        <span class="fi">{edit_icon()}</span>
                        <span class="fl">"Changes"</span>
                        <span class="fv">{changes}</span>
                    </div>
                    <div class="fact">
                        <span class="fi">{stash_icon()}</span>
                        <span class="fl">"Stash"</span>
                        <span class="fv">{stash}</span>
                    </div>
                    <div class="fact">
                        <span class="fi">{pr_icon()}</span>
                        <span class="fl">"PRs"</span>
                        <span class="fv">{prs}</span>
                    </div>
                    <div class="fact">
                        <span class="fi">{clock_icon()}</span>
                        <span class="fl">"Last"</span>
                        <span class="fv">{last}</span>
                    </div>
                    <div class="fact">
                        <span class="fi">{activity_icon()}</span>
                        <span class="fl">"Activity"</span>
                        <span class="fv">{sparkline(&s.activity)}</span>
                    </div>
                </div>
                {attention_block}
                {move || rich_block(detail.get(), now)}
                {link.map(|url| {
                    let shown = url.clone();
                    view! {
                        <details class="sec" open>
                            <summary class="sec-h">"Remote source"</summary>
                            <div class="link">
                                <a href=url target="_blank" rel="noreferrer">{shown}</a>
                            </div>
                        </details>
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
            <details class="sec" open>
                <summary class="sec-h">"Recent commits"</summary>
                <ul class="rows">{rows}</ul>
            </details>
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
                    let draft = p
                        .draft
                        .then(|| view! { <span class="badge">"draft"</span> });
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
                <details class="sec" open>
                    <summary class="sec-h">"Open PRs"</summary>
                    <ul class="rows">{rows}</ul>
                </details>
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
                <details class="sec" open>
                    <summary class="sec-h">"Top contributors"</summary>
                    <ul class="rows">{rows}</ul>
                </details>
            }
        });
        view! {
            <details class="sec" open>
                <summary class="sec-h">"Remote"</summary>
                <div class="stats">{stats}</div>
            </details>
            {prs}
            {contribs}
        }
    });

    view! { <div class="rich">{commits}{remote}</div> }
}

/// The working-tree half of the fused "Needs attention" section: the changed-file
/// list and the collapsible, size-capped diff. Driven by the async drill-in, so
/// it's empty until that loads (the reasons above it come from the snapshot and
/// render immediately). `Idle`/`Loading` render nothing here.
fn working_tree_block(state: DetailState) -> AnyView {
    let DetailState::Loaded(d) = state else {
        return ().into_any();
    };
    let files = (!d.local.changed_files.is_empty()).then(|| {
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
        view! { <ul class="rows wtree">{rows}</ul> }
    });

    // The capped working-tree patch, in a collapsed <details> so it doesn't crowd
    // the drawer. Lines are colored by their diff prefix (+/-/@/file header). The
    // patch is byte-capped server-side (20 KB), so line count stays bounded.
    let patch = d
        .changes
        .patch
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(|p| {
            let lines = p
                .lines()
                .map(|ln| {
                    let cls = match ln.as_bytes().first() {
                        Some(b'+') if !ln.starts_with("+++") => "add",
                        Some(b'-') if !ln.starts_with("---") => "del",
                        Some(b'@') => "hunk",
                        Some(b'd') if ln.starts_with("diff ") => "fileh",
                        _ => "ctx",
                    };
                    view! { <span class=cls>{format!("{ln}\n")}</span> }
                })
                .collect::<Vec<_>>();
            let trunc = d
                .changes
                .truncated
                .then(|| view! { <div class="trunc">"diff truncated at 20 KB"</div> });
            view! {
                <details class="diff">
                    <summary>"Working-tree diff"</summary>
                    <pre class="patch">{lines}</pre>
                    {trunc}
                </details>
            }
        });

    view! { {files}{patch} }.into_any()
}

/// The severity → bucket index used by the fleet-pulse breakdown.
fn sev_index(s: Severity) -> usize {
    match s {
        Severity::Ok => 0,
        Severity::Info => 1,
        Severity::Notice => 2,
        Severity::Warn => 3,
        Severity::Risk => 4,
    }
}

/// The detail aside when nothing is selected. Onboarding now lives in the tour,
/// so instead of tips this shows a live "fleet pulse" — a combined activity
/// sparkline and an attention breakdown, drawn from the loaded fleet — plus a
/// button to replay the tour.
fn hint_panel(repos: &[RepoSnapshot], now: i64, tour: RwSignal<Option<usize>>) -> impl IntoView {
    // Element-wise sum of every repo's weekly activity into one fleet sparkline.
    let weeks = repos.iter().map(|r| r.activity.len()).max().unwrap_or(0);
    let mut agg = vec![0u32; weeks];
    for r in repos {
        for (i, &v) in r.activity.iter().enumerate() {
            if i < weeks {
                agg[i] += v as u32;
            }
        }
    }
    let peak = agg.iter().copied().max().unwrap_or(1).max(1);
    let total_commits: u32 = agg.iter().sum();
    let repo_count = repos.len();
    let bars = agg
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let h = (v as f64 / peak as f64 * 100.0).max(4.0);
            let delay = i as f64 * 45.0;
            view! { <div class="pulse-bar" style=format!("height:{h}%;animation-delay:{delay}ms")></div> }
        })
        .collect::<Vec<_>>();

    // Attention breakdown by severity.
    let mut counts = [0usize; 5];
    for r in repos {
        counts[sev_index(assess(r, now).severity)] += 1;
    }
    let total = repo_count.max(1);
    let attention = counts[2] + counts[3] + counts[4];
    let calm = counts[0] + counts[1];
    let seg = |n: usize, cls: &'static str| {
        (n > 0).then(|| {
            let w = n as f64 / total as f64 * 100.0;
            view! { <div class=format!("seg {cls}") style=format!("width:{w}%")></div> }
        })
    };

    view! {
        <div class="card">
            <div class="card-title">"Detail"</div>
            <div class="scroll hint">
                <p class="hint-lead">
                    "Pick a repository to inspect it — or take in the fleet's pulse."
                </p>
                <div class="pulse-card">
                    <div class="pulse-h">"Fleet activity · last 12 weeks"</div>
                    <div class="pulse-spark">{bars}</div>
                    <div class="pulse-cap">
                        {format!("{total_commits} commits across {repo_count} repos")}
                    </div>
                </div>
                <div class="pulse-card">
                    <div class="pulse-h">"Attention"</div>
                    <div class="pulse-stack">
                        {seg(counts[4], "risk")} {seg(counts[3], "warn")}
                        {seg(counts[2], "notice")} {seg(calm, "calm")}
                    </div>
                    <div class="pulse-legend">
                        <span class="need">{format!("{attention} need you")}</span>
                        <span class="calm">{format!("{calm} calm")}</span>
                    </div>
                </div>
                <button class="tour-start" on:click=move |_| tour.set(Some(0))>
                    "↻ Replay the tour"
                </button>
            </div>
        </div>
    }
}

// ── Guided tour ──────────────────────────────────────────────────────────────

/// A demo side-effect a tour step performs when entered. All are safe — they
/// only move UI state (selection, a preview modal), never a real git action.
#[derive(Clone, Copy, PartialEq)]
enum TourAct {
    None,
    /// Open the first visible repo's detail.
    Select,
    /// Check the first visible repo.
    Mark,
    /// Pop the confirm/preview modal with sample targets.
    ShowConfirm,
    /// Pop the results modal with a sample outcome.
    ShowResults,
    /// Clear any selection / marks / modals.
    Clear,
}

/// localStorage key marking that this browser has seen the intro tour.
const TOUR_FLAG: &str = "cohors_toured";

/// Whether the intro tour has already run in this browser.
fn tour_seen() -> bool {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(TOUR_FLAG).ok().flatten())
        .is_some()
}

/// Remember that the tour ran, so it won't auto-open again.
fn set_tour_seen() {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(TOUR_FLAG, "1");
    }
}

/// One step of the guided tour: an optional element to spotlight (a CSS
/// selector), the copy, and a demo action.
struct TourStep {
    target: Option<&'static str>,
    title: &'static str,
    body: &'static str,
    act: TourAct,
}

const TOUR: &[TourStep] = &[
    TourStep {
        target: None,
        title: "Welcome to cohors",
        body: "Every git repo you have, on one screen — their status and what needs you. Here's a quick tour. It only looks around and stages a preview; it never runs git.",
        act: TourAct::Clear,
    },
    TourStep {
        target: Some(".ov-top .fleet-name"),
        title: "This is your fleet",
        body: "cohors scanned this folder. Every repository below lives somewhere under it — no setup, no config to point at each one.",
        act: TourAct::None,
    },
    TourStep {
        target: Some(".overview .conn"),
        title: "A live local scan",
        body: "It reads your repos straight from disk. Made changes in a terminal? Hit Rescan and the fleet catches up.",
        act: TourAct::None,
    },
    TourStep {
        target: Some(".overview .attention"),
        title: "Your fleet's pulse",
        body: "The at-a-glance count: how many repositories you have, and how many need attention, are dirty, or hold stashes. Watch it drop as you deal with things.",
        act: TourAct::None,
    },
    TourStep {
        target: Some("tbody tr:nth-child(1)"),
        title: "One row, one repo",
        body: "Read a row left to right: its name and branch, sync with the remote, stashes, PRs, CI, last commit, and activity — ending with the Status column, the headline that says exactly why it needs you.",
        act: TourAct::None,
    },
    TourStep {
        target: Some(".repos-head .filter"),
        title: "Filter by name",
        body: "Start typing to narrow the fleet — useful once you're tracking dozens or hundreds of repos.",
        act: TourAct::None,
    },
    TourStep {
        target: Some(".repos-head .toggle"),
        title: "Focus on what matters",
        body: "Flip \"needs attention\" to hide the calm repos and see only the ones that want something from you.",
        act: TourAct::None,
    },
    TourStep {
        target: Some(".fleet thead th.sorted"),
        title: "Sort any column",
        body: "Click a column header to sort — by attention, recency, name, or how far out of sync. The active column shows an arrow.",
        act: TourAct::None,
    },
    TourStep {
        target: Some("tbody tr:nth-child(1) td.mk"),
        title: "Pick a subset",
        body: "Tick rows to build a selection — we've checked the first one for you. Now you can act on just those repos.",
        act: TourAct::Mark,
    },
    TourStep {
        target: Some(".act-controls"),
        title: "Act on the fleet",
        body: "\"act on selected\" targets your ticked repos; \"act on all\" targets everything. Either way you pick a verb — fetch, pull, push, commit, stash, or a shell command.",
        act: TourAct::None,
    },
    TourStep {
        target: Some(".modal"),
        title: "Preview before it runs",
        body: "Nothing happens by surprise. First, cohors shows you the exact repositories an action will touch — like this — and waits for you to confirm.",
        act: TourAct::ShowConfirm,
    },
    TourStep {
        target: Some(".modal.results"),
        title: "See what happened",
        body: "After it runs, you get a per-repo report: what succeeded, what was skipped and why, and any command output.",
        act: TourAct::ShowResults,
    },
    TourStep {
        target: Some("tbody tr:nth-child(1) td.act"),
        title: "Or act on one repo",
        body: "Prefer one at a time? Every row has its own Git menu with the same actions, scoped to that single repository.",
        act: TourAct::Clear,
    },
    TourStep {
        target: Some(".side"),
        title: "Look closer",
        body: "Click a repo to open it — its recent commits, branches, working-tree changes, and pull requests all fill in right here in the Detail panel.",
        act: TourAct::Select,
    },
    TourStep {
        target: None,
        title: "That's the tour",
        body: "Reads are always on. Relaunch with --allow-writes to enable the actions you just saw. Happy shipping — you can replay this tour anytime from the Detail panel.",
        act: TourAct::Clear,
    },
];

/// The on-screen rect (left, top, width, height) of the first element matching
/// `sel`, scrolling it into view first. `None` if it isn't in the DOM.
fn element_rect(sel: &str) -> Option<(f64, f64, f64, f64)> {
    let doc = web_sys::window()?.document()?;
    let el = doc.query_selector(sel).ok().flatten()?;
    el.scroll_into_view();
    let r = el.get_bounding_client_rect();
    Some((r.left(), r.top(), r.width(), r.height()))
}

/// The repo id of the first visible fleet row (its `data-repo` attribute), so a
/// tour step can act on exactly the row it highlights.
fn first_visible_repo() -> Option<String> {
    let doc = web_sys::window()?.document()?;
    let tr = doc.query_selector("tbody tr[data-repo]").ok().flatten()?;
    tr.get_attribute("data-repo")
}

/// The viewport size (width, height), with sane fallbacks.
fn viewport() -> (f64, f64) {
    let win = web_sys::window();
    let w = win
        .as_ref()
        .and_then(|w| w.inner_width().ok())
        .and_then(|j| j.as_f64())
        .unwrap_or(1200.0);
    let h = win
        .as_ref()
        .and_then(|w| w.inner_height().ok())
        .and_then(|j| j.as_f64())
        .unwrap_or(800.0);
    (w, h)
}

/// The tour card's on-screen size (falling back to an estimate before it exists).
fn card_size() -> (f64, f64) {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".tour-card").ok().flatten())
        .map(|el| {
            let r = el.get_bounding_client_rect();
            (r.width(), r.height())
        })
        .filter(|&(w, h)| w > 0.0 && h > 0.0)
        .unwrap_or((360.0, 200.0))
}

/// Where the floating card should sit for the current spotlight rect: near the
/// target (below, or above / bottom when there's no room), or dead-centre when
/// the step has no target. Returned as `(left, top)` pixels so it can be
/// animated between steps.
fn card_position(rect: Option<(f64, f64, f64, f64)>) -> (f64, f64) {
    let (vw, vh) = viewport();
    let (cw, ch) = card_size();
    match rect {
        None => ((vw - cw) / 2.0, (vh - ch) / 2.0),
        Some((x, y, _w, h)) => {
            let gap = 16.0;
            let left = x.min(vw - cw - 16.0).max(16.0);
            let top = if h > vh * 0.6 {
                // A tall target (e.g. the detail panel): pin the card near the bottom.
                vh - ch - 24.0
            } else if y + h + gap + ch < vh - 8.0 {
                y + h + gap // below the target
            } else {
                (y - ch - gap).max(16.0) // above it
            };
            (left, top)
        }
    }
}

/// The guided-tour overlay: a spotlight over the current step's element and a
/// floating card that slides to each highlight. The card element persists across
/// steps (so its position animates); only its content and coordinates change.
/// Runs on the demo/live fleet and only ever selects or marks — never a real git
/// action — so it's safe anywhere.
fn tour_overlay(
    tour: RwSignal<Option<usize>>,
    tour_rect: RwSignal<Option<(f64, f64, f64, f64)>>,
    tour_card_pos: RwSignal<Option<(f64, f64)>>,
    selected: RwSignal<Option<String>>,
    marked: RwSignal<HashSet<String>>,
) -> impl IntoView {
    // Only flips on start/end, so the tour DOM persists across steps.
    let touring = Memo::new(move |_| tour.get().is_some());
    let end = move || {
        set_tour_seen();
        selected.set(None);
        marked.update(|m| m.clear());
        tour.set(None);
    };
    let idx = move || tour.get().unwrap_or(0);
    let last = move || idx() + 1 >= TOUR.len();

    // The spotlight box; a no-target step shrinks it to a point (full dim).
    let spot_style = move || match tour_rect.get() {
        Some((x, y, w, h)) => format!(
            "left:{}px;top:{}px;width:{}px;height:{}px",
            x - 6.0,
            y - 6.0,
            w + 12.0,
            h + 12.0
        ),
        None => "left:50%;top:45%;width:0px;height:0px".to_string(),
    };
    let card_style = move || {
        let (l, t) = tour_card_pos.get().unwrap_or_else(|| card_position(None));
        format!("left:{l}px;top:{t}px")
    };

    view! {
        <Show when=move || touring.get() fallback=|| ()>
            <div class="tour">
                <div class="tour-spot" style=spot_style></div>
                <div class="tour-card" style=card_style>
                    <div class="tour-step">{move || format!("{} / {}", idx() + 1, TOUR.len())}</div>
                    <h3>{move || TOUR[idx()].title}</h3>
                    <p>{move || TOUR[idx()].body}</p>
                    <div class="tour-actions">
                        <button class="ghost" on:click=move |_| end()>
                            "Skip"
                        </button>
                        <span class="tour-nav">
                            <Show when=move || { idx() > 0 } fallback=|| ()>
                                <button
                                    class="ghost"
                                    on:click=move |_| {
                                        tour.update(|t| {
                                            if let Some(i) = t
                                                && *i > 0
                                            {
                                                *i -= 1;
                                            }
                                        })
                                    }
                                >
                                    "Back"
                                </button>
                            </Show>
                            <button
                                class="confirm"
                                on:click=move |_| {
                                    if last() {
                                        end()
                                    } else {
                                        tour.update(|t| {
                                            if let Some(i) = t {
                                                *i += 1;
                                            }
                                        })
                                    }
                                }
                            >
                                {move || if last() { "Done" } else { "Next" }}
                            </button>
                        </span>
                    </div>
                </div>
            </div>
        </Show>
    }
}

// ── Text variants of the cells, for the detail facts list ────────────────────

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

fn prs_text(s: &RepoSnapshot) -> String {
    match &s.remote {
        None => "local".to_string(),
        Some(r) if r.open_prs == 0 => "none".to_string(),
        Some(r) => format!("{} open", r.open_prs),
    }
}
