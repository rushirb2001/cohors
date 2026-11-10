//! Running actions: fetch, pull, push, commit, stash and run.
//!
//! Every action takes the same path, so nothing ever runs unseen:
//!
//! 1. [`request`] asks the server for a dry run, which names the exact
//!    repositories the action would touch and changes nothing.
//! 2. If the server is read-only, a dialog says so and shows the flag that
//!    enables it. Otherwise a dialog lists the repositories and waits.
//! 3. On confirmation the action runs and its per-repository results are shown.
//!
//! The server enforces the gates; this module only explains them.

use leptos::prelude::*;
use serde_json::{Value, json};
use wasm_bindgen_futures::spawn_local;

use super::Ctx;
use super::ui::{self, Icon};
use crate::api;

/// What the dialog layer is showing.
#[derive(Clone)]
pub enum Dialog {
    /// Waiting for the dry run to name the repositories.
    Resolving { verb: String },
    /// The server will not run this verb: explain why and how to allow it.
    ReadOnly { verb: String, targets: Vec<String> },
    /// The repositories are known; waiting for the go-ahead.
    Confirm(Pending),
    /// The action is running on the server.
    Running { verb: String, count: usize },
    /// The action finished; one row per repository.
    Done(Outcome),
    /// Something went wrong before any repository was touched.
    Failed { title: String, message: String },
}

/// An action ready to run.
#[derive(Clone)]
pub struct Pending {
    verb: String,
    selector: Value,
    /// The names of the repositories it will touch, from the dry run.
    targets: Vec<String>,
}

/// A finished action.
#[derive(Clone)]
pub struct Outcome {
    verb: String,
    rows: Vec<ResultRow>,
}

/// How one repository fared.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowState {
    Ok,
    Skipped,
    Failed,
}

/// One repository's result.
#[derive(Clone)]
struct ResultRow {
    repo: String,
    state: RowState,
    /// The one-line outcome: git's message, or `exit 0` for `run`.
    message: String,
    /// Captured output (`run` only).
    stdout: String,
    stderr: String,
}

/// The selector for an explicit list of repositories.
pub fn ids_selector(ids: &[String]) -> Value {
    json!({ "ids": ids })
}

/// The selector for every repository.
pub fn all_selector() -> Value {
    json!({ "all": true })
}

/// Whether the server will run `verb`, read outside any reactive scope.
fn gate_open(cx: Ctx, verb: &str) -> bool {
    cx.meta.with_untracked(|m| {
        if verb == "run" {
            m.allow_run
        } else {
            m.allow_writes
        }
    })
}

/// The flag that opens the gate for `verb`.
fn gate_flag(verb: &str) -> &'static str {
    if verb == "run" {
        "--allow-run"
    } else {
        "--allow-writes"
    }
}

/// What a verb does, in one plain sentence, for the confirmation dialog.
fn describe(verb: &str) -> &'static str {
    match verb {
        "fetch" => {
            "Runs git fetch in each one. It updates what is known about the remote and changes no files."
        }
        "pull" => {
            "Runs git pull --ff-only in each one. A fast-forward cannot lose work, and a branch that cannot fast-forward is left as it is."
        }
        "push" => {
            "Runs git push in each one. A branch that was never pushed is published to origin and starts tracking it. It never force-pushes."
        }
        "commit" => "Stages every change in each one (git add -A) and commits it with one message.",
        "stash" => "Runs git stash push in each one, parking its tracked changes.",
        "run" => "Runs one shell command in the folder of each one and collects the output.",
        _ => "",
    }
}

