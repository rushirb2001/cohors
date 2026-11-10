//! Small shared pieces every page uses: icons, the brand mark, the severity
//! marks, copyable commands, and a few text helpers.

use cohors_core::{Severity, time};
use leptos::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};

use super::Ctx;

// ── Icons ────────────────────────────────────────────────────────────────────

/// The icon set: minimal outlines on a 24-unit grid, drawn in the current text
/// colour (see `.ic` in style.css).
#[derive(Clone, Copy)]
pub enum Icon {
    Grid,
    Folder,
    Check,
    Calendar,
    Alert,
    Upload,
    Layers,
    File,
    Refresh,
    Commit,
    Branch,
    Bookmark,
    Clock,
    Bolt,
    Copy,
    Close,
    Sun,
    Moon,
    Lock,
    Menu,
    Arrow,
    Terminal,
    Flask,
    Chevron,
}

impl Icon {
    /// The SVG path data for this icon.
    fn path(self) -> &'static str {
        match self {
            Icon::Grid => "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z",
            Icon::Folder => {
                "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"
            }
            Icon::Check => "M4 5h16v14H4zM8 12l3 3 5-6",
            Icon::Calendar => "M4 6h16v14H4zM4 10h16M8 3v4M16 3v4",
            Icon::Alert => "M12 4l9 16H3zM12 10v5M12 17.5v.5",
            Icon::Upload => "M12 16V5M7 10l5-5 5 5M5 20h14",
            Icon::Layers => "M12 4l9 5-9 5-9-5zM3 14l9 5 9-5",
            Icon::File => "M6 3h8l4 4v14H6zM14 3v4h4",
            Icon::Refresh => "M20 12a8 8 0 1 1-2.3-5.7M20 4v5h-5",
            Icon::Commit => "M3 12h6M15 12h6M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z",
            Icon::Branch => {
                "M7 8v8M7 8a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM7 20a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM17 10a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM17 10c0 4-10 2-10 6"
            }
            Icon::Bookmark => "M7 4h10v16l-5-4-5 4z",
            Icon::Clock => "M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16zM12 8v4l3 2",
            Icon::Bolt => "M13 3L5 14h6l-1 7 8-11h-6z",
            Icon::Copy => "M9 9h11v11H9zM5 15V4h11",
            Icon::Close => "M6 6l12 12M18 6L6 18",
            Icon::Sun => {
                "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6L7 7M17 17l1.4 1.4M5.6 18.4L7 17M17 7l1.4-1.4"
            }
            Icon::Moon => "M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z",
            Icon::Lock => "M6 11h12v9H6zM8 11V8a4 4 0 0 1 8 0v3",
            Icon::Menu => "M4 7h16M4 12h16M4 17h16",
            Icon::Arrow => "M12 19V5M6 11l6-6 6 6",
            Icon::Terminal => "M4 5h16v14H4zM8 10l3 2-3 2M13 15h4",
            Icon::Flask => "M10 3h4M10 3v6l-5 9a2 2 0 0 0 2 3h10a2 2 0 0 0 2-3l-5-9V3M8 15h8",
            Icon::Chevron => "M9 6l6 6-6 6",
        }
    }
}

/// An icon, sized by `.ic` in style.css.
pub fn icon(which: Icon) -> impl IntoView {
    view! {
        <svg class="ic" viewBox="0 0 24 24" aria-hidden="true">
            <path d=which.path() />
        </svg>
    }
}

// ── Brand mark ───────────────────────────────────────────────────────────────

/// The cohors spider, as three rows of quadrant-block characters: the same
/// art the terminal prints. Each character is a 2×2 grid of quadrants.
const SPIDER: [&str; 3] = ["▜▒▟███▙▒▛", "▟██▌█▐██▙", "▀▐▖▀█▀▗▌▀"];

/// Which quadrants of a block character are filled, as
/// `[upper-left, upper-right, lower-left, lower-right]`.
fn quadrants(block: char) -> [bool; 4] {
    match block {
        '█' | '▒' => [true, true, true, true],
        '▜' => [true, true, false, true],
        '▟' => [false, true, true, true],
        '▙' => [true, false, true, true],
        '▛' => [true, true, true, false],
        '▌' => [true, false, true, false],
        '▐' => [false, true, false, true],
        '▀' => [true, true, false, false],
        '▖' => [false, false, true, false],
        '▗' => [false, false, false, true],
        _ => [false; 4],
    }
}

/// The brand mark as an SVG: one small rectangle per filled quadrant. The
/// shaded `▒` cells are drawn at reduced opacity.
pub fn brand_mark() -> impl IntoView {
    let mut rects = Vec::new();
    for (row, line) in SPIDER.iter().enumerate() {
        for (col, block) in line.chars().enumerate() {
            let opacity = if block == '▒' { "0.45" } else { "1" };
            // Offsets of the four quadrants inside one character cell, which
            // is 2 units wide and 4 tall.
            let offsets = [(0, 0), (1, 0), (0, 2), (1, 2)];
            for (filled, (dx, dy)) in quadrants(block).into_iter().zip(offsets) {
                if filled {
                    let x = (col * 2 + dx).to_string();
                    let y = (row * 4 + dy).to_string();
                    // Slightly oversized so neighbours overlap and no hairline
                    // gap shows between them.
                    rects.push(view! {
                        <rect x=x y=y width="1.04" height="2.04" opacity=opacity />
                    });
                }
            }
        }
    }
    view! {
        <svg viewBox="0 0 18 12" fill="currentColor" aria-hidden="true">
            {rects}
        </svg>
    }
}

