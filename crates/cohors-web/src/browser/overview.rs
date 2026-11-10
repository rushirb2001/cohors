//! The Overview page: what to do next, the key figures, the repositories
//! where work could be lost, open tasks, the week's commits, and every
//! repository. All of it is drawn straight from `cohors_core::Overview`.

use cohors_core::attention::STALE_FETCH_SECS;
use cohors_core::{ActionKind, KeyFigures, NextAction, RepoLine};
use leptos::prelude::*;

use super::actions::{self, ids_selector};
use super::repos::{self, RepoView};
use super::ui::{self, Icon};
use super::{Ctx, Page};

pub fn page(cx: Ctx) -> impl IntoView {
    view! {
        <section class="head">
            <div>
                <h1>"Overview"</h1>
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
                {selection_menu(cx)}
            </div>
        </section>

        <div class="lead">
            <div>
                <div class="sechead">
                    <h2>{ui::icon(Icon::Bolt)} "Do next"</h2>
                    <span>"Most urgent first"</span>
                </div>
                <div class="block">{do_next(cx)}</div>
            </div>
            <div>
                <div class="sechead">
                    <h2>{ui::icon(Icon::Grid)} "At a glance"</h2>
                </div>
                {figures(cx)}
            </div>
        </div>

        {at_risk(cx)}

        <section class="pair">
            {open_tasks(cx)}
            {commits(cx)}
        </section>

        <section>
            <div class="sechead">
                <h2>{ui::icon(Icon::Folder)} "All repositories"</h2>
            </div>
            {repos::repo_table(cx, false)}
        </section>
    }
}

/// "Act on selected": the six verbs for the repositories ticked in the table.
/// With nothing ticked it says how to tick something instead of doing nothing.
fn selection_menu(cx: Ctx) -> impl IntoView {
    let open = RwSignal::new(false);
    let count = move || cx.marked.with(|m| m.len());
    let label = move || match count() {
        0 => "Act on selected".to_string(),
        n => format!("Act on {n} selected"),
    };
    let toggle = move |_| {
        if cx.marked.with_untracked(|m| m.is_empty()) {
            cx.say(
                "Tick repositories in the table below first, then choose an action.".to_string(),
            );
        } else {
            open.update(|o| *o = !*o);
        }
    };
    let menu = move || {
        open.get().then(|| {
            let verbs = cohors_web::ACTION_VERBS
                .iter()
                .map(|&verb| {
                    view! {
                        <button
                            role="menuitem"
                            on:click=move |_| {
                                open.set(false);
                                actions::request(cx, verb, ids_selector(&repos::marked_ids(cx)));
                            }
                        >
                            {ui::capitalise(verb)}
                        </button>
                    }
                })
                .collect::<Vec<_>>();
            view! {
                // A full-screen layer under the menu: a click anywhere else closes it.
                <div class="menu-back" on:click=move |_| open.set(false)></div>
                <div class="menu" role="menu">
                    {verbs}
                </div>
            }
        })
    };
    view! {
        <span class="menu-wrap">
            <button
                class="btn"
                class:primary=move || count() != 0
                aria-haspopup="menu"
                aria-expanded=move || if open.get() { "true" } else { "false" }
                on:click=toggle
            >
                {ui::icon(Icon::Bolt)}
                <span>{label}</span>
            </button>
            {menu}
        </span>
    }
}

// ── Do next ──────────────────────────────────────────────────────────────────

/// One row per action: a severity mark, what to do, why, and one button.
fn do_next(cx: Ctx) -> impl IntoView {
    move || {
        let actions = cx.overview.with(|o| o.actions.clone());
        if actions.is_empty() {
            return view! {
                <div class="empty">
                    "Nothing needs doing. No repository has unpushed, uncommitted or stale work."
                </div>
            }
            .into_any();
        }
        actions
            .into_iter()
            .enumerate()
            .map(|(index, action)| {
                // The first, most urgent action carries the one primary button.
                let class = if index == 0 { "small primary" } else { "small" };
                view! {
                    <div class="todo">
                        {ui::action_mark(action.severity)}
                        <span class="t">{action.title.clone()}</span>
                        {action_button(cx, action.clone(), class)}
                        <span class="d">{action.why}</span>
                    </div>
                }
            })
            .collect::<Vec<_>>()
            .into_any()
    }
}

/// The button of a "Do next" row. An action with a registry verb runs it over
/// exactly the repositories the model named; any other opens the list where
/// the person can look first.
fn action_button(cx: Ctx, action: NextAction, class: &'static str) -> impl IntoView {
    // The verb arrives as text; match it to the static registry name.
    let verb = action.verb.as_deref().and_then(|v| {
        cohors_web::ACTION_VERBS
            .iter()
            .copied()
            .find(|known| *known == v)
    });
    match verb {
        Some(verb) => {
            let ids = action.repos.clone();
            actions::verb_button(cx, verb, action.label, class, None, move || {
                ids_selector(&ids)
            })
            .into_any()
        }
        None => {
            let label = action.label.clone();
            view! {
                <button class=format!("btn {class}") on:click=move |_| open_list(cx, &action)>
                    {label}
                </button>
            }
            .into_any()
        }
    }
}

