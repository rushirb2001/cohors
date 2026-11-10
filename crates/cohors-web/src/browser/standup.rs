//! The Standup page: your own commits across every repository, today or over
//! the last seven days, grouped by repository, with a markdown copy to paste
//! into a status update.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use super::Ctx;
use super::ui::{self, Icon};
use crate::api;

/// The two windows the server offers, as it names them.
const TODAY: &str = "today";
const WEEK: &str = "week";
/// How many commits a repository shows before the rest are folded away.
const FOLD_AFTER: usize = 8;

#[derive(Clone)]
enum State {
    Loading,
    Loaded(Box<api::Standup>),
    Failed(String),
}

pub fn page(cx: Ctx) -> impl IntoView {
    let window = RwSignal::new(WEEK);
    let state = RwSignal::new(State::Loading);
    // The fleet's commit count for the week: when a scan changes it, the list
    // below is out of date and is read again.
    let fleet_commits = Memo::new(move |_| cx.overview.with(|o| o.figures.commits_week));

    // Read the standup when the page opens, when the window changes, and when
    // new commits show up. The effect returns the window it loaded, so the next
    // run can tell a change of window (show "loading") from a quiet refresh.
    Effect::new(move |previous: Option<&'static str>| {
        let wanted = window.get();
        fleet_commits.track();
        if previous != Some(wanted) {
            state.set(State::Loading);
        }
        spawn_local(async move {
            let result = api::fetch_standup(wanted).await;
            // Ignore an answer for a window the user has already left.
            if window.get_untracked() != wanted {
                return;
            }
            state.set(match result {
                Ok(standup) => State::Loaded(Box::new(standup)),
                Err(error) => State::Failed(error),
            });
        });
        wanted
    });

    let subtitle = move || match state.get() {
        State::Loaded(standup) => match &standup.author {
            Some(author) => {
                // The server's day starts at midnight UTC, which is rarely
                // midnight for the reader: say so beside the local times.
                let day = if window.get() == TODAY {
                    " The day starts at midnight UTC."
                } else {
                    ""
                };
                format!(
                    "Commits by {author}, {} to {}.{day}",
                    ui::local_time(standup.since),
                    ui::local_time(standup.until)
                )
            }
            None => "Git has no identity configured, so there is nothing to list.".to_string(),
        },
        State::Loading => "Reading your commits across every repository.".to_string(),
        State::Failed(_) => "The standup could not be read.".to_string(),
    };

    let segment = move |value: &'static str, label: &'static str| {
        view! {
            <button
                aria-pressed=move || if window.get() == value { "true" } else { "false" }
                on:click=move |_| window.set(value)
            >
                {label}
            </button>
        }
    };

    let copy = move |_| {
        if let State::Loaded(standup) = state.get_untracked() {
            ui::copy_text(cx, standup.markdown.clone(), "standup as markdown");
        }
    };

    let body = move || match state.get() {
        State::Loading => view! {
            <section>
                <div class="block">
                    <div class="empty" role="status">
                        <span class="spin"></span>
                        "Reading commits"
                    </div>
                </div>
            </section>
        }
        .into_any(),
        State::Failed(error) => view! {
            <div class="notice" role="alert">
                <span>{format!("Could not read the standup: {error}")}</span>
                // Setting the window to itself re-runs the effect above.
                <button
                    class="btn small"
                    on:click=move |_| {
                        state.set(State::Loading);
                        window.set(window.get_untracked());
                    }
                >
                    "Retry"
                </button>
            </div>
        }
        .into_any(),
        State::Loaded(standup) => loaded(cx, *standup, window).into_any(),
    };

    view! {
        <section class="head">
            <div>
                <h1>"Standup"</h1>
                <p>{subtitle}</p>
            </div>
            <div class="actions">
                <div class="seg" role="group" aria-label="Window">
                    {segment(TODAY, "Today")}
                    {segment(WEEK, "Last 7 days")}
                </div>
                <button
                    class="btn primary"
                    disabled=move || !matches!(state.get(), State::Loaded(_))
                    on:click=copy
                >
                    {ui::icon(Icon::Copy)}
                    <span>"Copy as markdown"</span>
                </button>
            </div>
        </section>
        {body}
    }
}

/// The id of the one repository the standup calls `name`, to open its drawer.
/// `None` when no repository or more than one has that name.
fn only_repo_named(cx: Ctx, name: &str) -> Option<String> {
    cx.overview.with_untracked(|o| {
        let mut matches = o.repos.iter().filter(|r| r.name == name);
        let first = matches.next()?;
        matches.next().is_none().then(|| first.id.clone())
    })
}

