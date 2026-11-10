//! The Tasks page: the focus board, open tasks by repository, what each
//! repository can run, the repositories with no launchpad file yet, and recent
//! agent sessions.
//!
//! Everything here is text from files in the user's repositories, so it is
//! always rendered as plain text, and a command is shown to copy, never run.

use cohors_core::launchpad::{ActivityEvent, Flow, FocusItem, LaunchpadEntry, Task};
use leptos::prelude::*;

use super::Ctx;
use super::ui::{self, Icon};

// ── Pieces shared with the drawer ────────────────────────────────────────────

/// A task's status as a shape mark and a word.
pub fn status_label(status: &str) -> impl IntoView + use<> {
    let (class, word) = match status {
        "doing" => ("doing", "In progress"),
        "blocked" => ("blocked", "Blocked"),
        "done" => ("ok", "Done"),
        _ => ("note", "To do"),
    };
    view! { <span class="state">{ui::mark(class, word)} {word}</span> }
}

/// A task's priority as a shape mark and a word. Normal priority gets the
/// word alone, so the marks that are there stand out.
pub fn priority_label(priority: Option<&str>) -> AnyView {
    match priority {
        Some("high") => {
            view! { <span class="state">{ui::mark("warn", "High priority")} "High"</span> }
                .into_any()
        }
        Some("low") => {
            view! { <span class="state dim">{ui::mark("low", "Low priority")} "Low"</span> }
                .into_any()
        }
        _ => view! { <span class="state dim">"Normal"</span> }.into_any(),
    }
}

/// The command that asks an agent to write a repository's launchpad file.
///
/// The path is always shell-quoted. The user pastes this line into a terminal
/// and a folder name is not trustworthy (it can hold quotes, `;` or `$(…)`),
/// so it must reach the shell as one inert argument. See
/// [`cohors_web::shell_quote`].
pub fn setup_command(path: &str) -> String {
    format!(
        "cohors agent run setup-repo --repo {}",
        cohors_web::shell_quote(path)
    )
}

/// A flow's label with its small facts (primary, port, folder, and whether it
/// was declared in the launchpad file or detected from a manifest) on one line.
pub fn flow_title(flow: &Flow, declared: bool) -> impl IntoView + use<> {
    let mut facts = Vec::new();
    if flow.primary {
        facts.push("primary".to_string());
    }
    if let Some(port) = flow.port {
        facts.push(format!("port {port}"));
    }
    if let Some(cwd) = &flow.cwd {
        facts.push(format!("in {cwd}"));
    }
    if flow.background {
        facts.push("keeps running".to_string());
    }
    facts.push(if declared { "declared" } else { "detected" }.to_string());
    // A detected flow is usually labelled with its own command, which is
    // already shown beside it: fall back to its kind (server, build, test).
    let label = if flow.label.is_empty() || flow.label == flow.run {
        flow.kind.clone().unwrap_or_else(|| flow.id.clone())
    } else {
        flow.label.clone()
    };
    view! {
        <span>{ui::capitalise(&label)}</span>
        <span class="tag">{facts.join(", ")}</span>
    }
}

/// Every flow of a repository: the ones its file declares, then the ones
/// detected from its manifests. The flag says which is which.
pub fn all_flows(entry: &LaunchpadEntry) -> Vec<(Flow, bool)> {
    let declared = entry
        .contribution
        .iter()
        .flat_map(|c| c.flows.iter())
        .map(|f| (f.clone(), true));
    let detected = entry.detected_flows.iter().map(|f| (f.clone(), false));
    declared.chain(detected).collect()
}

