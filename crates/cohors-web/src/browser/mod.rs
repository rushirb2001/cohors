//! cohors web — the dashboard in the browser.
//!
//! The page is a renderer. `cohors web` (the native server) scans the
//! repositories, builds the shared [`cohors_core::Overview`] and serves it as
//! JSON; this module draws it. It decides layout, never numbers: every count
//! and every judgment on screen comes from the server's model.
//!
//! How it is organised:
//!
//! - this file: the shared state ([`Ctx`]), loading and refreshing, the route
//!   in the URL hash, and the page shell;
//! - [`sidebar`]: navigation, saved views, roots, rescan, theme;
//! - [`overview`], [`repos`], [`tasks`], [`standup`]: the four pages;
//! - [`drawer`]: one repository in full, sliding over the page;
//! - [`actions`]: running fetch / pull / push / commit / stash / run, with a
//!   preview first;
//! - [`ui`]: icons, marks and other small shared pieces.

mod actions;
mod drawer;
mod overview;
mod repos;
mod sidebar;
mod standup;
mod tasks;
mod ui;

use std::collections::HashSet;

use cohors_core::launchpad::{Board, LaunchpadEntry};
use cohors_core::{Overview, RepoLine, RepoSnapshot};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::api;
use repos::{RepoView, Sort};

/// How often the page asks the server for a fresh scan, in milliseconds.
const POLL_MS: u32 = 10_000;
/// A scan identical to the last one still replaces it after this many seconds,
/// so relative ages ("3h ago") do not go stale on a quiet fleet.
const REDRAW_SECS: i64 = 300;
/// How long a toast stays up, in milliseconds.
const TOAST_MS: u32 = 2_600;

/// Mount the app on the page body (called from the wasm `main`).
pub fn mount() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

/// The four pages.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Overview,
    Repos,
    Tasks,
    Standup,
}

impl Page {
    /// The page's name in the URL hash: `#/repositories`.
    fn slug(self) -> &'static str {
        match self {
            Page::Overview => "overview",
            Page::Repos => "repositories",
            Page::Tasks => "tasks",
            Page::Standup => "standup",
        }
    }

    /// A link to the page, for `<a href>`.
    pub fn href(self) -> String {
        format!("#/{}", self.slug())
    }
}

/// Whether the first scan has arrived.
#[derive(Clone, PartialEq, Eq)]
pub enum Load {
    Loading,
    Ready,
    /// The first scan failed; the string says what went wrong.
    Failed(String),
}

/// Everything the page knows, as reactive signals. `Ctx` is `Copy` (a signal
/// is just a handle), so it is passed by value to every view function.
#[derive(Clone, Copy)]
pub struct Ctx {
    // ── What the server said ──
    pub load: RwSignal<Load>,
    pub meta: RwSignal<api::Meta>,
    /// The shared model. Starts as the overview of an empty fleet so there is
    /// always something valid to read.
    pub overview: RwSignal<Overview>,
    /// The raw snapshots, for the few facts the overview does not carry (the
    /// remote URL, CI, the newest tag).
    pub snapshots: RwSignal<Vec<RepoSnapshot>>,
    pub launchpad: RwSignal<api::Launchpad>,
    /// Why the launchpad could not be read, when it could not.
    pub launchpad_error: RwSignal<Option<String>>,
    /// Why the latest refresh failed, when it did (the last good scan stays up).
    pub refresh_error: RwSignal<Option<String>>,
    /// When the server last answered a scan (Unix seconds).
    pub scanned_at: RwSignal<Option<i64>>,
    pub scanning: RwSignal<bool>,

    // ── Where the user is ──
    pub page: RwSignal<Page>,
    /// Which repositories the table shows.
    pub view: RwSignal<RepoView>,
    /// The text in the filter box.
    pub filter: RwSignal<String>,
    /// The column the table is sorted by; `None` keeps the model's order
    /// (most urgent first).
    pub sort: RwSignal<Option<Sort>>,
    /// The ticked repositories, by id.
    pub marked: RwSignal<HashSet<String>>,
    /// The repository open in the drawer, by id.
    pub selected: RwSignal<Option<String>>,
    /// Whether the sidebar is open on a narrow screen.
    pub nav_open: RwSignal<bool>,
    pub dark: RwSignal<bool>,