/// Go to the list an action is about.
fn open_list(cx: Ctx, action: &NextAction) {
    match action.kind {
        ActionKind::StartTask => cx.go(Page::Tasks),
        // The same set as the saved view, so open that one.
        ActionKind::Commit => cx.open_view(RepoView::Uncommitted),
        _ => cx.open_view(RepoView::Custom {
            label: action.title.clone(),
            ids: action.repos.clone(),
        }),
    }
}

// ── At a glance ──────────────────────────────────────────────────────────────

/// Where a figure leads when clicked.
#[derive(Clone)]
enum Leads {
    View(RepoView),
    Page(Page),
}

/// The six key figures. Each is a button that opens the list behind it.
fn figures(cx: Ctx) -> impl IntoView {
    move || {
        let f: KeyFigures = cx.overview.with(|o| o.figures);
        let stale_days = STALE_FETCH_SECS / (24 * 60 * 60);

        let backed_up = match (f.unpublished, f.unpushed_repos) {
            (0, 0) => "everything is pushed".to_string(),
            (never, 0) => format!("{never} never pushed"),
            (0, _) => format!(
                "{} unpushed",
                ui::plural(f.unpushed_commits as usize, "commit", "commits")
            ),
            (never, _) => format!(
                "{never} never pushed, {} unpushed",
                ui::plural(f.unpushed_commits as usize, "commit", "commits")
            ),
        };
        let stashes = if f.stash_repos == 0 {
            "nothing parked".to_string()
        } else {
            let stale = match f.stale_stash_repos {
                0 => "none stale".to_string(),
                n if n == f.stash_repos => "all stale".to_string(),
                n => format!("{n} stale"),
            };
            format!("in {}, {stale}", ui::plural(f.stash_repos, "repo", "repos"))
        };
        let uncommitted = if f.dirty_repos == 0 {
            "every working tree is clean".to_string()
        } else {
            format!("across {}", ui::plural(f.dirty_repos, "repo", "repos"))
        };
        let fetches = if f.stale_fetch_repos == 0 {
            format!("all fetched within {stale_days} days")
        } else {
            format!("oldest {} days ago", f.oldest_fetch_days)
        };
        let tasks = if f.open_tasks == 0 {
            "nothing open".to_string()
        } else if f.doing_tasks > 0 {
            format!(
                "{} high priority, {} in progress",
                f.high_tasks, f.doing_tasks
            )
        } else {
            format!("{} high priority", f.high_tasks)
        };
        let commits = format!("{} the week before", f.commits_prev_week);

        let risk = if f.not_backed_up > 0 { "risk" } else { "" };
        let warn = if f.stale_stash_repos > 0 { "warn" } else { "" };
        view! {
            <div class="figs" role="group" aria-label="Key figures">
                {figure(
                    cx,
                    Icon::Upload,
                    "Not backed up",
                    f.not_backed_up.to_string(),
                    backed_up,
                    risk,
                    Leads::View(RepoView::NotBackedUp),
                )}
                {figure(
                    cx,
                    Icon::Layers,
                    "Stashes",
                    f.stashes.to_string(),
                    stashes,
                    warn,
                    Leads::View(RepoView::HasStash),
                )}
                {figure(
                    cx,
                    Icon::File,
                    "Uncommitted files",
                    f.uncommitted_files.to_string(),
                    uncommitted,
                    "",
                    Leads::View(RepoView::Uncommitted),
                )}
                {figure(
                    cx,
                    Icon::Refresh,
                    "Stale fetches",
                    f.stale_fetch_repos.to_string(),
                    fetches,
                    "",
                    Leads::View(RepoView::StaleFetch),
                )}
                {figure(
                    cx,
                    Icon::Check,
                    "Open tasks",
                    f.open_tasks.to_string(),
                    tasks,
                    "",
                    Leads::Page(Page::Tasks),
                )}
                {figure(
                    cx,
                    Icon::Commit,
                    "Commits, 7 days",
                    f.commits_week.to_string(),
                    commits,
                    "",
                    Leads::Page(Page::Standup),
                )}
            </div>
        }
    }
}