/// One task. With notes it is a `<details>`, so a click on the row opens them.
pub fn task_row(task: &Task) -> AnyView {
    let agent = task.agent.clone().unwrap_or_default();
    let cells = |hint: Option<&'static str>| {
        view! {
            {status_label(task.status())}
            <span class="t">
                {task.title.clone()}
                {hint.map(|text| view! { <span class="more">{text}</span> })}
            </span>
            {priority_label(task.priority.as_deref())}
            <span class="who clip" title=agent.clone()>{agent.clone()}</span>
        }
    };
    match task.notes.clone().filter(|n| !n.trim().is_empty()) {
        Some(notes) => view! {
            <details class="task">
                <summary class="task-row">{cells(Some("notes"))}</summary>
                <p class="task-notes">{notes}</p>
            </details>
        }
        .into_any(),
        None => view! {
            <div class="task">
                <div class="task-row">{cells(None)}</div>
            </div>
        }
        .into_any(),
    }
}

// ── The page ─────────────────────────────────────────────────────────────────

pub fn page(cx: Ctx) -> impl IntoView {
    let summary = move || {
        let f = cx.overview.with(|o| o.figures);
        let files = format!(
            "{} of {} have a launchpad file.",
            f.configured_repos,
            ui::plural(f.repos, "repository", "repositories")
        );
        if f.open_tasks == 0 {
            return format!("No open tasks. {files}");
        }
        format!(
            "{} in {}: {} high priority, {} in progress, {} blocked. {files}",
            ui::plural(f.open_tasks, "open task", "open tasks"),
            ui::plural(f.task_repos, "repository", "repositories"),
            f.high_tasks,
            f.doing_tasks,
            f.blocked_tasks,
        )
    };
    let problem = move || {
        cx.launchpad_error.get().map(|error| {
            view! {
                <div class="notice" role="alert">
                    <span>{format!("Could not read the launchpad: {error}")}</span>
                    <button class="btn small" on:click=move |_| cx.refresh()>
                        "Retry"
                    </button>
                </div>
            }
        })
    };
    view! {
        <section class="head">
            <div>
                <h1>"Tasks"</h1>
                <p>{summary}</p>
            </div>
        </section>
        {problem}
        {focus_board(cx)}
        {open_tasks(cx)}
        {flows(cx)}
        {not_set_up(cx)}
        {activity(cx)}
    }
}

/// The id of the one repository called `name`. `None` when no repository or
/// more than one has that name: a focus item names its repository by name, and
/// guessing between two would open the wrong one.
fn only_repo_named(cx: Ctx, name: &str) -> Option<String> {
    cx.launchpad.with(|lp| {
        let mut matches = lp.entries.iter().filter(|e| e.name == name);
        let first = matches.next()?;
        matches.next().is_none().then(|| first.id.clone())
    })
}

/// The cross-repository focus board: the short list of what is being worked
/// on next.
fn focus_board(cx: Ctx) -> impl IntoView {
    let items = move || cx.overview.with(|o| o.focus.clone());
    let updated = move || {
        cx.launchpad
            .with(|lp| lp.board.updated.clone())
            .and_then(|stamp| parse_time(&stamp))
            .map(|at| format!("Updated {}", ui::local_time(at)))
    };
    let rows = move || {
        let items = items();
        if items.is_empty() {
            return view! {
                <div class="empty">
                    <p>"The focus board is empty."</p>
                    <p>
                        "It is the short list of what you mean to work on next across your \
                         repositories, kept in "
                        <code>"~/.cohors/board.json"</code>
                        "."
                    </p>
                </div>
            }
            .into_any();
        }
        items
            .into_iter()
            .map(|item| focus_row(cx, item))
            .collect::<Vec<_>>()
            .into_any()
    };
    view! {
        <section>
            <div class="sechead">
                <h2>{ui::icon(Icon::Bookmark)} "Focus board"</h2>
                <span>
                    {move || ui::plural(items().len(), "item", "items")}
                </span>
                <span class="grow"></span>
                <span>{updated}</span>
            </div>
            <div class="block">{rows}</div>
        </section>
    }
}