    // ── Passing things ──
    pub dialog: RwSignal<Option<actions::Dialog>>,
    /// The toast and a counter that tells one toast from the next.
    pub toast: RwSignal<Option<(u32, String)>>,
    /// Bumped after an action runs, so the open drawer reloads its detail.
    pub detail_epoch: RwSignal<u32>,
}

impl Ctx {
    fn new() -> Self {
        Ctx {
            load: RwSignal::new(Load::Loading),
            meta: RwSignal::new(api::Meta::default()),
            overview: RwSignal::new(cohors_core::overview(&[], &[], &Board::default(), 0)),
            snapshots: RwSignal::new(Vec::new()),
            launchpad: RwSignal::new(api::Launchpad::default()),
            launchpad_error: RwSignal::new(None),
            refresh_error: RwSignal::new(None),
            scanned_at: RwSignal::new(None),
            scanning: RwSignal::new(false),
            page: RwSignal::new(Page::Overview),
            view: RwSignal::new(RepoView::All),
            filter: RwSignal::new(String::new()),
            sort: RwSignal::new(None),
            marked: RwSignal::new(HashSet::new()),
            selected: RwSignal::new(None),
            nav_open: RwSignal::new(false),
            dark: RwSignal::new(sidebar::theme_is_dark()),
            dialog: RwSignal::new(None),
            toast: RwSignal::new(None),
            detail_epoch: RwSignal::new(0),
        }
    }

    // ── Lookups ──

    /// The overview's line for one repository.
    pub fn line(self, id: &str) -> Option<RepoLine> {
        self.overview
            .with(|o| o.repos.iter().find(|r| r.id == id).cloned())
    }

    /// The raw snapshot of one repository.
    pub fn snapshot(self, id: &str) -> Option<RepoSnapshot> {
        self.snapshots
            .with(|all| all.iter().find(|s| s.id.0 == id).cloned())
    }

    /// One repository's launchpad entry. Matched by id only: two repositories
    /// can share a name, and a guess would show one's tasks under the other.
    pub fn entry(self, id: &str) -> Option<LaunchpadEntry> {
        self.launchpad
            .with(|lp| lp.entries.iter().find(|e| e.id == id).cloned())
    }

    /// Whether the server will run `verb`. `run` has its own gate.
    pub fn allowed(self, verb: &str) -> bool {
        self.meta.with(|m| {
            if verb == "run" {
                m.allow_run
            } else {
                m.allow_writes
            }
        })
    }

    // ── Navigation ──

    /// Go to a page.
    pub fn go(self, page: Page) {
        self.show(page);
        set_hash(page.slug());
    }

    /// Open the Repositories page showing `view`.
    pub fn open_view(self, view: RepoView) {
        let hash = format!("{}/{}", Page::Repos.slug(), view.slug());
        set_if_changed(self.view, view);
        self.show(Page::Repos);
        set_hash(&hash);
    }

    fn show(self, page: Page) {
        if self.page.get_untracked() != page {
            // The Overview's table is headed "All repositories": arrive there
            // with no saved view still applied from the Repositories page.
            if page == Page::Overview {
                set_if_changed(self.view, RepoView::All);
            }
            self.page.set(page);
            scroll_to_top();
        }
        self.nav_open.set(false);
    }

    /// Read the URL hash and show what it names. Runs at start and whenever
    /// the hash changes (a sidebar link, or the browser's back button).
    fn apply_route(self) {
        let hash = web_sys::window()
            .and_then(|w| w.location().hash().ok())
            .unwrap_or_default();
        let route = hash.trim_start_matches('#').trim_start_matches('/');
        let (head, rest) = route.split_once('/').unwrap_or((route, ""));
        let page = match head {
            "repositories" => Page::Repos,
            "tasks" => Page::Tasks,
            "standup" => Page::Standup,
            _ => Page::Overview,
        };
        if page == Page::Repos {
            let current = self.view.get_untracked();
            let view = match RepoView::from_slug(rest) {
                Some(view) => view,
                // The list a "Do next" button opened exists only in memory:
                // keep it while it is there, otherwise show everything.
                None if matches!(current, RepoView::Custom { .. }) && rest == "selection" => {
                    current
                }
                None => RepoView::All,
            };
            set_if_changed(self.view, view);
        }
        self.show(page);
    }

