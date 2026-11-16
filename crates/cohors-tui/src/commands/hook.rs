//! `cohors hook <EVENT>` — what Claude Code runs when a session starts
//! (ADR-073).
//!
//! Claude Code runs a hook as a program: it writes one JSON object about the
//! event to the program's standard input, and reads one JSON object from its
//! standard output. This is that program, for the one event cohors has
//! something to say at: `session-start`.
//!
//! When the repository the session opened in has work left behind (see
//! `cohors_core::hooks`), it prints one line of JSON: a note for the model
//! (`additionalContext`) and, when the person has not been told yet, a line
//! for the person (`systemMessage`). Otherwise it prints nothing at all.
//!
//! A hook runs while a session waits for it, and a failing hook shows up as an
//! error in somebody's conversation. So this command keeps to three rules:
//!
//! - **Quiet.** Nothing on standard error, ever, and an exit status of 0 for
//!   every input: an event it does not know, input it cannot read, a folder
//!   that is not a repository, even a bug (a panic is caught and silenced).
//! - **Quick.** It reads one repository and no more: no config, no discovery
//!   walk, no network (`cohors_fleet::checkout_at`).
//! - **Once.** The model is given the note every time, since that is cheap and
//!   nobody sees it. The person is shown it once, then again when what it says
//!   changes or a day has passed. That memory is one small file in cohors'
//!   cache folder; if it cannot be read or written the command behaves as if
//!   nothing had been shown yet and still prints the note.
//!
//! The pure parts (reading the input, deciding what to print, updating the
//! memory) take values and return values, so they are tested without a
//! filesystem. [`run`] is the thin layer that touches the world.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{IsTerminal, Read, Write};

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use cohors_core::hooks::{StartNote, session_start_note, should_show};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The only event there is something to say at.
const SESSION_START: &str = "session-start";

/// The file in cohors' cache folder that remembers what the person was told.
const MEMORY_FILE: &str = "hook-notes.json";

/// How many repositories the memory keeps. A person does not open sessions in
/// more than a few dozen, and a cap means the file can never grow without end.
const MAX_REMEMBERED: usize = 200;

/// The most of the hook's input that is read. Claude Code sends a few hundred
/// bytes; the limit only keeps an odd caller from making this read forever.
const INPUT_MAX: u64 = 64 * 1024;

/// The most of the memory file that is read (it is a few kilobytes).
const MEMORY_MAX: u64 = 1024 * 1024;

/// `cohors hook <EVENT>`. Always `Ok`: see the module notes.
pub fn run(event: &str) -> Result<()> {
    // An event a newer plugin knows and this cohors does not is not an error.
    if event == SESSION_START {
        session_start_quietly();
    }
    Ok(())
}

/// [`session_start`], but it cannot make a noise: a panic would print to
/// standard error and end the process with status 101, which Claude Code shows
/// as a failed hook. This command's job is to never be that, so a bug here
/// costs the person one missing note instead.
fn session_start_quietly() {
    std::panic::set_hook(Box::new(|_| {}));
    let _ = std::panic::catch_unwind(session_start);
}

/// Read the event, look at the repository, print the note if there is one.
fn session_start() {
    let input = parse_input(&read_input());
    let Some(folder) = input.cwd.or_else(current_folder) else {
        return;
    };
    let now = super::now_secs();

    let Some(checkout) = cohors_fleet::checkout_at(&folder, now) else {
        return;
    };
    let Some(note) = session_start_note(&checkout.repo, checkout.worktree.as_deref(), now) else {
        return;
    };

    // The repository is remembered by its path, the main checkout's, so every
    // session in any of its worktrees shares one memory of what was said.
    let repo = checkout.repo.id.0.as_str();
    let file = memory_file();
    let remembered = file.as_deref().map(read_remembered).unwrap_or_default();
    let previous = remembered
        .get(repo)
        .map(|shown| (shown.fingerprint.as_str(), shown.at));

    let answer = answer(input.source, &note, previous, now);
    // A closed pipe means nobody is listening; there is nothing to do about
    // it, and `println!` would panic. Standard output's own buffer flushes at
    // the end of the line.
    let _ = writeln!(std::io::stdout().lock(), "{}", answer.output);

    // Only what was actually shown is remembered, and only after it was
    // printed: a failure to write costs a repeat of the note, never its loss.
    if answer.told_the_person
        && let Some(file) = file
    {
        let remembered = remember(remembered, repo, &note.fingerprint, now);
        let _ = write_remembered(&file, &remembered);
    }
}

