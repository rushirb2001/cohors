//! `cohors watch` — the fleet as a stream of changes.
//!
//! One long-lived process that prints the fleet once it has been read, then
//! again each time something a front-end shows has changed. Piped, or with
//! `--json`, every line is one JSON object (NDJSON), which is how the editor
//! extension follows the fleet without starting a process per refresh:
//!
//! ```text
//! {"type":"fleet","seq":3,"reason":"git","changed":["/code/api"],
//!  "overview":{…what `cohors overview --json` prints…},
//!  "launchpad":{…what `cohors launchpad --json` prints…},
//!  "suggestions":[{"kind","title","why","command","repo","score","session"?}]}
//! ```
//!
//! The changes come from the fleet watch (`cohors_fleet::FleetWatch`), the
//! same change source the dashboard and `cohors web` use, and a line is
//! printed only when its content differs from the last one: with nothing
//! changed the stream is silent. The one exception is time: every five
//! minutes the line is worked out again, because ages ("3h") and judgments
//! such as a stash turning stale move with the clock.
//!
//! It runs until interrupted, until its standard input closes when another
//! program started it through a pipe (that program has gone), or until the
//! reader of its output goes away (a closed pipe ends it quietly, with
//! success).

use std::io::{ErrorKind, IsTerminal, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use cohors_fleet::{FleetState, FleetWatch, WatchOptions};
use serde_json::{Value, json};

use super::{bold, dim, now_secs, wants_color};
use crate::cli::Cli;

/// With nothing changed, work the line out again this often: ages and
/// time-based judgments move with the clock. The web page redraws on the
/// same interval.
const TIME_CHECK: Duration = Duration::from_secs(300);

/// `cohors watch [--json]`.
pub fn run(cli: &Cli, json: bool) -> Result<()> {
    let human = !json && std::io::stdout().is_terminal();
    let color = wants_color(human);
    let scanner = Arc::new(crate::scan::from_cli(cli)?);
    end_when_input_closes();

    let watch = FleetWatch::start(scanner, WatchOptions::default());
    let subscription = watch.subscribe();
    let here = std::env::current_dir().ok();
    let user_home = cohors_config::paths::home_dir()
        .ok()
        .map(|p| p.into_std_path_buf());

    let mut out = std::io::stdout().lock();
    let mut state: Option<Arc<FleetState>> = None;
    let mut printed: Option<Value> = None;
    loop {
        let fresh = match subscription.recv_timeout(TIME_CHECK) {
            Ok(newer) => {
                state = Some(newer);
                true
            }
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => bail!("the fleet watch stopped"),
        };
        let Some(state) = &state else { continue };
        let line = Line::of(state, here.as_deref(), user_home.as_deref(), now_secs());
        // Only what a reader shows decides whether to print: not the clock,
        // and not which event happened to bring it.
        if printed.as_ref() == Some(&line.content) {
            continue;
        }
        let text = if human {
            line.human(state, fresh, color)
        } else {
            let mut json = serde_json::to_string(&line.json(state, fresh))
                .context("serializing a watch line")?;
            json.push('\n');
            json
        };
        match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
            Ok(()) => printed = Some(line.content),
            // The reader went away (`cohors watch --json | head -1`, or an
            // editor that closed): that is the end of the stream, not an error.
            Err(err) if err.kind() == ErrorKind::BrokenPipe => return Ok(()),
            Err(err) => return Err(err).context("writing to standard output"),
        }
    }
}

/// One state of the fleet, as the stream reports it.
struct Line {
    overview: Value,
    launchpad: Value,
    suggestions: Value,
    /// What decides whether it is printed: everything above, minus the clock.
    content: Value,
}

impl Line {
    fn of(state: &FleetState, here: Option<&Path>, user_home: Option<&Path>, now: i64) -> Self {
        let here = here.and_then(|dir| cohors_fleet::repo_containing(&state.snapshots, dir));
        let bundle = &state.launchpad;
        let view = cohors_fleet::live_view(
            &state.snapshots,
            &bundle.entries,
            &bundle.board,
            here.as_deref(),
            user_home,
            now,
        );
        // Exactly what `cohors overview --json` prints: the model plus the
        // roots, so a reader can say where it looked.
        let mut overview = serde_json::to_value(&view.overview).unwrap_or(Value::Null);
        if let Value::Object(map) = &mut overview {
            map.insert("roots".to_string(), json!(state.scanner.roots()));
        }
        // And exactly what `cohors launchpad --json` prints.
        let launchpad = json!({
            "summary": super::launchpad::summary_counts(&bundle.entries),
            "repos": bundle.entries,
            "board": bundle.board,
            "recent_activity": bundle.recent_activity,
        });
        let suggestions = json!(view.suggestions);
        let mut clockless = overview.clone();
        if let Value::Object(map) = &mut clockless {
            map.remove("now");
        }
        let content = json!({
            "overview": clockless,
            "launchpad": launchpad,
            "suggestions": suggestions,
        });
        Self {
            overview,
            launchpad,
            suggestions,
            content,
        }
    }

    /// The NDJSON object. `fresh` is false when only the clock moved.
    fn json(&self, state: &FleetState, fresh: bool) -> Value {
        json!({
            "type": "fleet",
            "seq": state.seq,
            "reason": if fresh { json!(state.reason) } else { json!("time") },
            "changed": if fresh { json!(state.changed) } else { json!([]) },
            "overview": self.overview,
            "launchpad": self.launchpad,
            "suggestions": self.suggestions,
        })
    }

    /// One or two lines for a person at a terminal: the headline, and which
    /// repositories changed.
    fn human(&self, state: &FleetState, fresh: bool, color: bool) -> String {
        let headline = self.overview["headline"].as_str().unwrap_or_default();
        let reason = if !fresh {
            "time".to_string()
        } else {
            serde_json::to_value(state.reason)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        };
        let mut text = format!(
            "  {}  {}\n",
            dim(color, &format!("{reason:<9}")),
            // Repository-authored text (names in the headline): no escapes.
            bold(color, &crate::ui::text::safe(headline))
        );
        if fresh && state.seq > 1 && !state.changed.is_empty() {
            let names: Vec<String> = state
                .changed
                .iter()
                .map(|id| {
                    state
                        .snapshots
                        .iter()
                        .find(|s| &s.id.0 == id)
                        .map(|s| s.name.clone())
                        // Gone from the fleet: its folder's name.
                        .unwrap_or_else(|| id.rsplit('/').next().unwrap_or(id).to_string())
                })
                .collect();
            let names = crate::ui::text::safe(&names.join(", "));
            text.push_str(&format!("  {:<9}  {}\n", "", dim(color, &names)));
        }
        text
    }
}

/// When another program started this one through a pipe, it closing that
/// pipe means it has gone (an editor window closed, a script ended): stop
/// with it rather than linger. A terminal, or input that is a file or
/// `/dev/null` (`nohup`, a service), says nothing about that and is left
/// alone; Ctrl-C or a signal ends the stream there.
fn end_when_input_closes() {
    if !input_is_a_pipe() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("cohors-watch-stdin".to_string())
        .spawn(|| {
            let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
            std::process::exit(0);
        });
}

#[cfg(unix)]
fn input_is_a_pipe() -> bool {
    use std::os::unix::fs::FileTypeExt;
    // `/dev/stdin` names this process's standard input on macOS and Linux.
    std::fs::metadata("/dev/stdin")
        .is_ok_and(|m| m.file_type().is_fifo() || m.file_type().is_socket())
}

#[cfg(not(unix))]
fn input_is_a_pipe() -> bool {
    !std::io::stdin().is_terminal()
}
