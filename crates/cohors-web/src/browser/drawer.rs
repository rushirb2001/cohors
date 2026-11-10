//! The drawer: one repository in full, sliding over the page.
//!
//! The top half comes from data already on the page (the overview's line, the
//! snapshot, the launchpad entry) and shows at once. The rest (changed files,
//! recent commits, branches, stashes, pull requests) is read from the
//! repository on demand through `/api/detail`.

use cohors_core::{BranchInfo, CiStatus, RepoLine, RepoSnapshot};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use super::Ctx;
use super::actions::{self, ids_selector};
use super::repos::{mini_bars, sync_summary};
use super::tasks;
use super::ui::{self, Icon};
use crate::api;

/// How many changed files the list shows before it says "and N more".
const FILES_SHOWN: usize = 60;

/// Where the on-demand detail stands.
#[derive(Clone)]
enum DetailState {
    Loading,
    Loaded(Box<api::Detail>),
    Failed(String),
    /// There is nothing to read, and this says why.
    Unavailable(&'static str),
}

pub fn drawer(cx: Ctx) -> impl IntoView {
    let detail = RwSignal::new(DetailState::Loading);

    // Read the detail whenever a repository opens, and again after an action
    // has run (`detail_epoch`), since the action may have changed it.
    Effect::new(move |_| {
        cx.detail_epoch.track();
        let Some(id) = cx.selected.get() else {
            return;
        };
        load_detail(cx, id, detail);
    });

    move || cx.selected.get().map(|id| panel(cx, id, detail))
}

fn load_detail(cx: Ctx, id: String, detail: RwSignal<DetailState>) {
    if cx.meta.with_untracked(|m| m.demo) {
        detail.set(DetailState::Unavailable(
            "This is demo data. There is no folder behind it to read commits, branches or files from.",
        ));
        return;
    }
    let path = cx.overview.with_untracked(|o| {
        o.repos
            .iter()
            .find(|r| r.id == id)
            .and_then(|r| r.path.clone())
    });
    let Some(path) = path else {
        detail.set(DetailState::Unavailable(
            "This repository has no folder on this machine, so there is nothing more to read.",
        ));
        return;
    };
    let remote_url = cx.snapshots.with_untracked(|all| {
        all.iter()
            .find(|s| s.id.0 == id)
            .and_then(|s| s.remote_url.clone())
    });
    detail.set(DetailState::Loading);
    spawn_local(async move {
        let result = api::fetch_detail(&path, remote_url.as_deref()).await;
        // The answer is stale if another repository was opened meanwhile.
        if cx.selected.get_untracked().as_deref() != Some(id.as_str()) {
            return;
        }
        detail.set(match result {
            Ok(loaded) => DetailState::Loaded(Box::new(loaded)),
            Err(error) => DetailState::Failed(error),
        });
    });
}

fn panel(cx: Ctx, id: String, detail: RwSignal<DetailState>) -> impl IntoView {
    // Each part reads only the signals it needs, so a background refresh
    // redraws a section without resetting the drawer's scroll position.
    let header = {
        let id = id.clone();
        move || match cx.line(&id) {
            Some(line) => view! {
                <h2>{ui::severity_mark(line.severity)} {line.name}</h2>
                <div class="sub">{ui::icon(Icon::Branch)} {line.branch}</div>
            }
            .into_any(),
            None => view! { <h2>{ui::last_component(&id).to_string()}</h2> }.into_any(),
        }
    };
    let body = {
        let id = id.clone();
        move || match cx.line(&id) {
            Some(line) => view! {
                {state_section(cx, line.clone())}
                {reasons_section(line.clone())}
            }
            .into_any(),
            None => view! {
                <div class="sec soft">"This repository is no longer in the scan."</div>
            }
            .into_any(),
        }
    };
    let on_demand = {
        let id = id.clone();
        move || match detail.get() {
            DetailState::Loading => view! {
                <div class="sec soft" role="status">
                    <span class="spin"></span>
                    "Reading commits, branches and files"
                </div>
            }
            .into_any(),
            DetailState::Unavailable(why) => view! { <div class="sec soft">{why}</div> }.into_any(),
            DetailState::Failed(error) => {
                let id = id.clone();
                view! {
                    <div class="sec" role="alert">
                        <h3>"Could not read this repository"</h3>
                        <p class="soft">{error}</p>
                        <p>
                            <button
                                class="btn small"
                                on:click=move |_| load_detail(cx, id.clone(), detail)
                            >
                                "Retry"
                            </button>
                        </p>
                    </div>
                }
                .into_any()
            }
            DetailState::Loaded(loaded) => {
                let now = cx.overview.with(|o| o.now);
                view! {
                    {changes_section(&loaded)}
                    {commits_section(&loaded, now)}
                    {branches_section(&loaded.local.branches, now)}
                    {stashes_section(&loaded.local.stashes)}
                }
                .into_any()
            }
        }
    };
    let remote = {
        let id = id.clone();
        move || {
            let snapshot = cx.snapshot(&id);
            let loaded = match detail.get() {
                DetailState::Loaded(loaded) => Some(loaded),
                _ => None,
            };
            remote_section(snapshot, loaded)
        }
    };
    let launchpad = {
        let id = id.clone();
        move || launchpad_sections(cx, &id)
    };

    view! {
        <aside class="drawer" aria-label="Repository detail">
            <header>
                <div class="grow">{header}</div>
                <button
                    class="icon-btn"
                    aria-label="Close"
                    on:click=move |_| cx.selected.set(None)
                >
                    {ui::icon(Icon::Close)}
                </button>
            </header>
            {action_row(cx, id)}
            <div class="body">{body} {on_demand} {remote} {launchpad}</div>
        </aside>
    }
}

/// The verbs for this one repository. The model's suggested step leads.
fn action_row(cx: Ctx, id: String) -> impl IntoView {
    move || {
        let suggested = cx
            .line(&id)
            .and_then(|line| line.action)
            .and_then(|action| {
                let verb = cohors_web::ACTION_VERBS
                    .iter()
                    .copied()
                    .find(|known| Some(*known) == action.verb.as_deref())?;
                Some((verb, action.label))
            });
        let lead = suggested.clone().map(|(verb, label)| {
            let ids = vec![id.clone()];
            actions::verb_button(cx, verb, label, "small primary", None, move || {
                ids_selector(&ids)
            })
        });
        let rest = cohors_web::ACTION_VERBS
            .iter()
            .copied()
            // The suggested step already has its button.
            .filter(|verb| suggested.as_ref().is_none_or(|(lead, _)| lead != verb))
            .map(|verb| {
                let ids = vec![id.clone()];
                actions::verb_button(cx, verb, ui::capitalise(verb), "small", None, move || {
                    ids_selector(&ids)
                })
            })
            .collect::<Vec<_>>();
        view! { <div class="acts">{lead} {rest} {actions::read_only_note(cx)}</div> }
    }
}

// ── From the page's own data ─────────────────────────────────────────────────

/// The repository's state as labelled values.
fn state_section(cx: Ctx, line: RepoLine) -> impl IntoView {
    let now = cx.overview.with(|o| o.now);
    let snapshot = cx.snapshot(&line.id);
    let sync = sync_summary(&line, now);

    let path = line.path.clone().map(|path| {
        let copied = path.clone();
        view! {
            <dt>"Path"</dt>
            <dd class="with-copy">
                <span class="mono" title=path.clone()>{ui::short_path(&path)}</span>
                <button
                    class="icon-btn"
                    aria-label="Copy the full path"
                    title="Copy the full path"
                    on:click=move |_| ui::copy_text(cx, copied.clone(), "path")
                >
                    {ui::icon(Icon::Copy)}
                </button>
            </dd>
        }
    });

    let upstream = match (&line.upstream, line.has_remote) {
        (Some(name), _) => view! {
            <span class="mono">{name.clone()}</span>
            {format!(", {}", sync.main.to_lowercase())}
        }
        .into_any(),
        (None, true) if sync.class == "bad" => {
            view! { <span class="bad">"None. This branch has never been pushed."</span> }.into_any()
        }
        (None, true) => "None.".into_any(),
        (None, false) => "None. This repository has no remote.".into_any(),
    };

    let remote = snapshot
        .as_ref()
        .and_then(|s| s.remote_url.clone())
        .map(|url| {
            // Only an http(s) URL is safe and useful as a link.
            let link = url.starts_with("https://") || url.starts_with("http://");
            let shown = if link {
                view! {
                    <a class="mono" href=url.clone() target="_blank" rel="noreferrer">
                        {url.clone()}
                    </a>
                }
                .into_any()
            } else {
                view! { <span class="mono">{url.clone()}</span> }.into_any()
            };
            view! {
                <dt>"Remote"</dt>
                <dd>{shown}</dd>
            }
        });

    let fetched = if !line.has_remote {
        "Nothing to fetch from".to_string()
    } else {
        match line.fetched {
            Some(at) => format!("{} ({})", ui::ago(at, now), ui::local_time(at)),
            None => "Never".to_string(),
        }
    };

    let mut tree = Vec::new();
    for (count, word) in [
        (line.conflicted, "conflicted"),
        (line.staged, "staged"),
        (line.modified, "modified"),
        (line.untracked, "untracked"),
    ] {
        if count > 0 {
            tree.push(format!("{count} {word}"));
        }
    }
    let tree = if tree.is_empty() {
        "Clean".to_string()
    } else {
        tree.join(", ")
    };

    let stashes = match line.stashes {
        0 => "None".to_string(),
        count => match snapshot.as_ref().and_then(|s| s.stash_latest) {
            Some(at) => format!("{count}, newest {}", ui::ago(at, now)),
            None => count.to_string(),
        },
    };

    let commit = match &line.commit {
        Some(c) => view! {
            <span class="mono">{c.short_id.clone()}</span>
            {format!(", {} by {}", ui::ago(c.timestamp, now), c.author)}
            <br />
            <span class="soft">{c.summary.clone()}</span>
        }
        .into_any(),
        None => "No commits yet".into_any(),
    };

    let in_progress = snapshot
        .as_ref()
        .and_then(|s| s.operation)
        .map(|operation| {
            view! {
                <dt>"In progress"</dt>
                <dd class="bad">{format!("A {} is paused part-way", operation.label())}</dd>
            }
        });
    let tag = snapshot
        .as_ref()
        .and_then(|s| s.latest_tag.clone())
        .map(|tag| {
            let since = match tag.commits_since {
                0 => "at this commit".to_string(),
                n => format!("{} since", ui::plural(n as usize, "commit", "commits")),
            };
            view! {
                <dt>"Latest tag"</dt>
                <dd>
                    <span class="mono">{tag.name}</span>
                    {format!(", {since}")}
                </dd>
            }
        });
    let groups = (!line.groups.is_empty()).then(|| {
        view! {
            <dt>"Groups"</dt>
            <dd>{line.groups.join(", ")}</dd>
        }
    });
    let total: u32 = line.activity.iter().map(|&w| u32::from(w)).sum();
    let activity = (!line.activity.is_empty()).then(|| {
        view! {
            <dt>"Commits"</dt>
            <dd>
                {mini_bars(&line.activity)}
                {format!(
                    " {} in {}",
                    total,
                    ui::plural(line.activity.len(), "week", "weeks"),
                )}
            </dd>
        }
    });

    view! {
        <div class="sec">
            <dl class="kv">
                {path} <dt>"Upstream"</dt> <dd>{upstream}</dd> {remote} <dt>"Last fetch"</dt>
                <dd>{fetched}</dd> <dt>"Working tree"</dt> <dd>{tree}</dd> <dt>"Stashes"</dt>
                <dd>{stashes}</dd> {in_progress} <dt>"Last commit"</dt> <dd>{commit}</dd> {tag}
                {groups} {activity}
            </dl>
        </div>
    }
}

/// Every reason the model gave, most urgent first, in its own words.
fn reasons_section(line: RepoLine) -> impl IntoView {
    let body = if line.reasons.is_empty() {
        view! { <p class="soft">"Nothing. This repository is clean."</p> }.into_any()
    } else {
        view! {
            <ul class="list reasons">
                {line
                    .reasons
                    .iter()
                    .map(|reason| {
                        view! {
                            <li>
                                {ui::mark("note", "Reason")}
                                <span class="t">{ui::capitalise(reason)}</span>
                            </li>
                        }
                    })
                    .collect::<Vec<_>>()}
            </ul>
        }
        .into_any()
    };
    view! {
        <div class="sec">
            <h3>
                "What needs attention"
                <span class="n">{line.reasons.len()}</span>
            </h3>
            {body}
        </div>
    }
}

// ── Read on demand ───────────────────────────────────────────────────────────

/// The changed files and, folded away, the diff.
fn changes_section(detail: &api::Detail) -> impl IntoView + use<> {
    let files = &detail.local.changed_files;
    let body = if files.is_empty() {
        view! { <p class="soft">"No uncommitted changes."</p> }.into_any()
    } else {
        let hidden = files.len().saturating_sub(FILES_SHOWN);
        let rows = files
            .iter()
            .take(FILES_SHOWN)
            .map(|file| {
                view! {
                    <li>
                        // Git's two-letter status code, e.g. " M" or "??".
                        <span class="code">{file.status.replace(' ', "\u{a0}")}</span>
                        <span class="t mono">{file.path.clone()}</span>
                    </li>
                }
            })
            .collect::<Vec<_>>();
        view! {
            <ul class="list">{rows}</ul>
            {(hidden > 0)
                .then(|| view! { <p class="soft">{format!("and {hidden} more files")}</p> })}
        }
        .into_any()
    };

    let patch = detail
        .changes
        .patch
        .clone()
        .filter(|p| !p.is_empty())
        .map(|patch| {
            let lines = patch
                .lines()
                .map(|line| {
                    // Tint a line by its diff prefix.
                    let class = if line.starts_with("diff ") {
                        "fileh"
                    } else if line.starts_with("@@") {
                        "hunk"
                    } else if line.starts_with('+') && !line.starts_with("+++") {
                        "add"
                    } else if line.starts_with('-') && !line.starts_with("---") {
                        "del"
                    } else {
                        ""
                    };
                    view! { <span class=class>{format!("{line}\n")}</span> }
                })
                .collect::<Vec<_>>();
            let cut = detail.changes.truncated.then(|| {
                view! { <p class="soft">"The diff is cut off at 20 KB. The rest is not shown."</p> }
            });
            view! {
                <details class="fold">
                    <summary>"Show the diff against the last commit"</summary>
                    <pre class="patch">{lines}</pre>
                    {cut}
                </details>
            }
        });

    view! {
        <div class="sec">
            <h3>
                "Changed files"
                <span class="n">{files.len()}</span>
            </h3>
            {body}
            {patch}
        </div>
    }
}

fn commits_section(detail: &api::Detail, now: i64) -> impl IntoView + use<> {
    let commits = &detail.local.recent_commits;
    let body = if commits.is_empty() {
        view! { <p class="soft">"No commits."</p> }.into_any()
    } else {
        view! {
            <ul class="list">
                {commits
                    .iter()
                    .map(|c| {
                        view! {
                            <li>
                                <span class="code">{c.short_id.clone()}</span>
                                <span class="t">{c.summary.clone()}</span>
                                <span class="meta" title=c.author.clone()>
                                    {ui::ago(c.timestamp, now)}
                                </span>
                            </li>
                        }
                    })
                    .collect::<Vec<_>>()}
            </ul>
        }
        .into_any()
    };
    view! {
        <div class="sec">
            <h3>
                "Recent commits"
                <span class="n">{commits.len()}</span>
            </h3>
            {body}
        </div>
    }
}

/// Every local branch with its own standing against its own upstream.
fn branches_section(branches: &[BranchInfo], now: i64) -> impl IntoView + use<> {
    let rows = branches
        .iter()
        .map(|branch| {
            let sync = if !branch.has_upstream {
                "no upstream".to_string()
            } else {
                match (branch.ahead, branch.behind) {
                    (0, 0) => "level".to_string(),
                    (ahead, 0) => format!("{ahead} ahead"),
                    (0, behind) => format!("{behind} behind"),
                    (ahead, behind) => format!("{ahead} ahead, {behind} behind"),
                }
            };
            let age = branch
                .last_commit
                .as_ref()
                .map(|c| ui::ago(c.timestamp, now))
                .unwrap_or_default();
            view! {
                <li>
                    <span class="t">
                        <span class="mono">{branch.name.clone()}</span>
                        {branch.is_current.then(|| view! { <span class="soft">" (checked out)"</span> })}
                    </span>
                    <span class="meta">{sync}</span>
                    <span class="meta">{age}</span>
                </li>
            }
        })
        .collect::<Vec<_>>();
    let body = if rows.is_empty() {
        view! { <p class="soft">"No local branches."</p> }.into_any()
    } else {
        view! { <ul class="list">{rows}</ul> }.into_any()
    };
    view! {
        <div class="sec">
            <h3>
                "Branches"
                <span class="n">{branches.len()}</span>
            </h3>
            {body}
        </div>
    }
}

fn stashes_section(stashes: &[String]) -> impl IntoView + use<> {
    let body = if stashes.is_empty() {
        view! { <p class="soft">"No stashes."</p> }.into_any()
    } else {
        view! {
            <ul class="list">
                {stashes
                    .iter()
                    .enumerate()
                    .map(|(index, message)| {
                        view! {
                            <li>
                                <span class="code">{format!("stash@{{{index}}}")}</span>
                                <span class="t">{message.clone()}</span>
                            </li>
                        }
                    })
                    .collect::<Vec<_>>()}
            </ul>
        }
        .into_any()
    };
    view! {
        <div class="sec">
            <h3>
                "Stashes"
                <span class="n">{stashes.len()}</span>
            </h3>
            {body}
        </div>
    }
}

/// A CI status as a shape mark and a word.
fn ci_label(status: CiStatus) -> impl IntoView {
    match status {
        CiStatus::Passing => view! { {ui::mark("ok", "Passing")} "Passing" }.into_any(),
        CiStatus::Failing => view! { {ui::mark("risk", "Failing")} "Failing" }.into_any(),
        CiStatus::Pending => view! { {ui::mark("note", "Running")} "Running" }.into_any(),
        CiStatus::None => "No checks".into_any(),
    }
}

/// CI, pull requests, issues and the latest release, when the server has
/// them. When it does not, say so instead of leaving a gap.
fn remote_section(snapshot: Option<RepoSnapshot>, detail: Option<Box<api::Detail>>) -> AnyView {
    let has_remote_url = snapshot.as_ref().is_some_and(|s| s.remote_url.is_some());
    let info = snapshot.as_ref().and_then(|s| s.remote.clone());
    let extra = detail.and_then(|d| d.remote);
    if info.is_none() && extra.is_none() {
        // No remote at all: the state section already says so.
        if !has_remote_url {
            return ().into_any();
        }
        return view! {
            <div class="sec">
                <h3>"CI and pull requests"</h3>
                <p class="soft">
                    "The server has no CI or pull request data for this remote. It reads them \
                     from GitHub, when a token is configured."
                </p>
            </div>
        }
        .into_any();
    }

    let branch = snapshot
        .as_ref()
        .map(|s| s.branch.label())
        .unwrap_or_default();
    let summary = info.map(|remote| {
        let on_branch = remote.ci_by_branch.get(&branch).copied();
        let review = match remote.prs_awaiting_review {
            0 => String::new(),
            n => format!(", {n} waiting for your review"),
        };
        view! {
            <dt>"Repository"</dt>
            <dd class="mono">{format!("{}/{}/{}", remote.host, remote.owner, remote.repo)}</dd>
            <dt>{format!("CI on {}", remote.default_branch)}</dt>
            <dd>{ci_label(remote.ci)}</dd>
            {on_branch
                .filter(|_| branch != remote.default_branch)
                .map(|status| {
                    view! {
                        <dt>"CI on this branch"</dt>
                        <dd>{ci_label(status)}</dd>
                    }
                })}
            <dt>"Pull requests"</dt>
            <dd>{format!("{} open{review}", remote.open_prs)}</dd>
            {remote
                .description
                .filter(|d| !d.is_empty())
                .map(|description| {
                    view! {
                        <dt>"About"</dt>
                        <dd class="soft">{description}</dd>
                    }
                })}
        }
    });

    let more = extra.clone().map(|remote| {
        let assigned = match remote.assigned_issues {
            0 => String::new(),
            n => format!(", {n} assigned to you"),
        };
        view! {
            {remote
                .latest_run
                .map(|status| {
                    view! {
                        <dt>"Latest run"</dt>
                        <dd>{ci_label(status)}</dd>
                    }
                })}
            <dt>"Issues"</dt>
            <dd>{format!("{} open{assigned}", remote.open_issues)}</dd>
            {remote
                .latest_release
                .map(|tag| {
                    view! {
                        <dt>"Release"</dt>
                        <dd class="mono">{tag}</dd>
                    }
                })}
            {remote
                .default_branch_protected
                .map(|protected| {
                    view! {
                        <dt>"Protection"</dt>
                        <dd>
                            {if protected {
                                "The default branch is protected"
                            } else {
                                "The default branch is not protected"
                            }}
                        </dd>
                    }
                })}
        }
    });

    let pulls = extra.clone().filter(|r| !r.prs.is_empty()).map(|remote| {
        view! {
            <ul class="list">
                {remote
                    .prs
                    .into_iter()
                    .map(|pr| {
                        let mut facts = vec![pr.author.clone(), pr.branch.clone()];
                        if pr.draft {
                            facts.push("draft".to_string());
                        }
                        if !pr.requested_reviewers.is_empty() {
                            facts.push(format!("waiting on {}", pr.requested_reviewers.join(", ")));
                        }
                        view! {
                            <li>
                                <a class="code" href=pr.url target="_blank" rel="noreferrer">
                                    {format!("#{}", pr.number)}
                                </a>
                                <span class="t">
                                    {pr.title} <br /> <span class="sub">{facts.join(", ")}</span>
                                </span>
                            </li>
                        }
                    })
                    .collect::<Vec<_>>()}
            </ul>
        }
    });

    let people = extra.filter(|r| !r.contributors.is_empty()).map(|remote| {
        let names = remote
            .contributors
            .iter()
            .take(6)
            .map(|c| format!("{} {}", c.login, c.contributions))
            .collect::<Vec<_>>()
            .join(", ");
        view! { <p class="soft">{format!("Contributors by commits: {names}")}</p> }
    });

    view! {
        <div class="sec">
            <h3>"CI and pull requests"</h3>
            <dl class="kv">{summary} {more}</dl>
            {pulls}
            {people}
        </div>
    }
    .into_any()
}

// ── From the launchpad ───────────────────────────────────────────────────────

/// Open tasks and flows from the repository's launchpad file, or how to
/// create one when there is none.
fn launchpad_sections(cx: Ctx, id: &str) -> AnyView {
    let Some(entry) = cx.entry(id) else {
        return view! {
            <div class="sec">
                <h3>"Open tasks"</h3>
                <p class="soft">"The launchpad has no entry for this repository."</p>
            </div>
        }
        .into_any();
    };

    let tasks = if entry.configured {
        let open = entry.open_tasks();
        let body = if open.is_empty() {
            view! { <p class="soft">"No open tasks."</p> }.into_any()
        } else {
            open.iter()
                .map(|task| tasks::task_row(task))
                .collect::<Vec<_>>()
                .into_any()
        };
        view! {
            <div class="sec tasks">
                <h3>
                    "Open tasks"
                    <span class="n">{open.len()}</span>
                </h3>
                {body}
            </div>
        }
        .into_any()
    } else {
        let path = cx.line(id).and_then(|line| line.path);
        let why = match &entry.invalid {
            Some(problem) => format!("This repository's launchpad file is invalid: {problem}"),
            None => "This repository has no launchpad file, so it lists no tasks.".to_string(),
        };
        view! {
            <div class="sec">
                <h3>"Open tasks"</h3>
                <p class="soft">{why}</p>
                {path
                    .map(|path| {
                        view! {
                            <p>"Set up with an agent"</p>
                            {ui::command(cx, tasks::setup_command(&path))}
                        }
                    })}
            </div>
        }
        .into_any()
    };

    let flows = tasks::all_flows(&entry);
    let run = (!flows.is_empty()).then(|| {
        let count = flows.len();
        let rows = flows
            .into_iter()
            .map(|(flow, declared)| {
                view! {
                    <div class="flow">
                        <div class="t">{tasks::flow_title(&flow, declared)}</div>
                        {ui::command(cx, flow.run.clone())}
                    </div>
                }
            })
            .collect::<Vec<_>>();
        view! {
            <div class="sec">
                <h3>
                    "Run"
                    <span class="n">{count}</span>
                </h3>
                {rows}
            </div>
        }
    });

    view! { {tasks} {run} }.into_any()
}