// ── Reading the event ────────────────────────────────────────────────────────

/// Why the session started, as Claude Code says in the event's `source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// A new conversation.
    Startup,
    /// An old conversation picked up again.
    Resume,
    /// Anything else: `clear`, `compact`, `fork`, or a reason added since this
    /// was written. These happen in the middle of a conversation the person
    /// has already been told things in.
    Other,
}

impl Source {
    fn from_name(name: &str) -> Self {
        match name {
            "startup" => Self::Startup,
            "resume" => Self::Resume,
            _ => Self::Other,
        }
    }

    /// Whether this is a person sitting down to work, which is the only time
    /// a line is worth showing them.
    fn person_is_arriving(self) -> bool {
        matches!(self, Self::Startup | Self::Resume)
    }
}

/// What this command uses of the event.
#[derive(Debug, PartialEq, Eq)]
struct Input {
    /// The folder the session opened in, when the event says.
    cwd: Option<Utf8PathBuf>,
    /// Why it started. A run with no event to read (by hand, or a caller that
    /// sent nothing) counts as a fresh start.
    source: Source,
}

/// The fields of the event this command uses. Anything unreadable (empty,
/// not JSON, not an object, a field of the wrong type) leaves that field at
/// its default rather than failing: a hook must not break a session over its
/// own input.
fn parse_input(text: &str) -> Input {
    let event: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let cwd = event
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|cwd| !cwd.is_empty())
        .map(Utf8PathBuf::from);
    let source = event
        .get("source")
        .and_then(Value::as_str)
        .map_or(Source::Startup, Source::from_name);
    Input { cwd, source }
}

/// All of standard input, up to [`INPUT_MAX`]; empty when there is none.
fn read_input() -> String {
    let stdin = std::io::stdin();
    // Run by hand at a terminal, waiting for input would just hang.
    if stdin.is_terminal() {
        return String::new();
    }
    let mut bytes = Vec::new();
    let _ = stdin.lock().take(INPUT_MAX).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The folder this process runs in, as UTF-8.
fn current_folder() -> Option<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(std::env::current_dir().ok()?).ok()
}

// ── What to print ────────────────────────────────────────────────────────────

/// What to print, and whether it tells the person (which is what gets
/// remembered).
#[derive(Debug, PartialEq)]
struct Answer {
    output: Value,
    told_the_person: bool,
}

/// The reply to a session start with `note` to give.
///
/// The model always gets the note. The person gets the summary line only when
/// they are sitting down to work (not after a `/clear` or a compaction in the
/// middle of the conversation) and `should_show` says it is new to them or a
/// day old. `previous` is what was last shown for this repository.
fn answer(source: Source, note: &StartNote, previous: Option<(&str, i64)>, now: i64) -> Answer {
    let told_the_person =
        source.person_is_arriving() && should_show(previous, &note.fingerprint, now);

    let mut output = Map::new();
    output.insert(
        "hookSpecificOutput".to_string(),
        json!({
            "hookEventName": "SessionStart",
            "additionalContext": note.context,
        }),
    );
    if told_the_person {
        output.insert("systemMessage".to_string(), json!(note.summary));
    }
    Answer {
        output: Value::Object(output),
        told_the_person,
    }
}

// ── What was shown ───────────────────────────────────────────────────────────

/// What the person was last shown for one repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Shown {
    /// `StartNote::fingerprint`: changes when what the note says changes.
    fingerprint: String,
    /// When, in Unix seconds.
    at: i64,
}

/// Repository path to what was last shown there. A `BTreeMap` keeps the file
/// in a stable order, so it only changes when something was shown.
type Remembered = BTreeMap<String, Shown>;

/// `remembered` with `fingerprint` recorded as shown in `repo` at `now`. When
/// that leaves more than [`MAX_REMEMBERED`] repositories, the ones shown
/// longest ago are dropped (never the one just recorded).
fn remember(mut remembered: Remembered, repo: &str, fingerprint: &str, now: i64) -> Remembered {
    remembered.insert(
        repo.to_string(),
        Shown {
            fingerprint: fingerprint.to_string(),
            at: now,
        },
    );
    let excess = remembered.len().saturating_sub(MAX_REMEMBERED);
    if excess > 0 {
        let mut by_age: Vec<(i64, String)> = remembered
            .iter()
            .filter(|(path, _)| path.as_str() != repo)
            .map(|(path, shown)| (shown.at, path.clone()))
            .collect();
        by_age.sort();
        for (_, path) in by_age.into_iter().take(excess) {
            remembered.remove(&path);
        }
    }
    remembered
}