fn focus_row(cx: Ctx, item: FocusItem) -> impl IntoView {
    let status = item.status.clone().unwrap_or_else(|| "todo".to_string());
    let mut facts = Vec::new();
    if let Some(reference) = &item.reference {
        facts.push(format!("task {reference}"));
    }
    if let Some(agent) = &item.agent {
        facts.push(format!("agent {agent}"));
    }
    let repo = item.repo.clone().map(|name| {
        match only_repo_named(cx, &name) {
            Some(id) => view! {
                <button class="btn small quiet" on:click=move |_| cx.selected.set(Some(id.clone()))>
                    {ui::icon(Icon::Folder)}
                    <span>{name}</span>
                </button>
            }
            .into_any(),
            // Not in this scan (or ambiguous): show the name without a link.
            None => view! { <span class="soft">{name}</span> }.into_any(),
        }
    });
    view! {
        <div class="focus-row">
            {status_label(&status)}
            <span>
                {item.title}
                <span class="sub">{facts.join(", ")}</span>
            </span>
            <span>{repo}</span>
        </div>
    }
}

/// Open tasks, one block per repository, in the model's order (most open
/// tasks first).
fn open_tasks(cx: Ctx) -> impl IntoView {
    let groups = move || {
        let work = cx.overview.with(|o| o.work.clone());
        if work.is_empty() {
            return view! {
                <div class="block">
                    <div class="empty">
                        "No launchpad file lists an open task."
                    </div>
                </div>
            }
            .into_any();
        }
        work.into_iter()
            .filter_map(|group| {
                // The entry holds the tasks themselves; matched by id only.
                let entry = cx.entry(&group.id)?;
                let id = group.id.clone();
                let mut counts = vec![format!("{} open", group.open)];
                if group.high > 0 {
                    counts.push(format!("{} high priority", group.high));
                }
                if group.doing > 0 {
                    counts.push(format!("{} in progress", group.doing));
                }
                if group.blocked > 0 {
                    counts.push(format!("{} blocked", group.blocked));
                }
                if group.focus > 0 {
                    counts.push(format!("{} on the focus board", group.focus));
                }
                let rows = entry
                    .open_tasks()
                    .into_iter()
                    .map(task_row)
                    .collect::<Vec<_>>();
                Some(view! {
                    <div class="block">
                        <div class="blockhead">
                            <h3>
                                <button
                                    class="link"
                                    on:click=move |_| cx.selected.set(Some(id.clone()))
                                >
                                    {group.name}
                                </button>
                            </h3>
                            <span class="meta">{counts.join(", ")}</span>
                        </div>
                        {rows}
                    </div>
                })
            })
            .collect::<Vec<_>>()
            .into_any()
    };
    view! {
        <section>
            <div class="sechead">
                <h2>{ui::icon(Icon::Check)} "Open tasks"</h2>
                <span>"By repository, most first. A task with notes opens on click."</span>
            </div>
            {groups}
        </section>
    }
}