/// One figure: a small label, the number, and one line that qualifies it.
/// `tone` tints the number when it reports a problem.
fn figure(
    cx: Ctx,
    icon: Icon,
    label: &'static str,
    value: String,
    qualifier: String,
    tone: &'static str,
    leads: Leads,
) -> impl IntoView {
    view! {
        <button
            class=format!("fig {tone}")
            on:click=move |_| match leads.clone() {
                Leads::View(view) => cx.open_view(view),
                Leads::Page(page) => cx.go(page),
            }
        >
            <span class="k">{ui::icon(icon)} {label}</span>
            <span class="v">{value}</span>
            <span class="q">{qualifier}</span>
        </button>
    }
}

// ── Work at risk ─────────────────────────────────────────────────────────────

/// The repositories where local work could be lost, with every reason and
/// the single most useful step.
fn at_risk(cx: Ctx) -> impl IntoView {
    let summary = move || {
        let count = cx.overview.with(|o| o.figures.at_risk);
        match count {
            0 => "No repository has work at risk".to_string(),
            1 => "1 repository where local work could be lost".to_string(),
            n => format!("{n} repositories where local work could be lost"),
        }
    };
    let rows = move || {
        let (lines, now) = cx.overview.with(|o| {
            let lines: Vec<RepoLine> = o.at_risk().cloned().collect();
            (lines, o.now)
        });
        lines
            .into_iter()
            .map(|line| risk_row(cx, line, now))
            .collect::<Vec<_>>()
    };
    let any = move || cx.overview.with(|o| o.figures.at_risk > 0);
    view! {
        <section>
            <div class="sechead">
                <h2>{ui::icon(Icon::Alert)} "Work at risk"</h2>
                <span>{summary}</span>
            </div>
            <Show when=any>
                <div class="block">
                    <table>
                        <colgroup>
                            <col style="width:25%" />
                            <col />
                            <col style="width:92px" />
                            <col style="width:172px" />
                        </colgroup>
                        <thead>
                            <tr>
                                <th>"Repository"</th>
                                <th>"What is wrong"</th>
                                <th class="r">"Last commit"</th>
                                <th></th>
                            </tr>
                        </thead>
                        <tbody>{rows}</tbody>
                    </table>
                </div>
            </Show>
        </section>
    }
}

fn risk_row(cx: Ctx, line: RepoLine, now: i64) -> impl IntoView {
    let id = line.id.clone();
    let open = {
        let id = id.clone();
        move || cx.selected.set(Some(id.clone()))
    };
    let is_open = {
        let id = id.clone();
        move || cx.selected.with(|s| s.as_deref() == Some(id.as_str()))
    };
    let on_enter = {
        let open = open.clone();
        move |ev: leptos::ev::KeyboardEvent| {
            if ev.key() == "Enter" {
                open();
            }
        }
    };
    // The model's phrases can contain commas, so join them with semicolons.
    let wrong = ui::capitalise(&line.reasons.join("; "));
    let age = line
        .commit
        .as_ref()
        .map_or("never".to_string(), |c| ui::ago(c.timestamp, now));

    // The model's single suggested step. With a verb it runs on this one
    // repository; without one it opens the repository to look.
    let verb = line
        .action
        .as_ref()
        .and_then(|a| a.verb.as_deref())
        .and_then(|v| {
            cohors_web::ACTION_VERBS
                .iter()
                .copied()
                .find(|known| *known == v)
        });
    let label = line
        .action
        .as_ref()
        .map_or("Open".to_string(), |a| a.label.clone());
    let button = match verb {
        Some(verb) => {
            let ids = vec![id.clone()];
            actions::verb_button(cx, verb, label, "small", None, move || ids_selector(&ids))
                .into_any()
        }
        None => {
            let open = open.clone();
            view! {
                <button
                    class="btn small"
                    on:click=move |ev| {
                        ev.stop_propagation();
                        open();
                    }
                >
                    {label}
                </button>
            }
            .into_any()
        }
    };

    view! {
        <tr class="open-row" class:sel=is_open tabindex="0" on:click=move |_| open() on:keydown=on_enter>
            <td>
                <div class="clip" title=line.name.clone()>
                    {ui::severity_mark(line.severity)}
                    <span class="name">{line.name.clone()}</span>
                </div>
                <div class="sub clip">{ui::icon(Icon::Branch)} {line.branch.clone()}</div>
            </td>
            <td class="wrap">{wrong}</td>
            <td class="n r">{age}</td>
            <td class="r">{button}</td>
        </tr>
    }
}

// ── Open tasks and commits ───────────────────────────────────────────────────