/// Start an action: resolve its targets with a dry run, then either explain
/// that the server is read-only or ask for confirmation.
pub fn request(cx: Ctx, verb: &str, selector: Value) {
    let verb = verb.to_string();
    cx.dialog
        .set(Some(Dialog::Resolving { verb: verb.clone() }));
    spawn_local(async move {
        let mut preview = json!({ "verb": verb, "selector": selector, "dry_run": true });
        // The server wants these arguments even for a dry run; the real text
        // is typed into the confirmation dialog afterwards.
        if verb == "commit" {
            preview["message"] = json!("(preview)");
        }
        if verb == "run" {
            preview["command"] = json!("(preview)");
        }

        let answer = match api::post_action(preview).await {
            Ok(answer) => answer,
            Err(message) => return fail(cx, &verb, message),
        };
        if let Some(message) = answer.get("error").and_then(Value::as_str) {
            return fail(cx, &verb, message.to_string());
        }
        let targets: Vec<String> = answer
            .get("repos")
            .and_then(Value::as_array)
            .map(|repos| {
                repos
                    .iter()
                    .filter_map(|r| r.get("name").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if targets.is_empty() {
            return fail(
                cx,
                &verb,
                "None of these repositories can be acted on. A repository that could not be \
                 read, or has no folder on this machine, is left out."
                    .to_string(),
            );
        }

        let dialog = if gate_open(cx, &verb) {
            Dialog::Confirm(Pending {
                verb,
                selector,
                targets,
            })
        } else {
            Dialog::ReadOnly { verb, targets }
        };
        cx.dialog.set(Some(dialog));
    });
}

fn fail(cx: Ctx, verb: &str, message: String) {
    cx.dialog.set(Some(Dialog::Failed {
        title: format!("{} did not run", ui::capitalise(verb)),
        message,
    }));
}

/// Run a confirmed action. `text` is the commit message or shell command.
fn run(cx: Ctx, pending: Pending, text: String) {
    let Pending {
        verb,
        selector,
        targets,
    } = pending;
    let mut body = json!({ "verb": verb, "selector": selector });
    // Only the verbs the registry marks as needing a deliberate confirm carry it.
    if cohors_web::CONFIRM_VERBS.contains(&verb.as_str()) {
        body["confirm"] = json!(true);
    }
    if verb == "commit" {
        body["message"] = json!(text);
    }
    if verb == "run" {
        body["command"] = json!(text);
    }
    cx.dialog.set(Some(Dialog::Running {
        verb: verb.clone(),
        count: targets.len(),
    }));
    spawn_local(async move {
        let answer = api::post_action(body).await;
        match parse_outcome(&verb, answer) {
            Ok(outcome) => cx.dialog.set(Some(Dialog::Done(outcome))),
            Err(message) => fail(cx, &verb, message),
        }
        // Whatever happened, the repositories may have changed: look again.
        cx.refresh();
        cx.detail_epoch.update(|n| *n += 1);
    });
}

/// Turn the server's answer into rows. Git verbs answer
/// `results: [{repo, ok, message, skipped}]`; `run` answers
/// `results: [{repo, ok, exit_code, stdout, stderr, timed_out}]`.
fn parse_outcome(verb: &str, answer: Result<Value, String>) -> Result<Outcome, String> {
    let answer = answer?;
    if let Some(message) = answer.get("error").and_then(Value::as_str) {
        return Err(message.to_string());
    }
    let text = |row: &Value, key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let flag = |row: &Value, key: &str| row.get(key).and_then(Value::as_bool).unwrap_or(false);

    let rows = answer
        .get("results")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|row| {
            let state = if flag(row, "skipped") {
                RowState::Skipped
            } else if flag(row, "ok") {
                RowState::Ok
            } else {
                RowState::Failed
            };
            let message = if verb != "run" {
                text(row, "message")
            } else if flag(row, "timed_out") {
                "timed out".to_string()
            } else {
                let code = row.get("exit_code").and_then(Value::as_i64).unwrap_or(-1);
                format!("exit {code}")
            };
            ResultRow {
                // The id of a scanned repository is its path; show its name.
                repo: ui::last_component(&text(row, "repo")).to_string(),
                state,
                message,
                stdout: text(row, "stdout"),
                stderr: text(row, "stderr"),
            }
        })
        .collect();
    Ok(Outcome {
        verb: verb.to_string(),
        rows,
    })
}

/// Close the dialog, unless an action is mid-flight (its result must be seen).
pub fn dismiss(cx: Ctx) {
    let running = cx
        .dialog
        .with_untracked(|d| matches!(d, Some(Dialog::Running { .. })));
    if !running {
        cx.dialog.set(None);
    }
}

/// A button that starts `verb` over the repositories `selector` names when it
/// is clicked. On a read-only server it stays visible, carries a lock, says
/// why on hover, and explains in full on click.
pub fn verb_button(
    cx: Ctx,
    verb: &'static str,
    label: String,
    class: &'static str,
    icon: Option<Icon>,
    selector: impl Fn() -> Value + 'static,
) -> impl IntoView {
    let hint = move || {
        (!cx.allowed(verb)).then(|| {
            format!(
                "The server is read-only. Start it with cohors web {} to run this.",
                gate_flag(verb)
            )
        })
    };
    view! {
        <button
            class=format!("btn {class}")
            title=hint
            on:click=move |ev| {
                // The button may sit inside a clickable row.
                ev.stop_propagation();
                request(cx, verb, selector());
            }
        >
            // The lock replaces the button's own icon while the gate is shut.
            {move || {
                if cx.allowed(verb) {
                    icon.map(ui::icon).into_any()
                } else {
                    view! { <span class="lock">{ui::icon(Icon::Lock)}</span> }.into_any()
                }
            }}
            <span>{label}</span>
        </button>
    }
}

/// One note for a whole row of verb buttons on a read-only server, so the row
/// does not repeat a lock on every button.
pub fn read_only_note(cx: Ctx) -> impl IntoView {
    move || {
        (!cx.allowed("fetch")).then(|| {
            view! {
                <span
                    class="soft readonly"
                    title="Actions are previewed, not run. Start the server with cohors web --allow-writes to run them."
                >
                    {ui::icon(Icon::Lock)}
                    <span>"Read-only"</span>
                </span>
            }
        })
    }
}

/// The dialog layer: at most one dialog, over a scrim.
pub fn dialogs(cx: Ctx) -> impl IntoView {
    move || {
        cx.dialog.get().map(|dialog| {
            let body = match dialog {
                Dialog::Resolving { verb } => {
                    working(format!("Checking which repositories {verb} would touch")).into_any()
                }
                Dialog::Running { verb, count } => working(format!(
                    "Running {verb} on {}",
                    ui::plural(count, "repository", "repositories")
                ))
                .into_any(),
                Dialog::ReadOnly { verb, targets } => read_only(cx, verb, targets).into_any(),
                Dialog::Confirm(pending) => confirm(cx, pending).into_any(),
                Dialog::Done(outcome) => done(cx, outcome).into_any(),
                Dialog::Failed { title, message } => failed(cx, title, message).into_any(),
            };
            view! {
                <div class="scrim" on:click=move |_| dismiss(cx)>
                    {body}
                </div>
            }
        })
    }
}

/// The dialog frame: a title, a body, and a row of buttons. A click inside
/// must not reach the scrim, which would close the dialog.
fn frame(
    title: String,
    wide: bool,
    content: impl IntoView,
    buttons: impl IntoView,
) -> impl IntoView {
    let class = if wide { "dialog wide" } else { "dialog" };
    let label = title.clone();
    view! {
        <div
            class=class
            role="dialog"
            aria-modal="true"
            aria-label=label
            on:click=|ev| ev.stop_propagation()
        >
            <header>
                <h2>{title}</h2>
            </header>
            <div class="content">{content}</div>
            <footer>{buttons}</footer>
        </div>
    }
}

fn close_button(cx: Ctx, label: &'static str, class: &'static str) -> impl IntoView {
    view! {
        <button class=format!("btn {class}") on:click=move |_| cx.dialog.set(None)>
            {label}
        </button>
    }
}

/// A dialog with nothing to press: the server is working.
fn working(what: String) -> impl IntoView {
    view! {
        <div class="dialog" role="status" on:click=|ev| ev.stop_propagation()>
            <header>
                <h2>
                    <span class="spin"></span>
                    {what}
                </h2>
            </header>
        </div>
    }
}

/// The list of repository names an action applies to.
fn target_list(targets: Vec<String>) -> impl IntoView {
    view! {
        <ul class="targets">
            {targets.into_iter().map(|name| view! { <li>{name}</li> }).collect::<Vec<_>>()}
        </ul>
    }
}

/// The server is read-only: nothing ran. Say why, show the flag that allows
/// it, and list what the action would have touched.
fn read_only(cx: Ctx, verb: String, targets: Vec<String>) -> impl IntoView {
    let flag = gate_flag(&verb);
    let allows = if verb == "run" {
        "To allow shell commands, stop the server and start it again with:"
    } else {
        "To allow fetch, pull, push, commit and stash, stop the server and start it again with:"
    };
    let would = format!(
        "{} would have run on {}:",
        ui::capitalise(&verb),
        ui::plural(targets.len(), "repository", "repositories")
    );
    let content = view! {
        <p>
            {format!(
                "{} was not run. cohors web starts read-only, so nothing in the browser can \
                 change your repositories until you allow it.",
                ui::capitalise(&verb),
            )}
        </p>
        <p>{allows}</p>
        {ui::command(cx, format!("cohors web {flag}"))}
        <p class="soft">{would}</p>
        {target_list(targets)}
    };
    frame(
        "This server is read-only".to_string(),
        false,
        content,
        close_button(cx, "Close", "primary"),
    )
}

/// Ask for the go-ahead, listing every repository the action will touch.
/// Commit and run also need their text.
fn confirm(cx: Ctx, pending: Pending) -> impl IntoView {
    let verb = pending.verb.clone();
    let count = ui::plural(pending.targets.len(), "repository", "repositories");
    let title = format!("{} {count}", ui::capitalise(&verb));
    // What the text box is for, when the verb needs one.
    let asks_for = match verb.as_str() {
        "commit" => Some("Commit message"),
        "run" => Some("Shell command"),
        _ => None,
    };
    let text = RwSignal::new(String::new());
    let targets = pending.targets.clone();
    let go_label = format!("{} {}", ui::capitalise(&verb), pending.targets.len());

    let go = move |_| {
        let typed = text.get_untracked().trim().to_string();
        if asks_for.is_some() && typed.is_empty() {
            return;
        }
        run(cx, pending.clone(), typed);
    };

    let content = view! {
        <p>{describe(&verb)}</p>
        {target_list(targets)}
        {asks_for
            .map(|label| {
                view! {
                    <label>
                        <span class="sr">{label}</span>
                        <input
                            class="field"
                            type="text"
                            placeholder=label
                            prop:value=move || text.get()
                            on:input=move |ev| text.set(event_target_value(&ev))
                        />
                    </label>
                }
            })}
    };
    let buttons = view! {
        {close_button(cx, "Cancel", "")}
        <button
            class="btn primary"
            // Without its text, commit / run cannot go.
            disabled=move || asks_for.is_some() && text.with(|t| t.trim().is_empty())
            on:click=go
        >
            {go_label}
        </button>
    };
    frame(title, false, content, buttons)
}

/// The action finished: one row per repository.
fn done(cx: Ctx, outcome: Outcome) -> impl IntoView {
    let count = |state| outcome.rows.iter().filter(|r| r.state == state).count();
    let mut summary = format!("{} done", count(RowState::Ok));
    if count(RowState::Skipped) > 0 {
        summary.push_str(&format!(", {} skipped", count(RowState::Skipped)));
    }
    if count(RowState::Failed) > 0 {
        summary.push_str(&format!(", {} failed", count(RowState::Failed)));
    }
    let title = format!("{}: {summary}", ui::capitalise(&outcome.verb));
    let is_run = outcome.verb == "run";
    let content = if is_run {
        run_results(outcome.rows).into_any()
    } else {
        git_results(outcome.rows).into_any()
    };
    frame(title, is_run, content, close_button(cx, "Close", "primary"))
}

/// The mark for a result row: a dot for done, a ring for skipped, a triangle
/// for failed.
fn row_mark(state: RowState) -> impl IntoView {
    match state {
        RowState::Ok => ui::mark("ok", "Done"),
        RowState::Skipped => ui::mark("note", "Skipped"),
        RowState::Failed => ui::mark("risk", "Failed"),
    }
}

/// Results of a git verb: mark, repository, git's own message.
fn git_results(rows: Vec<ResultRow>) -> impl IntoView {
    view! {
        <ul class="results">
            {rows
                .into_iter()
                .map(|row| {
                    view! {
                        <li>
                            {row_mark(row.state)}
                            <span class="name clip">{row.repo}</span>
                            <span class="msg">{row.message}</span>
                        </li>
                    }
                })
                .collect::<Vec<_>>()}
        </ul>
    }
}

/// Results of `run`: the repositories on the left, the chosen one's captured
/// output on the right.
fn run_results(rows: Vec<ResultRow>) -> impl IntoView {
    let chosen = RwSignal::new(0usize);
    let list = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            view! {
                <li>
                    <button
                        aria-pressed=move || if chosen.get() == index { "true" } else { "false" }
                        on:click=move |_| chosen.set(index)
                    >
                        {row_mark(row.state)}
                        <span class="clip">{row.repo.clone()}</span>
                        <span class="sub">{row.message.clone()}</span>
                    </button>
                </li>
            }
        })
        .collect::<Vec<_>>();

    let output = move || {
        rows.get(chosen.get()).cloned().map(|row| {
            let quiet = row.stdout.is_empty() && row.stderr.is_empty();
            view! {
                <p>
                    <b>{row.repo}</b>
                    <span class="soft">{format!(", {}", row.message)}</span>
                </p>
                {(!row.stdout.is_empty()).then(|| view! { <pre>{row.stdout}</pre> })}
                {(!row.stderr.is_empty())
                    .then(|| {
                        view! {
                            <p class="soft">"Standard error"</p>
                            <pre>{row.stderr}</pre>
                        }
                    })}
                {quiet.then(|| view! { <p class="soft">"The command printed nothing."</p> })}
            }
        })
    };

    view! {
        <div class="run-panes">
            <ul class="run-list">{list}</ul>
            <div class="run-out">{output}</div>
        </div>
    }
}

/// Something failed before any repository was touched.
fn failed(cx: Ctx, title: String, message: String) -> impl IntoView {
    frame(
        title,
        false,
        view! { <p>{message}</p> },
        close_button(cx, "Close", "primary"),
    )
}