/// What each repository can run: the flows its launchpad file declares and
/// the ones detected from its manifests, each with a command to copy.
///
/// One row per repository, which opens to show its flows. Repositories that
/// declare flows come first and start open; a detected-only one stays closed
/// until asked, with its commands previewed on the row.
fn flows(cx: Ctx) -> impl IntoView {
    let groups = move || {
        let mut entries: Vec<LaunchpadEntry> = cx.launchpad.with(|lp| {
            lp.entries
                .iter()
                .filter(|e| !all_flows(e).is_empty())
                .cloned()
                .collect()
        });
        if entries.is_empty() {
            return view! {
                <div class="empty">
                    "No repository declares a flow, and none was detected from a manifest."
                </div>
            }
            .into_any();
        }
        let declares =
            |e: &LaunchpadEntry| e.contribution.as_ref().is_some_and(|c| !c.flows.is_empty());
        // Declared first; within each half the server's order (by name) stays.
        entries.sort_by_key(|e| !declares(e));
        entries
            .into_iter()
            .map(|entry| {
                let flows = all_flows(&entry);
                let declared = flows.iter().filter(|(_, declared)| *declared).count();
                let detected = flows.len() - declared;
                let preview = flows
                    .iter()
                    .map(|(flow, _)| flow.run.clone())
                    .collect::<Vec<_>>()
                    .join("  ·  ");
                let rows = flows
                    .into_iter()
                    .map(|(flow, declared)| {
                        view! {
                            <div class="flow-row">
                                <span>{flow_title(&flow, declared)}</span>
                                {ui::repo_command(cx, flow.run.clone())}
                            </div>
                        }
                    })
                    .collect::<Vec<_>>();
                view! {
                    <details class="group" open=declared != 0>
                        <summary>
                            {ui::icon(Icon::Chevron)}
                            <span class="name clip">{entry.name}</span>
                            <span class="meta">{flow_counts(declared, detected)}</span>
                            <span class="sub clip">{preview}</span>
                        </summary>
                        {rows}
                    </details>
                }
            })
            .collect::<Vec<_>>()
            .into_any()
    };
    let count = move || {
        cx.launchpad.with(|lp| {
            lp.entries
                .iter()
                .filter(|e| !all_flows(e).is_empty())
                .count()
        })
    };
    view! {
        <section>
            <div class="sechead">
                <h2>{ui::icon(Icon::Terminal)} "Flows"</h2>
                <span>
                    {move || {
                        format!(
                            "Commands written by {}, in its own files, shown as written. \
                             cohors never runs them: read one, then copy it to run it yourself.",
                            ui::plural(count(), "repository", "repositories"),
                        )
                    }}
                </span>
            </div>
            <div class="block">{groups}</div>
        </section>
    }
}

/// `2 declared, 1 detected`, leaving out a half that is zero.
fn flow_counts(declared: usize, detected: usize) -> String {
    match (declared, detected) {
        (declared, 0) => format!("{declared} declared"),
        (0, detected) => format!("{detected} detected"),
        (declared, detected) => format!("{declared} declared, {detected} detected"),
    }
}

/// Repositories with no launchpad file (or a broken one). The setup command
/// is the same for all of them apart from the path, so it is shown once and
/// each repository gets a button that copies its own.
fn not_set_up(cx: Ctx) -> impl IntoView {
    let missing = move || {
        cx.launchpad.with(|lp| {
            lp.entries
                .iter()
                .filter(|e| !e.configured)
                .cloned()
                .collect::<Vec<_>>()
        })
    };
    let rows = move || {
        missing()
            .into_iter()
            .map(|entry| {
                let path = cx.line(&entry.id).and_then(|line| line.path);
                let id = entry.id.clone();
                let copy = match path.clone() {
                    Some(path) => view! {
                        <button
                            class="btn small"
                            title=setup_command(&path)
                            on:click=move |_| {
                                ui::copy_text(cx, setup_command(&path), "setup command")
                            }
                        >
                            "Copy command"
                        </button>
                    }
                    .into_any(),
                    None => view! { <span class="dim">"No folder here"</span> }.into_any(),
                };
                // A broken file is said in words; otherwise show where it lives.
                let note = match entry.invalid {
                    Some(why) => format!("file is invalid: {why}"),
                    None => path.as_deref().map(ui::short_path).unwrap_or_default(),
                };
                view! {
                    <div class="setup-row">
                        <span class="clip">
                            <button
                                class="link name"
                                on:click=move |_| cx.selected.set(Some(id.clone()))
                            >
                                {entry.name}
                            </button>
                            <span class="sub clip" title=note.clone()>{note.clone()}</span>
                        </span>
                        {copy}
                    </div>
                }
            })
            .collect::<Vec<_>>()
    };
    move || {
        let count = missing().len();
        (count > 0).then(|| {
            view! {
                <section>
                    <div class="sechead">
                        <h2>{ui::icon(Icon::File)} "No launchpad file"</h2>
                        <span>
                            {format!(
                                "{} with no tasks or declared flows yet",
                                ui::plural(count, "repository", "repositories"),
                            )}
                        </span>
                    </div>
                    <div class="block">
                        <div class="setup-how">
                            <span>
                                "Set up with an agent. It reads the repository and writes the \
                                 file. Run this with the repository's path:"
                            </span>
                            <code>"cohors agent run setup-repo --repo <path>"</code>
                        </div>
                        <div class="setup-grid">{rows}</div>
                    </div>
                </section>
            }
        })
    }
}