    // ── Data ──

    /// Ask the server for a fresh scan and replace whatever changed.
    pub fn refresh(self) {
        if self.scanning.get_untracked() {
            return;
        }
        self.scanning.set(true);
        spawn_local(async move {
            if let Ok(meta) = api::fetch_meta().await {
                set_if_changed(self.meta, meta);
            }

            match api::fetch_overview().await {
                Ok(fresh) => {
                    let scanned = fresh.now;
                    // Redraw only when something other than the clock moved,
                    // or the clock moved far enough to change the ages shown.
                    let redraw = self.overview.with_untracked(|old| {
                        let mut same_clock = fresh.clone();
                        same_clock.now = old.now;
                        same_clock != *old || fresh.now - old.now > REDRAW_SECS
                    });
                    if redraw {
                        self.overview.set(fresh);
                    }
                    self.scanned_at.set(Some(scanned));
                    set_if_changed(self.refresh_error, None);
                    set_if_changed(self.load, Load::Ready);
                }
                Err(error) => {
                    if self.load.get_untracked() == Load::Ready {
                        // Keep the last good scan on screen and say so.
                        self.refresh_error.set(Some(error));
                    } else {
                        self.load.set(Load::Failed(error));
                    }
                    self.scanning.set(false);
                    return;
                }
            }

            match api::fetch_launchpad().await {
                Ok(launchpad) => {
                    set_if_changed(self.launchpad, launchpad);
                    set_if_changed(self.launchpad_error, None);
                }
                Err(error) => set_if_changed(self.launchpad_error, Some(error)),
            }

            // Snapshots: the plain scan first (fast), then the same scan with
            // GitHub CI and pull requests added, which can take a moment.
            if self.snapshots.with_untracked(Vec::is_empty)
                && let Ok(plain) = api::fetch_repos(false).await
            {
                self.snapshots.set(plain);
            }
            if let Ok(enriched) = api::fetch_repos(true).await {
                set_if_changed(self.snapshots, enriched);
            }
            self.scanning.set(false);
        });
    }

    /// Show a short message at the bottom of the screen.
    pub fn say(self, text: String) {
        let id = self.toast.get_untracked().map_or(0, |(id, _)| id + 1);
        self.toast.set(Some((id, text)));
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(TOAST_MS).await;
            // Clear only the toast this call put up; a newer one has its own timer.
            if self
                .toast
                .get_untracked()
                .is_some_and(|(current, _)| current == id)
            {
                self.toast.set(None);
            }
        });
    }
}

/// Set a signal only when the value differs, so nothing redraws for no reason.
fn set_if_changed<T: PartialEq + Send + Sync + 'static>(signal: RwSignal<T>, value: T) {
    if signal.with_untracked(|old| *old != value) {
        signal.set(value);
    }
}

fn set_hash(route: &str) {
    if let Some(window) = web_sys::window() {
        let _ = window.location().set_hash(&format!("/{route}"));
    }
}

fn scroll_to_top() {
    if let Some(window) = web_sys::window() {
        window.scroll_to_with_x_and_y(0.0, 0.0);
    }
}

/// True when the tab is in the background, where polling would be wasted work.
fn tab_hidden() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .is_some_and(|d| d.hidden())
}

