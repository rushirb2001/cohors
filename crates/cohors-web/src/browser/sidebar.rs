//! The sidebar: the brand, the four pages, the saved views with live counts,
//! the roots being scanned, when they were last scanned, and the theme.
//!
//! On a narrow screen it collapses to a top bar with a menu button.

use leptos::prelude::*;

use super::repos::RepoView;
use super::ui::{self, Icon};
use super::{Ctx, Page};

/// Where the theme choice is remembered between visits.
const THEME_KEY: &str = "cohors_theme";

/// Whether the page is currently dark. `index.html` sets `data-theme` on the
/// root element before first paint (a saved choice, else the system setting).
pub fn theme_is_dark() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
        .and_then(|root| root.get_attribute("data-theme"))
        .is_some_and(|theme| theme == "dark")
}

/// Switch the theme and remember the choice.
fn set_theme(dark: bool) {
    let name = if dark { "dark" } else { "light" };
    let Some(window) = web_sys::window() else {
        return;
    };
    if let Some(root) = window.document().and_then(|d| d.document_element()) {
        let _ = root.set_attribute("data-theme", name);
    }
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.set_item(THEME_KEY, name);
    }
}

pub fn sidebar(cx: Ctx) -> impl IntoView {
    // True from a click on "Scan again" until that scan lands. The background
    // poll does not set it, so the button does not flicker every few seconds.
    let asked = RwSignal::new(false);
    Effect::new(move |_| {
        if !cx.scanning.get() {
            asked.set(false);
        }
    });

    let pages = view! {
        <ul>
            {page_link(
                Page::Overview,
                Icon::Grid,
                "Overview",
                move || None,
                move || cx.page.get() == Page::Overview,
            )}
            {page_link(
                Page::Repos,
                Icon::Folder,
                "Repositories",
                move || Some(cx.overview.with(|o| o.figures.repos)),
                // A saved view highlights its own link instead.
                move || {
                    cx.page.get() == Page::Repos && !cx.view.with(|v| RepoView::SAVED.contains(v))
                },
            )}
            {page_link(
                Page::Tasks,
                Icon::Check,
                "Tasks",
                move || Some(cx.overview.with(|o| o.figures.open_tasks)),
                move || cx.page.get() == Page::Tasks,
            )}
            {page_link(
                Page::Standup,
                Icon::Calendar,
                "Standup",
                move || None,
                move || cx.page.get() == Page::Standup,
            )}
        </ul>
    };

    let saved = RepoView::SAVED
        .iter()
        .map(|view| {
            let view = view.clone();
            let counted = view.clone();
            let current = view.clone();
            let count = move || {
                cx.overview
                    .with(|o| o.repos.iter().filter(|r| counted.matches(r, o.now)).count())
            };
            view! {
                <li>
                    <a
                        class="nav-link"
                        href=format!("{}/{}", Page::Repos.href(), view.slug())
                        aria-current=move || {
                            (cx.page.get() == Page::Repos && cx.view.with(|v| *v == current))
                                .then_some("page")
                        }
                    >
                        {ui::icon(Icon::Bookmark)}
                        <span class="label">{view.label()}</span>
                        <span class="count">{count}</span>
                    </a>
                </li>
            }
        })
        .collect::<Vec<_>>();

    let roots = move || {
        let roots = cx.meta.with(|m| m.roots.clone());
        if roots.is_empty() {
            return view! { <li class="nav-root dim">"Not reported by the server"</li> }.into_any();
        }
        roots
            .into_iter()
            .map(|root| {
                let short = ui::short_path(&root);
                view! {
                    <li class="nav-root" title=root>
                        {ui::icon(Icon::Folder)}
                        <span>{short}</span>
                    </li>
                }
            })
            .collect::<Vec<_>>()
            .into_any()
    };

    // Sample data must never pass for a real fleet: say so, always in view.
    let demo = move || {
        cx.meta.with(|m| m.demo).then(|| {
            view! {
                <div class="nav-flag" role="note">
                    {ui::icon(Icon::Flask)}
                    <span>
                        <b>"Demo data"</b>
                        "A built-in sample fleet, not your repositories."
                    </span>
                </div>
            }
        })
    };

    let gates = move || {
        let (writes, run) = cx.meta.with(|m| (m.allow_writes, m.allow_run));
        let (text, tip) = match (writes, run) {
            (false, false) => (
                "Read-only",
                "Actions are previewed, not run. Start the server with cohors web --allow-writes to run them.",
            ),
            (true, false) => (
                "Write actions are on",
                "Fetch, pull, push, commit and stash can run. Shell commands need --allow-run.",
            ),
            (false, true) => (
                "Only run is on",
                "Shell commands can run. Fetch, pull, push, commit and stash need --allow-writes.",
            ),
            (true, true) => (
                "All actions are on",
                "Fetch, pull, push, commit, stash and shell commands can run.",
            ),
        };
        // A lock only while something is locked; a bolt once actions can run.
        let icon = if writes || run {
            Icon::Bolt
        } else {
            Icon::Lock
        };
        view! {
            <div class="row" title=tip>
                {ui::icon(icon)}
                <span>{text}</span>
            </div>
        }
    };

    let scanned = move || match cx.scanned_at.get() {
        Some(at) => format!("Scanned {}", ui::local_time(at)),
        None => "Not scanned yet".to_string(),
    };

    view! {
        <nav class="nav" class:open=move || cx.nav_open.get() aria-label="Main">
            <div class="nav-top">
                <a class="brand" href=Page::Overview.href()>
                    {ui::brand_mark()}
                    "cohors"
                </a>
                <button
                    class="icon-btn nav-toggle"
                    aria-label="Menu"
                    aria-expanded=move || if cx.nav_open.get() { "true" } else { "false" }
                    on:click=move |_| cx.nav_open.update(|open| *open = !*open)
                >
                    {ui::icon(Icon::Menu)}
                </button>
            </div>
            <div class="nav-body">
                {demo}
                {pages}
                <div>
                    <h2>"Saved views"</h2>
                    <ul>{saved}</ul>
                </div>
                <div>
                    <h2>"Roots"</h2>
                    <ul>{roots}</ul>
                </div>
                <span class="grow"></span>
                <div class="nav-foot">
                    {gates}
                    <div class="row">{ui::icon(Icon::Clock)} <span>{scanned}</span></div>
                    <button
                        class="btn small"
                        disabled=move || asked.get()
                        on:click=move |_| {
                            asked.set(true);
                            cx.refresh();
                        }
                    >
                        {ui::icon(Icon::Refresh)}
                        <span>{move || if asked.get() { "Scanning" } else { "Scan again" }}</span>
                    </button>
                    <button
                        class="btn small"
                        on:click=move |_| {
                            let dark = !cx.dark.get_untracked();
                            set_theme(dark);
                            cx.dark.set(dark);
                        }
                    >
                        {move || {
                            if cx.dark.get() {
                                view! {
                                    {ui::icon(Icon::Sun)}
                                    <span>"Light theme"</span>
                                }
                                    .into_any()
                            } else {
                                view! {
                                    {ui::icon(Icon::Moon)}
                                    <span>"Dark theme"</span>
                                }
                                    .into_any()
                            }
                        }}
                    </button>
                </div>
            </div>
        </nav>
    }
}

/// One of the four page links, with an optional count on the right.
fn page_link(
    page: Page,
    icon: Icon,
    label: &'static str,
    count: impl Fn() -> Option<usize> + Send + Sync + 'static,
    current: impl Fn() -> bool + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <li>
            <a
                class="nav-link"
                href=page.href()
                aria-current=move || current().then_some("page")
            >
                {ui::icon(icon)}
                <span class="label">{label}</span>
                {move || count().map(|n| view! { <span class="count">{n}</span> })}
            </a>
        </li>
    }
}
