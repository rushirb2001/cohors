//! cohors web — the fleet rendered in the browser, proving `cohors-core` drives
//! the UI through WebAssembly (the browser analog of `cohors demo`).
//!
//! Slice 1 of v0.5: no GitHub, no auth, no server — just the pure core
//! (`demo::fleet` + `resolve`/sort + `assess`/attention + `time::relative`)
//! compiled to WASM and shown with Leptos. Later slices add a WASM GitHub
//! client, OAuth, and deployment. Everything here that decides *what to show* and
//! *in what order* is the exact same code the TUI runs (ADR-002).

use std::collections::HashMap;

use cohors_core::{
    Branch, RepoSnapshot, Selector, Severity, SortMode, assess, resolve, time,
};
use leptos::prelude::*;

/// A fixed clock for the demo fleet. The core takes `now` injected (ADR-010), so
/// relative ages render deterministically; a live clock arrives with real data.
const NOW: i64 = 1_700_000_000;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let snaps = cohors_core::demo::fleet(NOW);

    // Order exactly like the TUI: the shared selector (everything) + dirty-first
    // sort, resolved by `cohors-core`.
    let order = resolve(
        &snaps,
        &Selector {
            all: true,
            ..Default::default()
        },
        SortMode::DirtyFirst,
        NOW,
    );
    let by_id: HashMap<&str, &RepoSnapshot> =
        snaps.iter().map(|s| (s.id.0.as_str(), s)).collect();
    let rows: Vec<_> = order
        .iter()
        .filter_map(|id| by_id.get(id.0.as_str()).copied())
        .map(repo_row)
        .collect();
    let count = snaps.len();

    view! {
        <main class="app">
            <div class="brand">
                <span class="name">"cohors"</span>
                <span class="tag">"All your git repositories at a glance"</span>
            </div>
            <table class="fleet">
                <thead>
                    <tr>
                        <th>"Repo"</th>
                        <th>"Branch"</th>
                        <th>"Sync"</th>
                        <th>"Changes"</th>
                        <th>"Last commit"</th>
                        <th>"Status"</th>
                    </tr>
                </thead>
                <tbody>{rows}</tbody>
            </table>
            <footer>
                {format!("Demo fleet · {count} repositories · sorted & scored by cohors-core in WebAssembly")}
            </footer>
        </main>
    }
}

/// One fleet row, built entirely from `cohors-core` data + judgments. The row
/// clones what it needs into owned strings, so `+ use<>` (precise capturing)
/// tells the compiler the returned view borrows nothing — it's `'static`, which
/// Leptos requires.
fn repo_row(s: &RepoSnapshot) -> impl IntoView + use<> {
    let a = assess(s, NOW);

    let name = s.name.clone();
    let name_class = if s.error.is_some() {
        "name error"
    } else if matches!(a.severity, Severity::Ok | Severity::Info) {
        "name clean"
    } else {
        "name"
    };

    let branch = match &s.branch {
        Branch::Named(b) => b.clone(),
        Branch::Detached(id) => format!("@{}", id.chars().take(7).collect::<String>()),
        Branch::Unborn => "unborn".to_string(),
    };

    // Sync: ahead/behind vs upstream.
    let (sync, sync_class) = match &s.upstream {
        None => ("—".to_string(), "dim"),
        Some(up) if up.ahead == 0 && up.behind == 0 => ("·".to_string(), "dim"),
        Some(up) => {
            let mut p = String::new();
            if up.ahead > 0 {
                p.push_str(&format!("↑{}", up.ahead));
            }
            if up.behind > 0 {
                if !p.is_empty() {
                    p.push(' ');
                }
                p.push_str(&format!("↓{}", up.behind));
            }
            (p, if up.behind > 0 { "behind" } else { "ahead" })
        }
    };

    // Changes: file count (+ stash), coloured by staged vs unstaged.
    let w = &s.worktree;
    let total = w.staged + w.modified + w.untracked;
    let (changes, changes_class) = if s.error.is_some() || total == 0 {
        ("·".to_string(), "dim")
    } else {
        let cls = if w.modified > 0 || w.untracked > 0 {
            "modified"
        } else {
            "staged"
        };
        let mut t = total.to_string();
        if s.stash_count > 0 {
            t.push_str(&format!(" s{}", s.stash_count));
        }
        (t, cls)
    };

    // Status: the primary attention reason (or the error), coloured by severity.
    let (status, status_class) = if let Some(e) = &s.error {
        (e.clone(), "status risk")
    } else if let Some(r) = &a.primary {
        let cls = match r.severity() {
            Severity::Risk => "status risk",
            Severity::Warn => "status warn",
            Severity::Notice => "status notice",
            _ => "status",
        };
        (r.label(), cls)
    } else {
        ("clean".to_string(), "status")
    };

    let (age, summary) = s
        .last_commit
        .as_ref()
        .map(|c| (time::relative(c.timestamp, NOW), c.summary.clone()))
        .unwrap_or_else(|| ("—".to_string(), String::new()));

    view! {
        <tr>
            <td class=name_class>{name}</td>
            <td class="dim">{branch}</td>
            <td class=sync_class>{sync}</td>
            <td class=changes_class>{changes}</td>
            <td class="commit">
                <span class="age">{age}</span>
                {summary}
            </td>
            <td class=status_class>{status}</td>
        </tr>
    }
}