/// Where the memory lives, when cohors has a cache folder.
fn memory_file() -> Option<Utf8PathBuf> {
    Some(cohors_config::paths::cache_dir().ok()?.join(MEMORY_FILE))
}

/// The memory, or an empty one when the file is missing or cannot be read
/// (the harmless way to be wrong: the person is told once more).
fn read_remembered(file: &Utf8Path) -> Remembered {
    let Ok(handle) = File::open(file) else {
        return Remembered::new();
    };
    let mut text = String::new();
    if handle.take(MEMORY_MAX).read_to_string(&mut text).is_err() {
        return Remembered::new();
    }
    serde_json::from_str(&text).unwrap_or_default()
}

/// Write the memory whole: to a temporary file beside it, then renamed into
/// place, so two sessions starting at once never leave half a file.
fn write_remembered(file: &Utf8Path, remembered: &Remembered) -> std::io::Result<()> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let body = serde_json::to_vec_pretty(remembered).map_err(std::io::Error::other)?;
    let temp = file.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&temp, body)?;
    std::fs::rename(&temp, file).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cohors_core::hooks::REMIND_AFTER_SECS;

    const NOW: i64 = 1_700_000_000;

    fn note() -> StartNote {
        StartNote {
            summary: "cohors: api has work that is not pushed.".to_string(),
            context: "cohors, about the repository this session is in (api).\n- worktree a-1"
                .to_string(),
            fingerprint: "a-1:2:0".to_string(),
        }
    }

    /// The reply for `source` with nothing remembered yet.
    fn first_time(source: Source) -> Answer {
        answer(source, &note(), None, NOW)
    }

    fn context_of(answer: &Answer) -> &str {
        answer.output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additionalContext is text")
    }

    #[test]
    fn a_person_sitting_down_is_told_and_the_model_always_is() {
        for source in [Source::Startup, Source::Resume] {
            let reply = first_time(source);
            assert!(reply.told_the_person, "{source:?}");
            assert_eq!(reply.output["systemMessage"], note().summary);
            assert_eq!(
                reply.output["hookSpecificOutput"]["hookEventName"],
                "SessionStart"
            );
            assert_eq!(context_of(&reply), note().context);
        }
    }

    #[test]
    fn the_middle_of_a_conversation_gets_the_model_only() {
        for name in ["clear", "compact", "fork", "something-new"] {
            let reply = first_time(Source::from_name(name));
            assert!(!reply.told_the_person, "{name}");
            assert!(reply.output.get("systemMessage").is_none(), "{name}");
            assert_eq!(context_of(&reply), note().context, "{name}");
        }
    }

    #[test]
    fn a_note_the_person_already_has_goes_to_the_model_alone() {
        let fingerprint = note().fingerprint;
        let previous = Some((fingerprint.as_str(), NOW - 60));
        let reply = answer(Source::Startup, &note(), previous, NOW);
        assert!(!reply.told_the_person);
        assert!(reply.output.get("systemMessage").is_none());
        assert_eq!(context_of(&reply), note().context);

        // Told again when it changed, or when a day has passed.
        let changed = answer(Source::Startup, &note(), Some(("other", NOW - 60)), NOW);
        assert!(changed.told_the_person);
        let a_day_on = Some((fingerprint.as_str(), NOW - REMIND_AFTER_SECS));
        assert!(answer(Source::Startup, &note(), a_day_on, NOW).told_the_person);
    }

    #[test]
    fn the_reply_is_one_line_of_json_with_only_the_expected_keys() {
        let reply = first_time(Source::Startup);
        let line = reply.output.to_string();
        assert!(!line.contains('\n'), "the note has a newline: {line}");
        let back: Value = serde_json::from_str(&line).unwrap();
        let mut keys: Vec<&str> = back
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["hookSpecificOutput", "systemMessage"]);
    }

    #[test]
    fn the_event_is_read_and_anything_unreadable_is_a_fresh_start_here() {
        let event = r#"{"session_id":"s","cwd":"/code/api","source":"resume","model":"m"}"#;
        assert_eq!(
            parse_input(event),
            Input {
                cwd: Some(Utf8PathBuf::from("/code/api")),
                source: Source::Resume,
            }
        );
        assert_eq!(
            parse_input(r#"{"cwd":"/x","source":"clear"}"#).source,
            Source::Other
        );
        assert_eq!(parse_input(r#"{"cwd":"/x"}"#).source, Source::Startup);

        let nothing = Input {
            cwd: None,
            source: Source::Startup,
        };
        for text in [
            "",
            "   \n",
            "not json at all",
            "{",
            "null",
            "[1, 2, 3]",
            "\"cwd\"",
            "{}",
            r#"{"cwd":""}"#,
            r#"{"cwd":42,"source":7}"#,
        ] {
            assert_eq!(parse_input(text), nothing, "{text:?}");
        }
    }

    #[test]
    fn remembering_inserts_then_overwrites() {
        let remembered = remember(Remembered::new(), "/code/api", "a:1:0", 100);
        assert_eq!(
            remembered["/code/api"],
            Shown {
                fingerprint: "a:1:0".to_string(),
                at: 100
            }
        );
        let remembered = remember(remembered, "/code/web", "w:0:2", 150);
        let remembered = remember(remembered, "/code/api", "a:2:0", 200);
        assert_eq!(remembered.len(), 2);
        assert_eq!(remembered["/code/api"].fingerprint, "a:2:0");
        assert_eq!(remembered["/code/api"].at, 200);
        assert_eq!(remembered["/code/web"].at, 150);
    }

    #[test]
    fn the_memory_is_capped_and_forgets_the_oldest_first() {
        let mut remembered = Remembered::new();
        for n in 0..MAX_REMEMBERED {
            remembered = remember(remembered, &format!("/repo/{n}"), "f", 1_000 + n as i64);
        }
        assert_eq!(remembered.len(), MAX_REMEMBERED);

        // One more: the one shown longest ago (`/repo/0`) goes.
        let remembered = remember(remembered, "/repo/new", "f", 5_000);
        assert_eq!(remembered.len(), MAX_REMEMBERED);
        assert!(!remembered.contains_key("/repo/0"));
        assert!(remembered.contains_key("/repo/1"));
        assert!(remembered.contains_key("/repo/new"));

        // Showing an old one again does not grow it, and refreshes its age.
        let remembered = remember(remembered, "/repo/1", "g", 6_000);
        assert_eq!(remembered.len(), MAX_REMEMBERED);
        let remembered = remember(remembered, "/repo/newer", "f", 7_000);
        assert!(remembered.contains_key("/repo/1"));
        assert!(!remembered.contains_key("/repo/2"));
    }

    #[test]
    fn a_memory_longer_than_the_cap_is_cut_down_and_keeps_the_newest() {
        // A file edited by hand, or written by a longer-memoried version.
        let mut remembered = Remembered::new();
        for n in 0..(MAX_REMEMBERED + 30) {
            remembered.insert(
                format!("/repo/{n}"),
                Shown {
                    fingerprint: "f".to_string(),
                    at: n as i64,
                },
            );
        }
        // Even with a clock that ran backwards, the entry just written stays.
        let remembered = remember(remembered, "/repo/now", "f", -5);
        assert_eq!(remembered.len(), MAX_REMEMBERED);
        assert!(remembered.contains_key("/repo/now"));
        assert!(!remembered.contains_key("/repo/0"));
        assert!(remembered.contains_key(&format!("/repo/{}", MAX_REMEMBERED + 29)));
    }

    #[test]
    fn the_memory_file_round_trips_and_a_bad_one_is_empty() {
        let folder = tempfile::tempdir().unwrap();
        let file = Utf8PathBuf::from_path_buf(folder.path().join("deep/hook-notes.json")).unwrap();

        // Missing: empty, and writing makes the folders it needs.
        assert!(read_remembered(&file).is_empty());
        let remembered = remember(Remembered::new(), "/code/api", "a:1:0", 100);
        write_remembered(&file, &remembered).unwrap();
        assert_eq!(read_remembered(&file), remembered);

        // No temporary file is left beside it.
        let left: Vec<_> = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(left.len(), 1, "{left:?}");

        // Garbage, or the wrong shape: empty, never an error.
        for text in ["", "nope", "[]", r#"{"/code/api":"x"}"#] {
            std::fs::write(&file, text).unwrap();
            assert!(read_remembered(&file).is_empty(), "{text:?}");
        }
    }
}