#[component]
fn App() -> impl IntoView {
    let cx = Ctx::new();
    cx.apply_route();
    cx.refresh();

    // Keep the page current: re-scan on a timer, but not behind the user's
    // back while a dialog is asking them something or the tab is hidden.
    spawn_local(async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(POLL_MS).await;
            if cx.dialog.get_untracked().is_none() && !tab_hidden() {
                cx.refresh();
            }
        }
    });

    // The handles are not kept: the app lives as long as the page, so these
    // two listeners are never removed.
    let _ = window_event_listener(leptos::ev::hashchange, move |_| cx.apply_route());
    let _ = window_event_listener(leptos::ev::keydown, move |ev| {
        if ev.key() != "Escape" {
            return;
        }
        // Escape closes the topmost thing: a dialog, else the drawer.
        if cx.dialog.get_untracked().is_some() {
            actions::dismiss(cx);
        } else if cx.selected.get_untracked().is_some() {
            cx.selected.set(None);
        }
    });

    view! {
        <div class="shell">
            {sidebar::sidebar(cx)}
            <main>
                {move || match cx.load.get() {
                    Load::Loading => loading().into_any(),
                    Load::Failed(error) => failed(cx, error).into_any(),
                    Load::Ready => ready(cx).into_any(),
                }}
            </main>
        </div>
        {drawer::drawer(cx)}
        {actions::dialogs(cx)}
        {move || {
            cx.toast
                .get()
                .map(|(_, text)| {
                    view! {
                        <div class="toast" role="status">
                            {text}
                        </div>
                    }
                })
        }}
    }
}

/// The first scan is still running.
fn loading() -> impl IntoView {
    view! {
        <div class="state-page" role="status">
            <h1>"Scanning your repositories"</h1>
            <p>
                <span class="spin"></span>
                "Reading the state of every repository under the configured roots."
            </p>
        </div>
    }
}

/// The first scan failed: say what failed and offer to try again. Nothing is
/// shown in its place.
fn failed(cx: Ctx, error: String) -> impl IntoView {
    view! {
        <div class="state-page" role="alert">
            <h1>"Could not load your repositories"</h1>
            <p>
                "The page asked the cohors server for a scan and did not get one. "
                "Check that `cohors web` is still running in your terminal."
            </p>
            <pre>{error}</pre>
            <button
                class="btn primary"
                on:click=move |_| {
                    cx.load.set(Load::Loading);
                    cx.refresh();
                }
            >
                {ui::icon(ui::Icon::Refresh)}
                <span>"Retry"</span>
            </button>
        </div>
    }
}

/// The scan arrived: show the current page, or say that it found nothing.
fn ready(cx: Ctx) -> impl IntoView {
    // A memo, so the page below is rebuilt only when this answer flips, not
    // every time a scan replaces the overview.
    let empty = Memo::new(move |_| cx.overview.with(|o| o.figures.repos == 0));
    view! {
        {move || {
            cx.refresh_error
                .get()
                .map(|error| {
                    let since = cx
                        .scanned_at
                        .get()
                        .map(|t| format!(" Showing the scan from {}.", ui::local_time(t)))
                        .unwrap_or_default();
                    view! {
                        <div class="notice" role="alert">
                            <span>{format!("Could not refresh: {error}.{since}")}</span>
                            <button class="btn small" on:click=move |_| cx.refresh()>
                                "Retry"
                            </button>
                        </div>
                    }
                })
        }}
        {move || {
            if empty.get() {
                return no_repositories(cx).into_any();
            }
            match cx.page.get() {
                Page::Overview => overview::page(cx).into_any(),
                Page::Repos => repos::page(cx).into_any(),
                Page::Tasks => tasks::page(cx).into_any(),
                Page::Standup => standup::page(cx).into_any(),
            }
        }}
    }
}

/// The scan worked but found no repositories.
fn no_repositories(cx: Ctx) -> impl IntoView {
    let roots = cx.meta.with(|m| m.roots.join(", "));
    let scanned = if roots.is_empty() {
        "cohors scanned its configured roots and found no git repositories.".to_string()
    } else {
        format!("cohors scanned {roots} and found no git repositories.")
    };
    view! {
        <div class="state-page">
            <h1>"No repositories found"</h1>
            <p>{scanned}</p>
            <p>"Point it at the folder that holds your repositories:"</p>
            {ui::command(cx, "cohors web --root <folder>".to_string())}
            <button class="btn" on:click=move |_| cx.refresh()>
                {ui::icon(ui::Icon::Refresh)}
                <span>"Scan again"</span>
            </button>
        </div>
    }
}