/// Recent agent sessions from the activity log, newest first.
fn activity(cx: Ctx) -> impl IntoView {
    let events = move || cx.launchpad.with(|lp| lp.recent_activity.clone());
    let body = move || {
        let events = events();
        if events.is_empty() {
            return view! {
                <div class="empty">
                    <p>"No agent sessions have been recorded."</p>
                    <p>
                        "Sessions appear here once an agent runner logs them to "
                        <code>"~/.cohors/history"</code>
                        "."
                    </p>
                </div>
            }
            .into_any();
        }
        view! {
            <table>
                <colgroup>
                    <col style="width:140px" />
                    <col style="width:96px" />
                    <col />
                    <col style="width:22%" />
                    <col style="width:20%" />
                </colgroup>
                <thead>
                    <tr>
                        <th>"When"</th>
                        <th>"Event"</th>
                        <th>"Repository"</th>
                        <th>"Branch"</th>
                        <th>"Agent"</th>
                    </tr>
                </thead>
                <tbody>
                    {events.into_iter().map(|event| activity_row(cx, event)).collect::<Vec<_>>()}
                </tbody>
            </table>
        }
        .into_any()
    };
    view! {
        <section>
            <div class="sechead">
                <h2>{ui::icon(Icon::Clock)} "Recent agent activity"</h2>
                <span>{move || ui::plural(events().len(), "event", "events")}</span>
            </div>
            <div class="block">{body}</div>
        </section>
    }
}

fn activity_row(cx: Ctx, event: ActivityEvent) -> impl IntoView {
    let when = parse_time(&event.ts).map_or(event.ts.clone(), ui::local_time);
    let phase = match event.phase.as_str() {
        "start" => "Started".to_string(),
        "end" => "Ended".to_string(),
        "launch" => "Launched".to_string(),
        "note" => "Note".to_string(),
        other => ui::capitalise(other),
    };
    // The runner is the program (claude-code); the agent is the repository's
    // own named agent, when one was running.
    let agent = if event.agent.is_empty() {
        event.runner.clone()
    } else {
        format!("{}, {}", event.agent, event.runner)
    };
    let repo = session_place(cx, &event.repo);
    view! {
        <tr>
            <td class="n">{when}</td>
            <td>{phase}</td>
            <td title=event.repo>{repo}</td>
            <td class="n">{event.branch}</td>
            <td>{agent}</td>
        </tr>
    }
}

/// Where a session ran, named the way the rest of the page names things. The
/// log records a folder: a scanned repository shows its name, a folder inside
/// one (a worktree, say) shows the repository and the folder, and anything
/// else shows its path.
fn session_place(cx: Ctx, folder: &str) -> String {
    let known = cx.overview.with_untracked(|o| {
        o.repos
            .iter()
            .filter_map(|line| Some((line.name.clone(), line.path.clone()?)))
            // The longest matching path is the most specific repository.
            .filter(|(_, path)| folder == path || folder.starts_with(&format!("{path}/")))
            .max_by_key(|(_, path)| path.len())
    });
    match known {
        Some((name, path)) if path == folder => name,
        Some((name, _)) => format!("{name}, in {}", ui::last_component(folder)),
        None => ui::short_path(folder),
    }
}

/// Parse an ISO-8601 timestamp (`2026-11-10T09:40:00Z`) to Unix seconds.
fn parse_time(stamp: &str) -> Option<i64> {
    let millis = js_sys::Date::parse(stamp);
    (!millis.is_nan()).then(|| (millis / 1000.0) as i64)
}