fn loaded(cx: Ctx, standup: api::Standup, window: RwSignal<&'static str>) -> impl IntoView {
    // No identity: the server cannot tell whose commits to list. Say how to fix it.
    let Some(author) = standup.author.clone() else {
        return view! {
            <section>
                <div class="block">
                    <div class="empty">
                        <p>
                            "cohors lists the commits written by you, and it reads who you are \
                             from git. Git has no user.email configured, so it cannot tell which \
                             commits are yours."
                        </p>
                        <p>"Set it, then start cohors web again:"</p>
                        {ui::command(
                            cx,
                            "git config --global user.email you@example.com".to_string(),
                        )}
                    </div>
                </div>
            </section>
        }
        .into_any();
    };

    let today = window.get_untracked() == TODAY;
    if standup.total == 0 {
        let nothing = if today {
            format!("No commits by {author} today.")
        } else {
            format!("No commits by {author} in the last 7 days.")
        };
        return view! {
            <section>
                <div class="block">
                    <div class="empty">
                        <p>{nothing}</p>
                        {today
                            .then(|| {
                                view! {
                                    <p>
                                        {format!(
                                            "The day is counted in UTC, so it began at {} your time.",
                                            ui::local_time(standup.since),
                                        )}
                                    </p>
                                    <p>
                                        <button class="btn small" on:click=move |_| window.set(WEEK)>
                                            "Show the last 7 days"
                                        </button>
                                    </p>
                                }
                            })}
                    </div>
                </div>
            </section>
        }
        .into_any();
    }

    let now = standup.until;
    let summary = format!(
        "{} in {}",
        ui::plural(standup.total, "commit", "commits"),
        ui::plural(standup.groups.len(), "repository", "repositories")
    );

    // The index: one bar per repository, most active first. A click scrolls
    // to that repository's commits further down the page.
    let most = standup.groups.first().map_or(1, |g| g.commits.len()).max(1);
    let index = standup
        .groups
        .iter()
        .enumerate()
        .map(|(position, group)| {
            let width = group.commits.len() as f32 / most as f32 * 100.0;
            view! {
                <button class="bar-row" on:click=move |_| scroll_to(&group_anchor(position))>
                    <span class="clip">{group.repo.clone()}</span>
                    <span class="track">
                        <i style=format!("width:{width:.1}%")></i>
                    </span>
                    <span class="num">{group.commits.len()}</span>
                </button>
            }
        })
        .collect::<Vec<_>>();

    let groups = standup
        .groups
        .into_iter()
        .enumerate()
        .map(|(position, group)| commit_group(cx, position, group, now))
        .collect::<Vec<_>>();

    let markdown = standup.markdown.clone();
    let copied = standup.markdown;
    view! {
        <section>
            <div class="sechead">
                <h2>{ui::icon(Icon::Folder)} "By repository"</h2>
                <span>{summary}</span>
            </div>
            <div class="block">{index}</div>
        </section>
        <section>
            <div class="sechead">
                <h2>{ui::icon(Icon::Commit)} "Commits"</h2>
                <span>"Most active repository first, newest commit first"</span>
            </div>
            {groups}
        </section>
        <section>
            <div class="sechead">
                <h2>{ui::icon(Icon::File)} "As markdown"</h2>
                <span>"What the copy button puts on the clipboard"</span>
                <span class="grow"></span>
                <button
                    class="btn small"
                    on:click=move |_| ui::copy_text(cx, copied.clone(), "standup as markdown")
                >
                    "Copy"
                </button>
            </div>
            <div class="block">
                <pre class="markdown">{markdown}</pre>
            </div>
        </section>
    }
    .into_any()
}

/// The element id of the `position`-th repository's block of commits.
fn group_anchor(position: usize) -> String {
    format!("standup-repo-{position}")
}

/// Scroll the element with this id into view.
fn scroll_to(id: &str) {
    let element = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(id));
    if let Some(element) = element {
        element.scroll_into_view();
    }
}

/// One repository's commits. A long list shows its newest commits and a
/// button for the rest, so one busy repository does not bury the others.
fn commit_group(cx: Ctx, position: usize, group: api::StandupGroup, now: i64) -> impl IntoView {
    let total = group.commits.len();
    let expanded = RwSignal::new(false);
    let title = match only_repo_named(cx, &group.repo) {
        Some(id) => view! {
            <button class="link" on:click=move |_| cx.selected.set(Some(id.clone()))>
                {group.repo}
            </button>
        }
        .into_any(),
        None => group.repo.into_any(),
    };
    let commits = group.commits;
    let rows = move || {
        let shown = if expanded.get() { total } else { FOLD_AFTER };
        commits
            .iter()
            .take(shown)
            .map(|commit| {
                view! {
                    <div class="commit-row">
                        <span class="sha">{commit.short_id.clone()}</span>
                        <span>{commit.summary.clone()}</span>
                        <span class="when" title=ui::local_time(commit.timestamp)>
                            {ui::ago(commit.timestamp, now)}
                        </span>
                    </div>
                }
            })
            .collect::<Vec<_>>()
    };
    let more = (total > FOLD_AFTER).then(|| {
        view! {
            <div class="blockfoot">
                <button
                    class="btn small"
                    on:click=move |_| expanded.update(|open| *open = !*open)
                >
                    {move || {
                        if expanded.get() {
                            format!("Show the newest {FOLD_AFTER} only")
                        } else {
                            format!("Show all {total} commits")
                        }
                    }}
                </button>
            </div>
        }
    });
    view! {
        <div class="block" id=group_anchor(position)>
            <div class="blockhead">
                <h3>{title}</h3>
                <span class="meta">{ui::plural(total, "commit", "commits")}</span>
            </div>
            {rows}
            {more}
        </div>
    }
}
