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
//!  "suggestions":[{"kind","title","why","command","repo","score","session"?}],
//!  "update":{"version","notice","detail"} | null}
//! ```
//!
//! `update` is a newer cohors, when one is known, in the words every surface
//! uses (the MCP server's `meta.update` has the same shape). Like the MCP
//! server, the stream looks for releases in the background and only reports
//! them: installing is left to the dashboard or `cohors update`.
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
use cohors_core::update::UpdateState;
use cohors_fleet::{FleetState, FleetWatch, WatchOptions};
use serde_json::{Value, json};

use super::{bold, dim, now_secs, wants_color};
use crate::cli::Cli;

/// With nothing changed, work the line out again this often: ages and
/// time-based judgments move with the clock. The web page redraws on the
/// same interval.
const TIME_CHECK: Duration = Duration::from_secs(300);

/// How often the stream looks whether the background look found a newer
/// cohors meanwhile: it is written down in one small file, so this is cheap,
/// and a release shows up in the stream within seconds of being found.
const UPDATE_CHECK: Duration = Duration::from_secs(10);

/// `cohors watch [--json]`.
pub fn run(cli: &Cli, json: bool) -> Result<()> {
    let human = !json && std::io::stdout().is_terminal();
    let color = wants_color(human);
    let scanner = Arc::new(crate::scan::from_cli(cli)?);
    end_when_input_closes();

    // Look for a newer cohors now and every few minutes, as the MCP server
    // does, so an editor that only follows this stream hears about releases.
    // Report only: a stream another program started never installs anything.
    let mode = crate::update::effective_mode(scanner.updates_mode());
    crate::update::watch(mode, false, |_| true);
    let wait = if mode == cohors_config::UpdatesMode::Off {
        TIME_CHECK
    } else {
        UPDATE_CHECK
    };

    let watch = FleetWatch::start(scanner, WatchOptions::default());
    let subscription = watch.subscribe();
    let here = std::env::current_dir().ok();
    let user_home = cohors_config::paths::home_dir()
        .ok()
        .map(|p| p.into_std_path_buf());

    let mut out = std::io::stdout().lock();
    let mut state: Option<Arc<FleetState>> = None;
    let mut printed: Option<Value> = None;
    let mut update = crate::update::remembered(mode);
    // When the line was last worked out; ages move with the clock, so it is
    // worked out again at least every TIME_CHECK.
    let mut worked_out = std::time::Instant::now();
    loop {
        let fresh = match subscription.recv_timeout(wait) {
            Ok(newer) => {
                state = Some(newer);
                true
            }
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => bail!("the fleet watch stopped"),
        };
        let newest = crate::update::remembered(mode);
        let update_moved = newest != update;
        update = newest;
        // Woken only to look at the update: nothing to do unless it moved or
        // the clock is due.
        if !fresh && !update_moved && worked_out.elapsed() < TIME_CHECK {
            continue;
        }
        worked_out = std::time::Instant::now();
        let Some(state) = &state else { continue };
        let line = Line::of(
            state,
            here.as_deref(),
            user_home.as_deref(),
            update.as_ref(),
            now_secs(),
        );
        // Only what a reader shows decides whether to print: not the clock,
        // and not which event happened to bring it.
        if printed.as_ref() == Some(&line.content) {
            continue;
        }
        // Why this line is printed: the fleet changed (`None`, the watch says
        // how), a newer cohors was found, or only the clock moved.
        let why = if fresh {
            None
        } else if update_moved {
            Some("update")
        } else {
            Some("time")
        };
        let text = if human {
            // The notice goes under the first line, and again when it changes.
            let with_update = printed.is_none() || update_moved;
            line.human(state, why, with_update, color)
        } else {
            let mut json = serde_json::to_string(&line.json(state, why))
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
    /// A newer cohors (`{version, notice, detail}`), or `null`.
    update: Value,
    /// What decides whether it is printed: everything above, minus the clock.
    content: Value,
}

impl Line {
    fn of(
        state: &FleetState,
        here: Option<&Path>,
        user_home: Option<&Path>,
        update: Option<&UpdateState>,
        now: i64,
    ) -> Self {
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
        let update = update_json(update);
        let mut clockless = overview.clone();
        if let Value::Object(map) = &mut clockless {
            map.remove("now");
        }
        let content = json!({
            "overview": clockless,
            "launchpad": launchpad,
            "suggestions": suggestions,
            "update": update,
        });
        Self {
            overview,
            launchpad,
            suggestions,
            update,
            content,
        }
    }

    /// The NDJSON object. `why` is `None` when the fleet changed, else why
    /// the line was printed without a change (`update`, `time`).
    fn json(&self, state: &FleetState, why: Option<&str>) -> Value {
        json!({
            "type": "fleet",
            "seq": state.seq,
            "reason": why.map_or_else(|| json!(state.reason), |w| json!(w)),
            "changed": if why.is_none() { json!(state.changed) } else { json!([]) },
            "overview": self.overview,
            "launchpad": self.launchpad,
            "suggestions": self.suggestions,
            "update": self.update,
        })
    }

    /// One or two lines for a person at a terminal: the headline, and which
    /// repositories changed; with `with_update`, a newer cohors too.
    fn human(
        &self,
        state: &FleetState,
        why: Option<&str>,
        with_update: bool,
        color: bool,
    ) -> String {
        let fresh = why.is_none();
        let headline = self.overview["headline"].as_str().unwrap_or_default();
        let reason = match why {
            Some(why) => why.to_string(),
            None => serde_json::to_value(state.reason)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default(),
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
        if with_update && let Some(notice) = self.update["notice"].as_str() {
            text.push_str(&format!("  {:<9}  {}\n", "", bold(color, notice)));
        }
        text
    }
}

/// A newer cohors as the stream carries it: the same shape as the MCP
/// server's `meta.update`, so a reader of either learns one form.
fn update_json(update: Option<&UpdateState>) -> Value {
    match update {
        Some(update) => json!({
            "version": update.version(),
            "notice": update.notice(),
            "detail": update,
        }),
        None => Value::Null,
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