// ── Severity marks ───────────────────────────────────────────────────────────

/// The CSS class of the mark for a severity: a triangle for risk, a diamond
/// for a warning, a ring for a notice, a small dot for calm.
pub fn severity_class(severity: Severity) -> &'static str {
    match severity {
        Severity::Risk => "risk",
        Severity::Warn => "warn",
        Severity::Notice => "note",
        // The model counts `Info` as calm (see `FleetSummary::calm`).
        Severity::Info | Severity::Ok => "ok",
    }
}

/// The severity in words, for screen readers and tooltips.
pub fn severity_word(severity: Severity) -> &'static str {
    match severity {
        Severity::Risk => "At risk",
        Severity::Warn => "Warning",
        Severity::Notice => "Notice",
        Severity::Info | Severity::Ok => "Calm",
    }
}

/// A shape mark. `class` picks the shape (see the `.mark` rules in style.css)
/// and `label` says what it means, so the shape is never the only signal.
pub fn mark(class: &'static str, label: &'static str) -> impl IntoView {
    view! { <span class=format!("mark {class}") role="img" aria-label=label title=label></span> }
}

/// The mark for a repository's overall severity.
pub fn severity_mark(severity: Severity) -> impl IntoView {
    mark(severity_class(severity), severity_word(severity))
}

/// The severity block that leads a row in the lists a person scans for
/// trouble (Do next, Work at risk, the repository table): a filled `RISK` or
/// `WARN`, a quiet outlined `NOTE` or `INFO`, and an empty slot of the same
/// width for a clean repository, so names stay in one column. The word is the
/// signal; the colour only reinforces it. The terminal dashboard and the
/// editor extension draw the same block.
pub fn severity_block(severity: Severity) -> impl IntoView {
    let (class, word) = match severity {
        Severity::Risk => ("risk", "RISK"),
        Severity::Warn => ("warn", "WARN"),
        Severity::Notice => ("note", "NOTE"),
        Severity::Info => ("info", "INFO"),
        Severity::Ok => ("none", ""),
    };
    view! { <span class=format!("blk {class}") title=severity_word(severity)>{word}</span> }
}

// ── Copyable commands ────────────────────────────────────────────────────────

/// Copy `text` to the clipboard and confirm it (or say that it failed) in the
/// toast. `what` names the thing copied, e.g. `"command"`.
pub fn copy_text(cx: Ctx, text: String, what: &'static str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let promise = window.navigator().clipboard().write_text(&text);
    spawn_local(async move {
        match JsFuture::from(promise).await {
            Ok(_) => cx.say(format!("Copied the {what}.")),
            Err(_) => cx.say(format!(
                "The browser refused to copy the {what}. Select it and copy it by hand."
            )),
        }
    });
}

/// A command the page wrote itself, in a monospace box with a Copy button.
///
/// The user pastes it into a terminal, so any part that comes from the fleet
/// (a path, a name, an id) must already have gone through
/// [`cohors_web::shell_quote`] before it gets here.
pub fn command(cx: Ctx, text: String) -> impl IntoView {
    command_box(cx, text, "command")
}

/// A command a repository wrote: the `run` line of a flow from its launchpad
/// file or its manifests. It is shown exactly as written, because it *is* the
/// command, and it is only ever copied, never run by the page. The caller
/// labels where it came from.
pub fn repo_command(cx: Ctx, text: String) -> impl IntoView {
    command_box(cx, text, "repository's command")
}

fn command_box(cx: Ctx, text: String, what: &'static str) -> impl IntoView {
    let shown = text.clone();
    let tip = text.clone();
    view! {
        <div class="cmd">
            <code title=tip>{shown}</code>
            <button class="btn small" on:click=move |_| copy_text(cx, text.clone(), what)>
                "Copy"
            </button>
        </div>
    }
}

// ── Text helpers ─────────────────────────────────────────────────────────────

/// `1 file` / `3 files`.
pub fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// How long ago `timestamp` was, as `3d ago` (or `just now`).
pub fn ago(timestamp: i64, now: i64) -> String {
    match time::relative(timestamp, now).as_str() {
        "now" => "just now".to_string(),
        age => format!("{age} ago"),
    }
}

/// A Unix timestamp in the browser's local time, e.g. `10 Nov, 09:40`.
pub fn local_time(timestamp: i64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(timestamp as f64 * 1000.0));
    let month = MONTHS
        .get(date.get_month() as usize)
        .copied()
        .unwrap_or("?");
    format!(
        "{} {month}, {:02}:{:02}",
        date.get_date(),
        date.get_hours(),
        date.get_minutes()
    )
}

/// Shorten a path under a home directory to `~/…`, the way a shell prompt
/// does. The browser cannot ask for `$HOME`, so this recognises the two usual
/// layouts and leaves anything else untouched.
pub fn short_path(path: &str) -> String {
    for prefix in ["/Users/", "/home/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            return match rest.split_once('/') {
                Some((_, tail)) => format!("~/{tail}"),
                None => "~".to_string(),
            };
        }
    }
    path.to_string()
}

/// The last component of a path or repository id: `/a/b/cohors` → `cohors`.
pub fn last_component(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
}

/// A verb with its first letter capitalised: `fetch` → `Fetch`.
pub fn capitalise(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