/// Open tasks per repository, most first.
fn open_tasks(cx: Ctx) -> impl IntoView {
    let summary = move || {
        let f = cx.overview.with(|o| o.figures);
        if f.open_tasks == 0 {
            "None open".to_string()
        } else {
            format!(
                "{} across {}",
                f.open_tasks,
                ui::plural(f.task_repos, "repository", "repositories")
            )
        }
    };
    let rows = move || {
        cx.overview
            .with(|o| o.work.clone())
            .into_iter()
            .map(|work| {
                let id = work.id.clone();
                let open = move || cx.selected.set(Some(id.clone()));
                let on_enter = {
                    let open = open.clone();
                    move |ev: leptos::ev::KeyboardEvent| {
                        if ev.key() == "Enter" {
                            open();
                        }
                    }
                };
                view! {
                    <tr class="open-row" tabindex="0" on:click=move |_| open() on:keydown=on_enter>
                        <td class="name">{work.name}</td>
                        {count_cell(work.open, "")}
                        {count_cell(work.high, "caution")}
                        {count_cell(work.focus, "")}
                    </tr>
                }
            })
            .collect::<Vec<_>>()
    };
    let foot = move || {
        let f = cx.overview.with(|o| o.figures);
        let progress = match (f.doing_tasks, f.blocked_tasks) {
            (0, 0) => "Nothing is marked in progress.".to_string(),
            (doing, 0) => format!("{doing} in progress."),
            (0, blocked) => format!("Nothing in progress, {blocked} blocked."),
            (doing, blocked) => format!("{doing} in progress, {blocked} blocked."),
        };
        let unset = f.repos.saturating_sub(f.configured_repos);
        let files = match unset {
            0 => "Every repository has a launchpad file.".to_string(),
            1 => "1 repository has no launchpad file.".to_string(),
            n => format!("{n} repositories have no launchpad file."),
        };
        format!("{progress} {files}")
    };
    let any = move || cx.overview.with(|o| !o.work.is_empty());
    view! {
        <div>
            <div class="sechead">
                <h2>{ui::icon(Icon::Check)} "Open tasks"</h2>
                <span>{summary}</span>
            </div>
            <div class="block">
                <Show
                    when=any
                    fallback=|| {
                        view! {
                            <div class="empty">
                                "No launchpad file lists an open task."
                            </div>
                        }
                    }
                >
                    <table class="narrow">
                        <colgroup>
                            <col />
                            <col style="width:64px" />
                            <col style="width:64px" />
                            <col style="width:64px" />
                        </colgroup>
                        <thead>
                            <tr>
                                <th>"Repository"</th>
                                <th class="r">"Open"</th>
                                <th class="r">"High"</th>
                                <th class="r">"Focus"</th>
                            </tr>
                        </thead>
                        <tbody>{rows}</tbody>
                    </table>
                </Show>
                <div class="blockfoot">
                    {foot} " "
                    <a href=Page::Tasks.href()>"Open tasks"</a>
                </div>
            </div>
        </div>
    }
}

/// A right-aligned count in a table; zero is set back, `tone` tints the rest.
fn count_cell(value: usize, tone: &'static str) -> impl IntoView {
    let class = if value == 0 {
        "n r zero".to_string()
    } else {
        format!("n r {tone}")
    };
    view! { <td class=class>{value}</td> }
}

/// Commits in the last seven days per repository, as ranked bars.
fn commits(cx: Ctx) -> impl IntoView {
    let summary = move || {
        let f = cx.overview.with(|o| o.figures);
        format!(
            "{} in {}",
            ui::plural(f.commits_week as usize, "commit", "commits"),
            ui::plural(f.active_repos, "repository", "repositories")
        )
    };
    let rows = move || {
        let commits = cx.overview.with(|o| o.commits.clone());
        if commits.is_empty() {
            return view! { <div class="empty">"No commits in the last 7 days."</div> }.into_any();
        }
        // The list is sorted most first, so the first row sets the scale.
        let most = commits.first().map_or(1, |c| c.count).max(1);
        commits
            .into_iter()
            .map(|row| {
                let width = row.count as f32 / most as f32 * 100.0;
                let id = row.id.clone();
                view! {
                    <button class="bar-row" on:click=move |_| cx.selected.set(Some(id.clone()))>
                        <span class="clip">{row.name}</span>
                        <span class="track">
                            <i style=format!("width:{width:.1}%")></i>
                        </span>
                        <span class="num">{row.count}</span>
                    </button>
                }
            })
            .collect::<Vec<_>>()
            .into_any()
    };
    let foot = move || {
        let f = cx.overview.with(|o| o.figures);
        let quiet = f.repos.saturating_sub(f.active_repos);
        format!(
            "{} had no commits this week. The week before: {}.",
            ui::plural(quiet, "repository", "repositories"),
            f.commits_prev_week
        )
    };
    view! {
        <div>
            <div class="sechead">
                <h2>{ui::icon(Icon::Commit)} "Commits, last 7 days"</h2>
                <span>{summary}</span>
            </div>
            <div class="block">
                {rows}
                <div class="blockfoot">
                    {foot} " "
                    <a href=Page::Standup.href()>"Open standup"</a>
                </div>
            </div>
        </div>
    }
}
