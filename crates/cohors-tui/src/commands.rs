//! Implementations of the CLI subcommands.

pub mod agent;
pub mod launchpad;
pub mod look;
pub mod reads;

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use cohors_core::{Selector, SortMode};

use crate::cli::Cli;

/// A command that already said what went wrong and only needs the process to
/// end with this status: `1` when some repos failed, `2` for a usage problem.
/// `main` exits with it quietly, without anyhow's `Error:` line.
#[derive(Debug)]
pub struct ExitCode(pub i32);

impl std::fmt::Display for ExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exit status {}", self.0)
    }
}

impl std::error::Error for ExitCode {}

/// `cohors init` — write a starter config and print its path. Auto-detects where
/// the user keeps repos and seeds `roots` with them, so the first `cohors` run
/// shows a populated fleet instead of an empty one.
pub fn init(cli: &Cli, force: bool) -> Result<()> {
    let path: Utf8PathBuf = match &cli.config {
        Some(p) => Utf8PathBuf::from(p),
        None => cohors_config::paths::config_file().context("resolving the config path")?,
    };
    let home = cohors_config::paths::home_dir().ok();
    let roots = cohors_fleet::detect_roots(home.as_deref());
    cohors_config::write_starter(&path, force, &roots)
        .with_context(|| format!("writing config to {path}"))?;
    println!("Wrote starter config to {path}");
    if roots.is_empty() {
        println!("No repos auto-detected — edit `roots` to point at your code, then run `cohors`.");
    } else {
        println!("Detected roots: {}", roots.join(", "));
        println!("Run `cohors` to see your fleet (edit `roots` to adjust).");
    }
    Ok(())
}

/// `cohors scan` — discover + snapshot all repos and report them.
///
/// A human at a terminal gets the styled report ([`scan_report`]); anything else
/// (a pipe, a redirect, or an explicit `--json`) gets the raw JSON, so scripts
/// keep the contract they always had.
///
/// With `--select`, only the matching repos are printed, in dirty-first order,
/// using the same [`cohors_core::resolve`] the dashboard and (later) the MCP
/// server use — so `scan --select behind` and `list_repos({behind:true})` agree.
pub fn scan(cli: &Cli, select: Option<&str>, json: bool, deep: bool) -> Result<()> {
    // Decide the surface up front: only a human at a terminal gets the progress
    // note and the styled report.
    let human = !json && std::io::stdout().is_terminal();
    let color = wants_color(human);
    let scanner = crate::scan::from_cli(cli)?;
    let roots = scanner.roots();

    // Discovery walks the disk, which on a large fleet is a beat or two of
    // silence. Say what is happening on stderr (so a pipe stays clean), then wipe
    // the line once the report is ready to take its place.
    let announce = human && std::io::stderr().is_terminal();
    if announce {
        let where_at = roots
            .iter()
            .map(|r| crate::ui::abbrev_home(r))
            .collect::<Vec<_>>()
            .join(", ");
        let doing = if deep {
            "searching deep in"
        } else {
            "scanning"
        };
        eprint!("  {} {where_at}…", dim(color, doing));
        let _ = std::io::stderr().flush();
    }
    let started = std::time::Instant::now();
    let (mut snapshots, found) = scanner.scan_discovered(deep);
    let took = started.elapsed();
    if announce {
        eprint!("\r\x1b[2K"); // carriage return, then erase the line
        let _ = std::io::stderr().flush();
    }

    // An empty fleet is cryptic on the scriptable surface. Keep stdout a clean
    // `[]` (the JSON contract), but nudge a human via stderr so they aren't left
    // guessing why nothing came back. Scripts piping `scan` ignore stderr.
    if snapshots.is_empty() {
        eprintln!(
            "cohors: no git repositories found under {}.\n  Run `cohors init` to detect your repos, or pass --root <dir>.",
            roots.join(", ")
        );
    }

    let scanned = snapshots.len();
    if let Some(query) = select {
        let selector = parse_selector(query)?;
        let order = cohors_core::resolve(&snapshots, &selector, SortMode::DirtyFirst, now_secs());
        // Reorder/filter the snapshots to the resolved set, preserving order.
        let by_id: HashMap<&str, usize> = snapshots
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.0.as_str(), i))
            .collect();
        let picked: Vec<usize> = order
            .iter()
            .filter_map(|id| by_id.get(id.0.as_str()).copied())
            .collect();
        snapshots = picked.into_iter().map(|i| snapshots[i].clone()).collect();
    }

    // JSON stays the contract for scripts: forced by `--json`, and the default
    // whenever stdout is not a terminal (piped into jq and friends).
    if !human {
        let out = serde_json::to_string_pretty(&snapshots).context("serializing snapshots")?;
        println!("{out}");
        return Ok(());
    }
    let matched = select.map(|q| (q, scanned));
    print!(
        "{}",
        scan_report(
            &snapshots,
            &roots,
            took,
            now_secs(),
            term_width(),
            color,
            matched
        )
    );
    print!("{}", deep_note(deep, found.deep_only.len(), "scan", color));
    Ok(())
}

/// The line under a `scan`/`discover` report about the search's reach: what a
/// deep search found and kept, what an earlier one left remembered, or how to
/// search deeper. `cmd` is the command to suggest `--deep` on.
fn deep_note(deep: bool, deep_only: usize, cmd: &str, color: bool) -> String {
    let text = match (deep, deep_only) {
        (true, 0) => "The deep search found nothing a normal search misses.".to_string(),
        (true, 1) => "Remembered 1 repository only a deep search reaches: the dashboard shows it from now on.".to_string(),
        (true, n) => format!(
            "Remembered {n} repositories only a deep search reaches: the dashboard shows them from now on."
        ),
        (false, 0) => {
            return format!(
                "  {}  {}\n\n",
                bold(color, &format!("cohors {cmd} --deep")),
                dim(color, "also looks inside every repo, at any depth")
            );
        }
        (false, n) => format!(
            "Includes {} remembered from an earlier --deep search.",
            plural(n, "repository", "repositories")
        ),
    };
    format!("  {}\n\n", dim(color, &text))
}

/// `cohors discover` — list the repos a search finds, without reading them.
///
/// At a terminal: the repos under each root, nested ones marked. Piped: one
/// absolute path per line. `--json`: `[{path, name, nested_in, deep_only}]`.
pub fn discover(cli: &Cli, deep: bool, json: bool) -> Result<()> {
    let human = !json && std::io::stdout().is_terminal();
    let color = wants_color(human);
    let scanner = crate::scan::from_cli(cli)?;
    let roots = scanner.root_dirs();

    let announce = human && std::io::stderr().is_terminal();
    if announce {
        let where_at = roots
            .iter()
            .map(|r| crate::ui::abbrev_home(r.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        let doing = if deep {
            "searching deep in"
        } else {
            "looking in"
        };
        eprint!("  {} {where_at}…", dim(color, doing));
        let _ = std::io::stderr().flush();
    }
    let started = std::time::Instant::now();
    let found = scanner.discover(deep);
    let took = started.elapsed();
    if announce {
        eprint!("\r\x1b[2K");
        let _ = std::io::stderr().flush();
    }

    if found.repos.is_empty() {
        let shown: Vec<String> = if roots.is_empty() {
            scanner.roots()
        } else {
            roots.iter().map(|r| r.to_string()).collect()
        };
        eprintln!(
            "cohors: no git repositories found under {}.\n  Run `cohors init` to detect your repos, or pass --root <dir>.",
            shown.join(", ")
        );
    }

    if json {
        let rows: Vec<serde_json::Value> = found
            .repos
            .iter()
            .map(|path| {
                serde_json::json!({
                    "path": path,
                    "name": path.file_name().unwrap_or(path.as_str()),
                    "nested_in": nested_in(path, &found.repos),
                    "deep_only": found.deep_only.contains(path),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if !human {
        // One path per line is only safe when no path can break the line: a
        // folder name with a newline or an escape sequence would forge entries
        // or reach the terminal. Those are left out here (JSON carries them).
        let mut held_back = 0;
        for path in &found.repos {
            if path.as_str().chars().any(char::is_control) {
                held_back += 1;
            } else {
                println!("{path}");
            }
        }
        if held_back > 0 {
            eprintln!(
                "cohors: left out {} whose path has control characters; `cohors discover --json` lists every repo.",
                plural(held_back, "repository", "repositories")
            );
        }
        return Ok(());
    }
    print!("{}", discover_report(&found, &roots, took, deep, color));
    Ok(())
}

/// The repo that encloses `path` most closely among `repos`, if any.
fn nested_in<'a>(path: &Utf8Path, repos: &'a [Utf8PathBuf]) -> Option<&'a Utf8PathBuf> {
    repos
        .iter()
        .filter(|other| other.as_path() != path && path.starts_with(other))
        .max_by_key(|other| other.as_str().len())
}

/// The human `cohors discover` list: each root as a section, its repos by
/// path relative to it, nested and deep-only ones marked. Pure (the paths come
/// in) so it's unit-testable; `color` gates the ANSI.
fn discover_report(
    found: &cohors_fleet::Discovery,
    roots: &[Utf8PathBuf],
    took: std::time::Duration,
    deep: bool,
    color: bool,
) -> String {
    let where_at = roots
        .iter()
        .map(|r| crate::ui::abbrev_home(r.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = masthead(
        &format!(
            "found {} in {}{}",
            plural(found.repos.len(), "repository", "repositories"),
            took_label(took),
            if deep { ", searching deep" } else { "" }
        ),
        &where_at,
        color,
    );
    if found.repos.is_empty() {
        out.push_str(&format!("  {}\n\n", dim(color, "nothing found")));
        return out;
    }

    // Each repo belongs to the closest root that holds it (roots may overlap).
    let home_of = |path: &Utf8Path| -> Option<usize> {
        roots
            .iter()
            .enumerate()
            .filter(|(_, root)| path.starts_with(root))
            .max_by_key(|(_, root)| root.as_str().len())
            .map(|(i, _)| i)
    };
    let width = found
        .repos
        .iter()
        .map(|p| match home_of(p).map(|i| &roots[i]) {
            Some(root) => relative_label(p, root).chars().count(),
            None => crate::ui::abbrev_home(p.as_str()).chars().count(),
        })
        .max()
        .unwrap_or(0)
        .min(term_width().saturating_sub(30).max(20));

    let mut groups: Vec<(String, Vec<&Utf8PathBuf>)> = roots
        .iter()
        .map(|r| {
            (
                crate::ui::text::safe(&crate::ui::abbrev_home(r.as_str())),
                Vec::new(),
            )
        })
        .collect();
    let mut elsewhere: Vec<&Utf8PathBuf> = Vec::new();
    for path in &found.repos {
        match home_of(path) {
            Some(i) => groups[i].1.push(path),
            None => elsewhere.push(path),
        }
    }
    for (i, (label, paths)) in groups.iter().enumerate() {
        if paths.is_empty() {
            continue;
        }
        out.push_str(&section(
            label,
            &plural(paths.len(), "repository", "repositories"),
            color,
        ));
        for path in paths {
            let shown = relative_label(path, &roots[i]);
            out.push_str(&discover_row(&shown, path, found, width, color));
        }
        out.push('\n');
    }
    if !elsewhere.is_empty() {
        out.push_str(&section("elsewhere", "outside every root", color));
        for path in elsewhere {
            let shown = crate::ui::abbrev_home(path.as_str());
            out.push_str(&discover_row(&shown, path, found, width, color));
        }
        out.push('\n');
    }
    out.push_str(&deep_note(deep, found.deep_only.len(), "discover", color));
    out
}

/// `path` relative to `root`; the root itself reads `.` (the folder is a repo).
fn relative_label(path: &Utf8Path, root: &Utf8Path) -> String {
    match path.strip_prefix(root) {
        Ok(rel) if rel.as_str().is_empty() => ".".to_string(),
        Ok(rel) => rel.to_string(),
        Err(_) => crate::ui::abbrev_home(path.as_str()),
    }
}

/// One `discover` row: the path, then what is notable about it.
fn discover_row(
    shown: &str,
    path: &Utf8Path,
    found: &cohors_fleet::Discovery,
    width: usize,
    color: bool,
) -> String {
    let mut notes: Vec<String> = Vec::new();
    if shown == "." {
        notes.push("this folder is a repo".to_string());
    }
    if let Some(outer) = nested_in(path, &found.repos) {
        let name = outer.file_name().unwrap_or(outer.as_str());
        notes.push(format!("inside {}", crate::ui::text::safe(name)));
    }
    let mut tail: Vec<String> = Vec::new();
    if found.deep_only.iter().any(|p| p == path) {
        tail.push(brand(color, "deep"));
    }
    if !notes.is_empty() {
        tail.push(dim(color, &notes.join(" · ")));
    }
    // A folder name is repository-controlled text: no escape sequences.
    let name = clip(&crate::ui::text::safe(shown), width);
    if tail.is_empty() {
        return format!("    {}\n", bold(color, &name));
    }
    format!(
        "    {}  {}\n",
        bold(color, &format!("{name:<width$}")),
        tail.join("  ")
    )
}

/// How long the scan took, in the coarsest unit that still reads honestly.
fn took_label(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", d.as_secs_f32())
    }
}

/// Clip `s` to `max` columns, marking the cut with an ellipsis.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Break `text` into lines of at most `width` columns at spaces. A single
/// word longer than the width is clipped rather than left to overflow.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let word = clip(word, width);
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Where a repo stands against its upstream, in words: `level`, `2 ahead`,
/// `3 behind`, `2 ahead, 3 behind`, `no upstream` (a remote exists but the
/// branch was never pushed) or `no remote`. Blank for a repo that could not be
/// read, since nothing is known about it.
fn sync_words(line: &cohors_core::RepoLine) -> String {
    if line.error.is_some() {
        return String::new();
    }
    match (&line.upstream, line.ahead, line.behind) {
        (None, _, _) if !line.has_remote => "no remote".to_string(),
        (None, _, _) => "no upstream".to_string(),
        (Some(_), 0, 0) => "level".to_string(),
        (Some(_), ahead, 0) => format!("{ahead} ahead"),
        (Some(_), 0, behind) => format!("{behind} behind"),
        (Some(_), ahead, behind) => format!("{ahead} ahead, {behind} behind"),
    }
}

/// `7 repositories` / `1 repository`.
fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The shortest command that starts a "Do next" step: the verb that carries it
/// out when there is one, otherwise the listing to look at first.
fn action_command(action: &cohors_core::NextAction) -> &'static str {
    use cohors_core::ActionKind as K;
    match action.kind {
        K::Unreadable => "cohors scan --select error",
        K::Resolve => "cohors scan --select conflicted",
        // Diverged branches are left out: a plain push would be rejected.
        K::Push => "cohors push --select 'ahead,!diverged'",
        K::Reconcile => "cohors scan --select diverged",
        K::Publish => "cohors push --select unpublished",
        K::ReviewStash => "cohors scan --select stash",
        // Diverged branches are left out: a fast-forward pull cannot apply.
        K::Pull => "cohors pull --select 'behind,!diverged'",
        K::Commit => "cohors scan --select dirty",
        K::Fetch => "cohors fetch --select stale-fetch",
        K::StartTask => "cohors launchpad",
    }
}

/// A key bar in the dashboard's grammar: `enter  open the dashboard    q  quit`.
fn key_bar(keys: &[(&str, &str)], color: bool) -> String {
    look::key_bar(keys, color)
}

/// The report's masthead: the mark beside the wordmark, mirroring the launch
/// screen so both surfaces open the same way.
fn masthead(scanned: &str, where_at: &str, color: bool) -> String {
    look::masthead(scanned, where_at, color)
}

/// One report section, in the launch screen's grammar: a brand-purple heading
/// over a dim one-line descriptor.
fn section(heading: &str, descriptor: &str, color: bool) -> String {
    look::section(heading, descriptor, color)
}

/// How wide the terminal is, for reports that lay out columns. A report that
/// is wider than the terminal wraps, and a wrapped table cannot be read, so
/// every column report sizes itself to this. Falls back to 100 when the width
/// cannot be read (output piped into a pager, say).
fn term_width() -> usize {
    ratatui::crossterm::terminal::size()
        .map(|(cols, _)| cols as usize)
        .unwrap_or(100)
        .clamp(60, 200)
}

/// The column widths of a repo listing: name, branch (0 = column dropped),
/// sync, and what is left for the reasons.
#[derive(Clone, Copy)]
struct RowWidths {
    name: usize,
    branch: usize,
    sync: usize,
    why: usize,
}

/// The age column is always this wide (`12mo`, `3w`), right-aligned.
const AGE_W: usize = 4;

impl RowWidths {
    /// Fit the columns of `lines` into `width` terminal columns. The reasons
    /// are the point of the report, so they get the room: the branch column is
    /// dropped first when the terminal is too narrow to show both properly.
    fn fit<'a>(
        lines: impl Iterator<Item = &'a cohors_core::RepoLine> + Clone,
        width: usize,
    ) -> Self {
        let widest = |of: &dyn Fn(&cohors_core::RepoLine) -> usize, min: usize, max: usize| {
            lines.clone().map(of).max().unwrap_or(min).clamp(min, max)
        };
        // A narrow terminal cannot afford a 28-column name.
        let name_cap = if width < 100 { 22 } else { 28 };
        let mut name = widest(&|r| r.name.chars().count(), 4, name_cap);
        let branch = widest(&|r| r.branch.chars().count(), 6, 20);
        let sync = widest(&|r| sync_words(r).chars().count(), 5, 18);
        // indent + mark + name + sync + age, each followed by a two-space gap.
        // indent, the severity block and a space, then each column and its gap.
        let fixed = |name: usize| 4 + BLOCK_W + 1 + name + 2 + sync + 2 + AGE_W + 2;
        // Reasons want at least this much to be worth reading…
        const WHY_WANTED: usize = 48;
        // …and below this they say nothing, so the name gives up room instead.
        const WHY_MIN: usize = 16;
        let branch = if fixed(name) + branch + 2 + WHY_WANTED <= width {
            branch
        } else {
            0
        };
        let branch_cols = if branch > 0 { branch + 2 } else { 0 };
        // One spare column, so the last character never touches the edge.
        let room = |name: usize| width.saturating_sub(fixed(name) + branch_cols + 1);
        if room(name) < WHY_MIN {
            name = name.saturating_sub(WHY_MIN - room(name)).max(8);
        }
        RowWidths {
            name,
            branch,
            sync,
            why: room(name).max(1),
        }
    }
}

/// One repo line: `mark name  [branch]  sync  age  why`. The mark is a shape
/// (triangle, diamond, ring), so urgency reads with colour off; colour is spent
/// only on what is genuinely urgent, so a long list of ordinary dirty repos
/// reads calm instead of turning the screen into a wall of red.
fn scan_row(line: &cohors_core::RepoLine, w: RowWidths, now: i64, color: bool) -> String {
    let age = line
        .commit
        .as_ref()
        .map(|c| cohors_core::time::relative(c.timestamp, now))
        .unwrap_or_default();
    // Every reason, not only the first: a repo with a never-pushed branch AND
    // an old stash should say both.
    let why = clip(&line.reasons.join("; "), w.why);
    // Pad before painting: ANSI escapes have no width, so colouring first would
    // throw the column alignment off.
    let branch = if w.branch > 0 {
        format!(
            "{}  ",
            dim(
                color,
                &format!("{:<width$}", clip(&line.branch, w.branch), width = w.branch)
            )
        )
    } else {
        String::new()
    };
    format!(
        "    {} {}  {branch}{}  {}  {}\n",
        severity_mark(line.severity, color),
        bold(
            color,
            &format!("{:<width$}", clip(&line.name, w.name), width = w.name)
        ),
        format_args!("{:<width$}", sync_words(line), width = w.sync),
        dim(color, &format!("{age:>AGE_W$}")),
        why,
    )
}

/// The human-facing `cohors scan` report: every repo that has something to
/// say, most urgent first, then the commands that start each next step. It is
/// drawn from the same [`cohors_core::Overview`] the dashboard uses, so a repo
/// is "at risk" here exactly when it is there. `color` gates the ANSI;
/// otherwise pure so it's unit-testable.
///
/// `selected` is `Some((query, scanned))` under `--select`: the masthead then
/// counts what was scanned and what matched, and clean matches are listed by
/// name (they are what was asked for) instead of collapsing into a count.
fn scan_report(
    snaps: &[cohors_core::RepoSnapshot],
    roots: &[String],
    took: std::time::Duration,
    now: i64,
    width: usize,
    color: bool,
    selected: Option<(&str, usize)>,
) -> String {
    let where_at = roots
        .iter()
        .map(|r| crate::ui::abbrev_home(r))
        .collect::<Vec<_>>()
        .join(", ");
    let headline = match selected {
        Some((query, scanned)) => format!(
            "scanned {} in {}, {} --select {query}",
            plural(scanned, "repository", "repositories"),
            took_label(took),
            if snaps.len() == 1 {
                "1 matches".to_string()
            } else {
                format!("{} match", snaps.len())
            }
        ),
        None => format!(
            "scanned {} in {}",
            plural(snaps.len(), "repository", "repositories"),
            took_label(took)
        ),
    };
    let mut out = masthead(&headline, &where_at, color);
    if snaps.is_empty() {
        let empty = if selected.is_some() {
            "nothing matched"
        } else {
            "nothing to report"
        };
        out.push_str(&format!("  {}\n\n", dim(color, empty)));
        return out;
    }

    // The scan needs no launchpad: tasks have their own report.
    let overview = cohors_core::overview(snaps, &[], &Default::default(), now);
    let risky: Vec<&cohors_core::RepoLine> = overview.at_risk().collect();
    let look: Vec<&cohors_core::RepoLine> = overview
        .repos
        .iter()
        .filter(|r| !r.at_risk && !r.reasons.is_empty())
        .collect();
    // Under --select the clean matches are part of the answer: list them.
    let calm: Vec<&cohors_core::RepoLine> = if selected.is_some() {
        overview
            .repos
            .iter()
            .filter(|r| !r.at_risk && r.reasons.is_empty())
            .collect()
    } else {
        Vec::new()
    };

    // Size the columns to the rows actually printed and to the terminal, so a
    // short fleet stays tight and no row is ever wider than the screen.
    let widths = RowWidths::fit(
        risky.iter().chain(look.iter()).chain(calm.iter()).copied(),
        width,
    );

    if !risky.is_empty() {
        out.push_str(&section(
            "work at risk",
            &format!("{} where local work could be lost", risky.len()),
            color,
        ));
        for line in &risky {
            out.push_str(&scan_row(line, widths, now, color));
        }
        out.push('\n');
    }
    if !look.is_empty() {
        out.push_str(&section(
            "needs a look",
            &format!("{} with uncommitted work or small drift", look.len()),
            color,
        ));
        for line in &look {
            out.push_str(&scan_row(line, widths, now, color));
        }
        out.push('\n');
    }
    // The clean ones collapse to a count: listing a dozen "up to date" rows is the
    // noise this report exists to remove.
    if !calm.is_empty() {
        out.push_str(&section(
            "clean",
            &format!("{} up to date, nothing to do", calm.len()),
            color,
        ));
        for line in &calm {
            out.push_str(&scan_row(line, widths, now, color));
        }
        out.push('\n');
    } else if overview.figures.clean > 0 {
        out.push_str(&section(
            "clean",
            &format!("{} up to date, nothing to do", overview.figures.clean),
            color,
        ));
        out.push('\n');
    }

    // What to do about it, as the command that starts each step.
    if !overview.actions.is_empty() {
        out.push_str(&section(
            "do next",
            "most urgent first; each command starts that step",
            color,
        ));
        let cmd_w = overview
            .actions
            .iter()
            .map(|a| action_command(a).chars().count())
            .max()
            .unwrap_or(0);
        // indent + mark + command + gap, then the title in what is left.
        let title_w = width.saturating_sub(4 + BLOCK_W + 1 + cmd_w + 2 + 1).max(8);
        for action in &overview.actions {
            out.push_str(&format!(
                "    {} {}  {}\n",
                severity_mark(action.severity, color),
                bold(color, &format!("{:<cmd_w$}", action_command(action))),
                dim(color, &clip(&action.title, title_w)),
            ));
        }
        out.push('\n');
    }
    out
}

/// `cohors overview` — the briefing, without the dashboard.
///
/// Renders the shared [`cohors_core::Overview`] (built once by the fleet
/// facade): what to do next, the key figures, and the work at risk. A pipe, a
/// redirect or `--json` gets the whole model as JSON, which is how the editor
/// extension and scripts read it.
pub fn overview(cli: &Cli, json: bool) -> Result<()> {
    let human = !json && std::io::stdout().is_terminal();
    let color = wants_color(human);
    let scanner = crate::scan::from_cli(cli)?;
    let snapshots = scanner.scan();
    let home = cohors_fleet::launchpad::launchpad_home();
    let overview = cohors_fleet::overview(&snapshots, home.as_deref(), now_secs());

    if !human {
        // The roots travel with the model so a consumer can show where it
        // looked (and explain an empty fleet) without a second call.
        let mut value = serde_json::to_value(&overview).context("serializing the overview")?;
        if let serde_json::Value::Object(map) = &mut value {
            map.insert("roots".to_string(), serde_json::json!(scanner.roots()));
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    print!(
        "{}",
        overview_report(&overview, &scanner.roots(), now_secs(), term_width(), color)
    );
    Ok(())
}

/// `cohors standup` — your commits across every repo, as a digest.
///
/// At a terminal it prints the markdown digest (the one the dashboard copies);
/// with `--json` it prints the commits grouped by repo. `today` narrows the
/// window from the last seven days to today.
pub fn standup(cli: &Cli, today: bool, json: bool) -> Result<()> {
    use cohors_core::StandupWindow;
    let scanner = crate::scan::from_cli(cli)?;
    let Some(author) = scanner.author_email() else {
        bail!(
            "no git identity: set `git config --global user.email` so cohors knows which commits are yours"
        );
    };
    let window = if today {
        StandupWindow::Today
    } else {
        StandupWindow::Week
    };
    let (since, until) = window.range(now_secs());
    let commits = cohors_fleet::fleet_commits(&scanner.scan(), &author, since, until);
    if json {
        let groups: Vec<serde_json::Value> = cohors_core::group_commits(&commits)
            .into_iter()
            .map(|(repo, commits)| serde_json::json!({ "repo": repo, "commits": commits }))
            .collect();
        let out = serde_json::json!({
            "window": window.label(),
            "author": author,
            "total": commits.len(),
            "groups": groups,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        print!("{}", cohors_core::to_markdown(&commits, window));
    }
    Ok(())
}

/// How wide a severity block is: ` RISK `, ` WARN `, ` NOTE `.
const BLOCK_W: usize = 6;

/// A severity as a block, the same one the dashboard leads its rows with: a
/// filled ` RISK ` (brick red) or ` WARN ` (ochre), a quiet unfilled ` NOTE `
/// or ` INFO `, and blank for a clean repo. Always [`BLOCK_W`] columns. The
/// word carries the meaning, so it reads with colour off and when piped.
///
/// The fills are the muted tones of the neutral theme. A report cannot ask
/// whether the terminal is light or dark, so it uses the deeper pair with
/// white text, which reads on both.
fn severity_mark(severity: cohors_core::Severity, color: bool) -> String {
    look::severity_mark(severity, color)
}

/// The human-facing `cohors overview` report. Pure (given the model, the
/// roots and a clock) so it is unit-testable; `color` gates the ANSI.
fn overview_report(
    o: &cohors_core::Overview,
    roots: &[String],
    now: i64,
    width: usize,
    color: bool,
) -> String {
    let where_at = roots
        .iter()
        .map(|r| crate::ui::abbrev_home(r))
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = masthead(&o.headline, &where_at, color);
    if o.repos.is_empty() {
        out.push_str(&format!(
            "  {}\n\n",
            dim(
                color,
                "run `cohors init` to detect your repos, or pass --root <dir>"
            )
        ));
        return out;
    }

    // ── Do next.
    if o.actions.is_empty() {
        out.push_str(&section("do next", "nothing needs you right now", color));
    } else {
        out.push_str(&section(
            "do next",
            "most urgent first; the command under each step starts it",
            color,
        ));
        // The reason and the command sit under the title, past the block.
        let lead = 4 + BLOCK_W + 1;
        let pad = " ".repeat(lead);
        for action in &o.actions {
            // The command that starts this step.
            let how = action_command(action);
            out.push_str(&format!(
                "    {} {}\n",
                severity_mark(action.severity, color),
                bold(color, &clip(&action.title, width.saturating_sub(lead + 1))),
            ));
            // The reason can be a full sentence: wrap it under the title
            // rather than let the terminal break it mid-word.
            for line in wrap(&action.why, width.saturating_sub(lead + 1)) {
                out.push_str(&format!("{pad}{}\n", dim(color, &line)));
            }
            out.push_str(&format!("{pad}{}\n", sgr(color, "36", how)));
        }
    }
    out.push('\n');

    // ── At a glance: the same six figures every surface leads with.
    let f = &o.figures;
    let figures: [(String, &str, String); 6] = [
        (
            f.not_backed_up.to_string(),
            "not backed up",
            format!(
                "{} never pushed, {} with unpushed commits",
                f.unpublished, f.unpushed_repos
            ),
        ),
        (
            f.stashes.to_string(),
            if f.stashes == 1 { "stash" } else { "stashes" },
            format!(
                "in {}, {} older than a week",
                plural(f.stash_repos, "repo", "repos"),
                f.stale_stash_repos
            ),
        ),
        (
            f.uncommitted_files.to_string(),
            if f.uncommitted_files == 1 {
                "uncommitted file"
            } else {
                "uncommitted files"
            },
            format!("across {}", plural(f.dirty_repos, "repo", "repos")),
        ),
        (
            f.stale_fetch_repos.to_string(),
            if f.stale_fetch_repos == 1 {
                "stale fetch"
            } else {
                "stale fetches"
            },
            if f.stale_fetch_repos == 0 {
                "every remote fetched recently".to_string()
            } else {
                format!("oldest {} days", f.oldest_fetch_days)
            },
        ),
        (
            f.open_tasks.to_string(),
            if f.open_tasks == 1 {
                "open task"
            } else {
                "open tasks"
            },
            format!(
                "{} high priority, {} in progress",
                f.high_tasks, f.doing_tasks
            ),
        ),
        (
            f.commits_week.to_string(),
            if f.commits_week == 1 {
                "commit, 7 days"
            } else {
                "commits, 7 days"
            },
            format!("{} the week before", f.commits_prev_week),
        ),
    ];
    out.push_str(&section("at a glance", "the fleet in six numbers", color));
    let num_w = figures.iter().map(|(n, _, _)| n.len()).max().unwrap_or(1);
    // indent + number + label column, then the detail in what is left.
    let detail_w = width.saturating_sub(4 + num_w + 2 + 18 + 2 + 1).max(8);
    for (number, label, detail) in &figures {
        let detail = &clip(detail, detail_w);
        out.push_str(&format!(
            "    {}  {}  {}\n",
            bold(color, &format!("{number:>num_w$}")),
            format_args!("{label:<18}"),
            dim(color, detail),
        ));
    }
    out.push('\n');

    // ── Work at risk.
    let risky: Vec<&cohors_core::RepoLine> = o.at_risk().collect();
    if !risky.is_empty() {
        out.push_str(&section(
            "work at risk",
            &format!("{} where local work could be lost", risky.len()),
            color,
        ));
        // The same row as `cohors scan`, sized to this terminal.
        let widths = RowWidths::fit(risky.iter().copied(), width);
        for repo in risky {
            out.push_str(&scan_row(repo, widths, now, color));
        }
        out.push('\n');
    }
    out
}

/// A bulk git action issued from the CLI. Mirrors the TUI verbs and the MCP
/// action tools — one core (`crate::action`), three surfaces.
#[derive(Clone)]
pub enum CliAction {
    Fetch,
    Pull,
    Push,
    /// `None` means the message was omitted, which prints the guide. The
    /// options say how to stage and whether secrets may go in.
    Commit(Option<String>, cohors_actions::CommitOptions),
    Stash,
}

impl CliAction {
    /// The verb used in the human-facing summary lines.
    fn verb(&self) -> &'static str {
        match self {
            CliAction::Fetch => "fetch",
            CliAction::Pull => "pull",
            CliAction::Push => "push",
            CliAction::Commit(..) => "commit",
            CliAction::Stash => "stash",
        }
    }

    /// Present participle, for the "doing it now" heading.
    fn doing(&self) -> &'static str {
        match self {
            CliAction::Fetch => "fetching",
            CliAction::Pull => "pulling",
            CliAction::Push => "pushing",
            CliAction::Commit(..) => "committing",
            CliAction::Stash => "stashing",
        }
    }

    /// The extra argument this verb needs beyond a selector, shown in the guide's
    /// worked examples so the shape of a real invocation is obvious.
    fn arg_hint(&self) -> &'static str {
        match self {
            CliAction::Commit(..) => "-m \"your message\"",
            _ => "",
        }
    }

    /// Whether a required argument other than the selector is still missing, which
    /// (like a missing selector) sends the user to the guide rather than an error.
    fn missing_arg(&self) -> bool {
        matches!(self, CliAction::Commit(None, _))
    }

    /// What the verb does, and why it is safe to run. Shown in the command's own
    /// guide and above every run, so the safety rule is never a surprise.
    fn blurb(&self) -> (&'static str, &'static str) {
        match self {
            CliAction::Fetch => (
                "git fetch across the repos you pick",
                "safe anywhere: it never touches your working tree",
            ),
            CliAction::Pull => (
                "git pull --ff-only across the repos you pick",
                "fast-forward only, so it can never merge or lose work",
            ),
            CliAction::Push => (
                "git push across the repos you pick",
                "never force-pushes, so remote history stays safe",
            ),
            CliAction::Commit(..) => (
                "git add -A and commit across the repos you pick",
                "stages tracked and untracked files, and never amends",
            ),
            CliAction::Stash => (
                "git stash push across the repos you pick",
                "shelves your changes, nothing is thrown away",
            ),
        }
    }
}

/// A guide row: `▸ command   what it does`, the launch screen's item grammar.
fn guide_row(cmd: &str, why: &str, width: usize, color: bool) -> String {
    look::guide_row(cmd, why, width, color)
}

/// The masthead and footer clap wraps around `cohors --help`, so the help page
/// opens and closes the same way the dashboard and the reports do.
///
/// These are built here rather than as `#[command(before_help = …)]` attributes
/// because whether to paint them depends on the terminal, which is only knowable
/// at runtime. `main` attaches them before parsing.
pub fn help_decorations() -> (String, String) {
    let color = wants_color(std::io::stdout().is_terminal());

    // clap prints its own blank line after `{before-help}`, so trim ours.
    let mut before = masthead(
        "every git repo you own, on one screen",
        "run it bare for the dashboard, or pick a command below",
        color,
    );
    while before.ends_with('\n') {
        before.pop();
    }

    // A flag list tells a new reader what exists, not what to do next. These
    // are the shortest paths to actually seeing, and acting on, their own fleet.
    let rows = [
        (
            "cohors demo",
            "the whole dashboard on sample data, no config, nothing touched",
        ),
        (
            "cohors --root ~/code",
            "skip the config and scan one folder right now",
        ),
        (
            "cohors init",
            "write a starter config, then just run `cohors`",
        ),
        (
            "cohors overview",
            "what to do next across every repo, and why, as a report",
        ),
        (
            "cohors agent run setup-repo",
            "have your coding agent set the current repo up for the launchpad",
        ),
    ];
    let w = rows
        .iter()
        .map(|(c, _)| c.chars().count())
        .max()
        .unwrap_or(0);
    let mut after = String::new();
    after.push_str(&section(
        "start here",
        "--root and --config work on every command, and nothing writes without a selector",
        color,
    ));
    for (cmd, why) in rows {
        after.push_str(&guide_row(cmd, why, w, color));
    }
    (before, after)
}

/// The selector vocabulary, shared by every action's guide.
const SELECTORS: [(&str, &str); 8] = [
    ("dirty", "repos with uncommitted work"),
    ("behind", "repos behind their upstream"),
    ("ahead", "repos with unpushed commits"),
    ("unpublished", "branches that were never pushed"),
    ("stash", "repos holding stashes"),
    ("attention", "anything that wants you"),
    ("name:pay*", "by name, globs allowed"),
    ("all", "the whole fleet, deliberately"),
];

/// What a bulk action prints when no selector was given: what the verb does, how
/// to pick repos, and where to start if you are new. A selector is required so an
/// action never silently hits every repo you own, and a missing one is a question
/// to answer, not an error to punish.
fn action_guide(verb: &str, arg: &str, does: &str, safety: &str, color: bool) -> String {
    let mut out = masthead(&format!("{verb}: {does}"), safety, color);

    // Verbs needing more than a selector (`commit -m`, `run <cmd>`) carry it into
    // every example, so the shape of a real invocation is never guesswork.
    let head = if arg.is_empty() {
        format!("cohors {verb}")
    } else {
        format!("cohors {verb} {arg}")
    };
    let picks: Vec<(String, &str)> = SELECTORS
        .iter()
        .map(|(s, why)| (format!("{head} --select {s}"), *why))
        .collect();
    let starts: Vec<(String, &str)> = vec![
        (
            "cohors".to_string(),
            "open the dashboard and act from there",
        ),
        (
            "cohors scan".to_string(),
            "see what is going on, changes nothing",
        ),
        (
            format!("{head} --select dirty --dry-run"),
            "preview the targets without acting",
        ),
    ];
    // One width across both blocks so every description starts in the same column.
    let w = picks
        .iter()
        .chain(starts.iter())
        .map(|(c, _)| c.chars().count())
        .max()
        .unwrap_or(0);

    out.push_str(&section(
        "pick what to act on",
        "cohors never touches the whole fleet unless you say so",
        color,
    ));
    for (cmd, why) in &picks {
        out.push_str(&guide_row(cmd, why, w, color));
    }
    out.push('\n');

    out.push_str(&section("new here?", "look before you leap", color));
    for (cmd, why) in &starts {
        out.push_str(&guide_row(cmd, why, w, color));
    }
    out.push('\n');
    out
}

/// Resolve a `--select` query to the actionable repos: the matching, readable
/// repos in dirty-first order, using the same [`cohors_core::resolve`] the
/// dashboard and MCP use — so `cohors push --select behind` hits exactly what
/// `scan --select behind` lists.
fn action_targets<'a>(
    snaps: &'a [cohors_core::RepoSnapshot],
    select: &str,
) -> Result<Vec<&'a cohors_core::RepoSnapshot>> {
    let selector = parse_selector(select)?;
    let order = cohors_core::resolve(snaps, &selector, SortMode::DirtyFirst, now_secs());
    let by_id: HashMap<&str, &cohors_core::RepoSnapshot> =
        snaps.iter().map(|s| (s.id.0.as_str(), s)).collect();
    Ok(order
        .iter()
        .filter_map(|id| by_id.get(id.0.as_str()).copied())
        .filter(|s| !s.has_error() && s.path.is_some())
        .collect())
}

/// `cohors fetch|pull|push|commit|stash --select <q>` — run a bulk git action
/// across the matching repos. The human typing the command is the consent, so
/// there are no capability flags here (unlike the agent-facing MCP server);
/// `--dry-run` previews the targets without acting. Safety still holds in the
/// action layer: pull is ff-only, push never force-pushes, stash/commit can't
/// lose work.
pub fn run_action(cli: &Cli, action: CliAction, select: Option<&str>, dry_run: bool) -> Result<()> {
    let color = wants_color(std::io::stdout().is_terminal());
    let verb = action.verb();
    let (does, safety) = action.blurb();

    // A missing selector (or a commit with no message) is a question, not an
    // error: say what this verb does and how to choose repos, rather than failing
    // with a bare "required argument" message.
    // A selector but no usable message is a mistake, not a question: name it.
    if let (Some(select), CliAction::Commit(message, _)) = (select, &action)
        && message.as_deref().is_none_or(|m| m.trim().is_empty())
    {
        eprintln!(
            "cohors: commit needs a message: cohors commit -m \"your message\" --select {select}"
        );
        return Err(ExitCode(2).into());
    }
    let select = match select {
        Some(s) if !action.missing_arg() => s,
        _ => {
            print!(
                "{}",
                action_guide(verb, action.arg_hint(), does, safety, color)
            );
            return Ok(());
        }
    };

    let scanner = crate::scan::from_cli(cli)?;
    let snapshots = scanner.scan();
    let targets = action_targets(&snapshots, select)?;
    print!("{}", masthead(&format!("{verb}: {does}"), safety, color));

    if targets.is_empty() {
        out_nothing_matched(verb, select, color);
        return Ok(());
    }
    let n = targets.len();
    let plural = if n == 1 { "" } else { "s" };
    if dry_run {
        print!(
            "{}",
            section(
                "dry run",
                &format!("would {verb} {n} repo{plural}, nothing will change"),
                color
            )
        );
        let w = targets
            .iter()
            .map(|s| s.name.chars().count())
            .max()
            .unwrap_or(0);
        for s in &targets {
            print!(
                "{}",
                guide_row(&s.name, s.path.as_ref().unwrap().as_str(), w, color)
            );
            // A commit's preview names the files, so nothing goes in unseen.
            if let CliAction::Commit(_, opts) = &action {
                print!("{}", commit_preview(s.path.as_ref().unwrap(), *opts, color));
            }
        }
        println!();
        return Ok(());
    }

    // Pull/stash/commit mutate the worktree: a conflicted or mid-operation
    // repo is reported and skipped (same `worktree_block` rule every surface
    // applies); fetch/push never touch the worktree.
    let guard_worktree = matches!(
        action,
        CliAction::Pull | CliAction::Stash | CliAction::Commit(..)
    );
    print!(
        "{}",
        section(action.doing(), &format!("{n} repo{plural}"), color)
    );
    let mut ok = 0usize;
    let mut skipped = 0usize;
    // Skips first (they need no git), then every repo at once on the shared
    // bounded pool the MCP server and the web use (ADR-050), each line printed
    // the moment its repo finishes, so a big fleet shows progress, not a pause.
    let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
    let mut started = 0usize;
    for s in &targets {
        if guard_worktree && let Some(reason) = cohors_actions::worktree_block(s) {
            skipped += 1;
            println!("    {} {}: {reason}", dim(color, "·"), dim(color, &s.name));
            continue;
        }
        let path = s.path.clone().unwrap_or_default();
        let name = s.name.clone();
        let action = action.clone();
        let tx = tx.clone();
        started += 1;
        cohors_actions::spawn_bounded(move || {
            let result = match &action {
                CliAction::Fetch => crate::action::fetch(&path, &name),
                CliAction::Pull => crate::action::pull_ff(&path, &name),
                CliAction::Push => crate::action::push(&path, &name),
                // Guarded above: a commit with no message never gets here.
                CliAction::Commit(message, opts) => cohors_actions::commit_with(
                    &path,
                    &name,
                    message.as_deref().unwrap_or_default(),
                    *opts,
                ),
                CliAction::Stash => crate::action::stash_push(&path, &name),
            };
            let _ = tx.send(result);
        });
    }
    drop(tx);
    let mut refused_secret = false;
    for result in rx.iter().take(started) {
        match result {
            Ok(message) => {
                ok += 1;
                println!("    {} {message}", sgr(color, "32", "✓"));
            }
            Err(message) => {
                refused_secret |= message.contains("looks like a secret");
                println!("    {} {message}", sgr(color, "31", "✗"));
            }
        }
        let _ = std::io::stdout().flush();
    }
    let mut tail = format!("{ok} of {n} ok");
    if skipped > 0 {
        tail.push_str(&format!(", {skipped} skipped"));
    }
    println!("\n  {}\n", bold(color, &tail));
    if refused_secret {
        println!(
            "  {}\n",
            dim(
                color,
                "A repo was refused because a file looks like a secret. Ignore it in .gitignore, commit with --staged after staging by hand, or pass --allow-secrets."
            )
        );
    }
    // Scripts and CI read the status: a repo that failed fails the command.
    // Skipped repos (conflicted, mid-operation) were left alone on purpose.
    if ok + skipped < n {
        return Err(ExitCode(1).into());
    }
    Ok(())
}

/// The files one repo's commit would include, under its row in a dry run:
/// what `git add -A` would stage (or, `--staged`, the index), each with its
/// porcelain status, and any that look like secrets marked as refused.
fn commit_preview(path: &Utf8Path, opts: cohors_actions::CommitOptions, color: bool) -> String {
    let files = match cohors_actions::paths_to_commit(path, opts.staged_only) {
        Ok(files) => files,
        Err(err) => return format!("        {}\n", dim(color, &err)),
    };
    if files.is_empty() {
        return format!("        {}\n", dim(color, "nothing to commit"));
    }
    let secrets = cohors_actions::secret_paths(&files);
    let status: HashMap<String, String> = cohors_fleet::repo_changes(path, false, 0)
        .files
        .into_iter()
        .map(|f| (f.path, f.status))
        .collect();
    const SHOWN: usize = 12;
    let mut out = String::new();
    for file in files.iter().take(SHOWN) {
        let code = status.get(file).map(String::as_str).unwrap_or("  ");
        let shown = crate::ui::text::safe(file);
        if secrets.contains(file) {
            let note = if opts.allow_secrets {
                "looks like a secret; --allow-secrets commits it"
            } else {
                "looks like a secret; the commit will be refused"
            };
            out.push_str(&format!(
                "        {} {}  {}\n",
                dim(color, code),
                sgr(color, "1;31", &shown),
                sgr(color, "31", note)
            ));
        } else {
            out.push_str(&format!("        {} {}\n", dim(color, code), shown));
        }
    }
    if files.len() > SHOWN {
        out.push_str(&format!(
            "        {}\n",
            dim(color, &format!("and {} more", files.len() - SHOWN))
        ));
    }
    out
}

/// The "your selector matched nothing" state: never a bare error, always a way
/// forward. Shared by every bulk action and by `run`.
fn out_nothing_matched(verb: &str, select: &str, color: bool) {
    print!(
        "{}",
        section(
            "nothing matched",
            &format!("no repos match --select {select}"),
            color
        )
    );
    let rows: Vec<(String, &str)> = vec![
        (
            "cohors scan".to_string(),
            "see the whole fleet and its state",
        ),
        (
            format!("cohors {verb} --select all"),
            "act on every repo, deliberately",
        ),
        (
            format!("cohors {verb}"),
            "the selector vocabulary for this command",
        ),
    ];
    let w = rows
        .iter()
        .map(|(c, _)| c.chars().count())
        .max()
        .unwrap_or(0);
    for (cmd, why) in &rows {
        print!("{}", guide_row(cmd, why, w, color));
    }
    println!();
}

/// `cohors run <command> --select <q>` — run a shell command in each matching
/// repo, bounded per-repo by `timeout` seconds, printing each repo's output and
/// a pass/fail summary. `--dry-run` previews the targets without running.
pub fn run_command_action(
    cli: &Cli,
    select: Option<&str>,
    command: Option<&str>,
    timeout: u64,
    dry_run: bool,
) -> Result<()> {
    let color = wants_color(std::io::stdout().is_terminal());
    let does = "run a shell command in each repo you pick";
    let safety = "arbitrary shell, bounded by a per-repo timeout";

    // `run` needs both a command and a selector. Missing either explains the verb
    // instead of failing with a clap error.
    if let (None, Some(select)) = (command, select) {
        eprintln!("cohors: run needs a command: cohors run \"your command\" --select {select}");
        return Err(ExitCode(2).into());
    }
    let (Some(command), Some(select)) = (command, select) else {
        print!(
            "{}",
            action_guide("run", "\"your command\"", does, safety, color)
        );
        return Ok(());
    };

    let scanner = crate::scan::from_cli(cli)?;
    let snapshots = scanner.scan();
    let targets = action_targets(&snapshots, select)?;
    print!("{}", masthead(&format!("run: {does}"), safety, color));

    if targets.is_empty() {
        out_nothing_matched("run", select, color);
        return Ok(());
    }
    let n = targets.len();
    let plural = if n == 1 { "" } else { "s" };
    if dry_run {
        print!(
            "{}",
            section(
                "dry run",
                &format!("would run `{command}` in {n} repo{plural}, nothing will change"),
                color
            )
        );
        let w = targets
            .iter()
            .map(|s| s.name.chars().count())
            .max()
            .unwrap_or(0);
        for s in &targets {
            print!(
                "{}",
                guide_row(&s.name, s.path.as_ref().unwrap().as_str(), w, color)
            );
        }
        println!();
        return Ok(());
    }

    let timeout = std::time::Duration::from_secs(timeout.max(1));
    // The shared bounded-pool runner (ADR-020) executes the targets concurrently
    // and returns results in target order, so we zip them back to print by name.
    let owned: Vec<cohors_core::RepoSnapshot> = targets.iter().map(|s| (*s).clone()).collect();
    // The jobs run in their own process groups (so a timeout can stop all
    // they start), which also keeps the terminal's Ctrl-C from reaching them:
    // forward it, so Ctrl-C stops every job instead of orphaning them.
    let interrupt = cohors_actions::interrupt_flag();
    #[cfg(unix)]
    let sigint =
        signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&interrupt)).ok();
    let results = cohors_actions::run_each(&owned, command, timeout);
    #[cfg(unix)]
    if let Some(id) = sigint {
        signal_hook::low_level::unregister(id);
    }
    if interrupt.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!("\ncohors: interrupted; every running command was stopped.");
        return Err(ExitCode(130).into());
    }
    let mut ok = 0usize;
    for (s, r) in targets.iter().zip(&results) {
        if r.ok {
            ok += 1;
        }
        let (mark, tag) = if r.timed_out {
            (sgr(color, "33", "!"), "timed out".to_string())
        } else if r.ok {
            (sgr(color, "32", "✓"), "ok".to_string())
        } else {
            (
                sgr(color, "31", "✗"),
                format!("failed (exit {})", r.exit_code),
            )
        };
        println!("\n  {mark} {}  {}", bold(color, &s.name), dim(color, &tag));
        if !r.stdout.trim().is_empty() {
            print!("{}", r.stdout);
        }
        if !r.stderr.trim().is_empty() {
            eprint!("{}", r.stderr);
        }
    }
    println!("\n  {}\n", bold(color, &format!("{ok} of {n} ok")));
    if ok < n {
        return Err(ExitCode(1).into());
    }
    Ok(())
}

/// Current Unix time in seconds (for the clock-dependent `attention` predicate).
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse a `--select` value into a [`Selector`]. A value starting with `{` is
/// JSON; anything else is the shared shorthand (parsed in `cohors-core`, so
/// every surface speaks the same language). `~` expansion reads `$HOME` here —
/// an environment concern that stays out of core.
fn parse_selector(query: &str) -> Result<Selector> {
    let query = query.trim();
    let mut selector = if query.starts_with('{') {
        serde_json::from_str::<Selector>(query).context("parsing --select JSON")?
    } else {
        cohors_core::parse_shorthand(query)?
    };
    cohors_core::expand_selector_tilde(&mut selector, std::env::var("HOME").ok().as_deref());
    Ok(selector)
}

/// The branded local host the dashboard is served at. `*.localhost` is reserved
/// loopback (RFC 6761) — modern browsers resolve it to 127.0.0.1 with no
/// `/etc/hosts` edit and no privileges — so `http://cohors.localhost:<port>` is a
/// clean, device-local URL that just works.
const WEB_HOST: &str = "cohors.localhost";

/// `cohors web` — one command to serve and open the web dashboard.
///
/// The page comes from one of two places. A release build carries it inside
/// the binary (see `build.rs`), so it is served from memory and nothing else is
/// needed. A build without it (from source) builds the page itself: it locates
/// the `cohors-web` crate in a checkout, ensures Trunk (the WASM bundler) is
/// installed, and keeps `trunk watch` running so an edit rebuilds the page.
/// Either way it serves on loopback, waits until the server accepts
/// connections, then prints and opens the branded local URL
/// (`http://cohors.localhost:<port>`). Blocks until Ctrl-C.
pub fn run_web(
    cli: &Cli,
    port: u16,
    open: bool,
    install: bool,
    allow_writes: bool,
    allow_run: bool,
    demo: bool,
) -> Result<()> {
    // A release build has the page compiled in; anything else must build it,
    // which needs a checkout (the developer path, with Trunk).
    let built_in = !crate::web_assets::WEB_ASSETS.is_empty();
    let web_dir = if built_in {
        None
    } else {
        Some(find_web_crate().context(
            "this cohors was built without the browser dashboard inside it. The release \
             builds (Homebrew, pip, the shell installer) include it. From source, run \
             `cohors web` inside a clone of the cohors repository, where it builds the page \
             with Trunk",
        )?)
    };
    // The web app is just another front-end over the SAME local scan the TUI,
    // CLI, and MCP run: discover the repos under `--root`/config, snapshot their
    // local state, and (with a token) enrich with remote CI/PRs. The server does
    // the scan and serves the `cohors-core` snapshots as JSON; the browser
    // renders them through the same `assess`/sort logic. The token is the SAME
    // one the TUI uses (`gh auth token` / `$GITHUB_TOKEN`) and never leaves here.
    let scanner = std::sync::Arc::new(crate::scan::from_cli(cli)?);

    // Fresh-user gate. The dashboard is only useful once cohors knows where the
    // repos live; launching into an empty (demo) fleet just confuses a first-time
    // user. So mirror the TUI's empty-state rescue: if nothing is found, point
    // them at `cohors` (which detects the roots and saves them to the config),
    // then back to `cohors web`. Checked *before* the Trunk install + WASM build,
    // so a first run isn't spent waiting only to be told to start over.
    // `--demo` serves the built-in sample fleet (like `cohors demo`): skip the
    // real scan and this fresh-user gate entirely.
    if !demo && scanner.scan().is_empty() {
        eprint!("{}", web_setup_hint(&scanner.roots(), stderr_wants_color()));
        return Ok(());
    }

    if web_dir.is_some() {
        ensure_trunk(install)?;
    }

    // If the requested port is busy (e.g. another `cohors web`), step to the next
    // free one rather than failing.
    let color = wants_color(std::io::stdout().is_terminal());
    let chosen = pick_port(port);
    if chosen != port {
        eprintln!(
            "  {}",
            dim(
                color,
                &format!("port {port} is in use, serving on {chosen} instead")
            )
        );
    }
    let port = chosen;

    let token = scanner.github_token();

    // Build the WASM assets to dist/ and keep watching for rebuilds while we
    // serve. (Our own server — not `trunk serve` — so we can proxy GitHub.)
    // The masthead lands before Trunk's own logs so the brand opens the run.
    print!(
        "{}",
        masthead(
            "web: the dashboard in your browser",
            if built_in {
                "the page is built in, starting the server"
            } else {
                "building the WASM bundle, this takes a moment"
            },
            color
        )
    );
    let mut watcher = None;
    let assets = match &web_dir {
        None => cohors_web::Assets::Embedded(crate::web_assets::WEB_ASSETS),
        Some(web_dir) => {
            let child = std::process::Command::new("trunk")
                .arg("watch")
                .current_dir(web_dir)
                .spawn()
                .context("starting `trunk watch` (is Trunk installed and on PATH?)")?;
            let watching = watcher.insert(child);
            let dist = web_dir.join("dist");
            if !wait_for_file(
                &dist.join("index.html"),
                std::time::Duration::from_secs(240),
            ) {
                let _ = watching.kill();
                bail!("the WASM build didn't finish in time");
            }
            cohors_web::Assets::Dir(dist)
        }
    };

    let url = format!("http://{WEB_HOST}:{port}");
    // What it is serving, and what that means, in the same two-column grammar the
    // rest of the CLI uses.
    let source = if demo {
        (
            "the built-in demo fleet".to_string(),
            "sample data, nothing on disk is read",
        )
    } else {
        (
            scanner
                .roots()
                .iter()
                .map(|r| crate::ui::abbrev_home(r))
                .collect::<Vec<_>>()
                .join(", "),
            "scanning your fleet live",
        )
    };
    let auth = if demo {
        ("no GitHub needed".to_string(), "the demo fleet is offline")
    } else if token.is_some() {
        (
            "GitHub login found".to_string(),
            "remote CI and PRs are enriched",
        )
    } else {
        (
            "no GitHub login".to_string(),
            "local status only, run `gh auth login` for CI and PRs",
        )
    };
    let rows: Vec<(String, &str)> = vec![source, auth, ("Ctrl-C".to_string(), "stop the server")];
    let w = rows
        .iter()
        .map(|(c, _)| c.chars().count())
        .max()
        .unwrap_or(0);
    print!("{}", section("live", &url, color));
    for (what, why) in &rows {
        print!("{}", guide_row(what, why, w, color));
    }
    println!();
    if open {
        let url = url.clone();
        std::thread::spawn(move || {
            if wait_for_port(port, std::time::Duration::from_secs(15)) {
                let _ = open_url(&url);
            }
        });
    }

    // Serve until stopped; then tear down the watcher. `--watch` makes the page
    // poll `/api/repos` so a fresh scan shows up without a manual rescan. The
    // server lives in `cohors-web` now (the write seam is no longer trapped in
    // this binary); we hand it a scan closure + the shared safety config.
    let actions_config = scanner.actions_config();
    let options = cohors_web::ServeOptions {
        port,
        roots: scanner.roots(),
        token,
        watch: cli.watch,
        caps: cohors_web::Caps {
            allow_writes,
            allow_run,
        },
        allowlist: actions_config.run_allowlist,
        max_targets: actions_config.max_action_targets,
        demo,
        author_email: scanner.author_email(),
    };
    let scan: std::sync::Arc<cohors_web::ScanFn> = if demo {
        let now = now_secs();
        std::sync::Arc::new(move || cohors_core::demo::fleet(now))
    } else {
        let scanner = scanner.clone();
        std::sync::Arc::new(move || scanner.scan())
    };
    let result = cohors_web::serve(assets, scan, options);
    if let Some(mut watching) = watcher {
        let _ = watching.kill();
    }
    result
}

// ── First-run welcome (shared by `cohors` and `cohors web`) ─────────────────
// These build the branded greeting the CLI prints before any dashboard appears.
// The CLI doesn't route through ratatui, so we emit our own ANSI — all of it
// gated on `color`, set only for a real terminal with colour allowed, so pipes,
// redirects, and `NO_COLOR` degrade to clean plain text.

/// True when `is_tty` and `NO_COLOR` is unset — the rule for emitting ANSI.
fn wants_color(is_tty: bool) -> bool {
    is_tty && std::env::var_os("NO_COLOR").is_none()
}

/// Whether the `cohors web` hint (which goes to stderr) should be styled.
fn stderr_wants_color() -> bool {
    wants_color(std::io::stderr().is_terminal())
}

/// Wrap `s` in an ANSI SGR sequence when `color`, else return it unchanged.
fn sgr(color: bool, code: &str, s: &str) -> String {
    if color {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Brand purple, bold — kept in lockstep with the TUI via `SPIDER_PURPLE_RGB`.
fn brand(color: bool, s: &str) -> String {
    let (r, g, b) = crate::ui::SPIDER_PURPLE_RGB;
    sgr(color, &format!("1;38;2;{r};{g};{b}"), s)
}
fn bold(color: bool, s: &str) -> String {
    sgr(color, "1", s)
}
fn dim(color: bool, s: &str) -> String {
    sgr(color, "2", s)
}

/// The guidance a front-end prints when it finds no repos anywhere: where it
/// looked, then the three shortest ways to get a fleet on screen. `cmd` is the
/// invoking command (`cohors` or `cohors web`) so the sample line matches.
/// `color` gates the ANSI; otherwise pure so it's unit-testable.
fn nothing_found_hint(cmd: &str, roots: &[String], color: bool) -> String {
    let mut out = masthead(
        "No git repositories found",
        &format!("looked under {}", roots.join(", ")),
        color,
    );
    let rows = [
        (format!("{cmd} --root ~/code"), "look in one folder"),
        (
            "cohors init".to_string(),
            "detect where your repos live and save it",
        ),
        (
            "cohors demo".to_string(),
            "try the dashboard on sample data first",
        ),
    ];
    let w = rows
        .iter()
        .map(|(c, _)| c.chars().count())
        .max()
        .unwrap_or(0);
    out.push_str(&section(
        "point cohors at your code",
        "any of these puts a fleet on screen",
        color,
    ));
    for (cmd, why) in &rows {
        out.push_str(&guide_row(cmd, why, w, color));
    }
    out.push_str(&format!(
        "\n  {}\n",
        dim(
            color,
            "With no --root, cohors gathers every folder it detects into one super fleet."
        )
    ));
    out
}

/// The `cohors web` no-repos guidance.
fn web_setup_hint(roots: &[String], color: bool) -> String {
    nothing_found_hint("cohors web", roots, color)
}

/// The bare-`cohors` no-repos guidance.
fn tui_welcome(roots: &[String], color: bool) -> String {
    nothing_found_hint("cohors", roots, color)
}

/// Outcome of the first-run onboarding step.
enum Onboard {
    /// Repos exist somewhere — launch the dashboard on the super fleet.
    Proceed,
    /// Nothing was found anywhere — guidance was printed; don't launch the TUI.
    Quit,
}

/// The config path: the `--config` override, else the default location.
fn config_path(cli: &Cli) -> Result<Utf8PathBuf> {
    match &cli.config {
        Some(p) => Ok(Utf8PathBuf::from(p)),
        None => cohors_config::paths::config_file().context("resolving the config path"),
    }
}

/// How many "Do next" steps the launch screen shows before pointing at the
/// dashboard for the rest.
const LAUNCH_STEPS: usize = 4;

/// The launch screen: what cohors found and the first things worth doing,
/// shown in the plain terminal before the dashboard takes the screen. It is
/// the dashboard's own headline and "Do next" list, so the two never disagree.
///
/// `first_run` is the bare `cohors` with no config: it also lists the folders
/// that were gathered and how to look in just one. `color` gates the ANSI;
/// otherwise pure so it's unit-testable.
fn launch_screen(
    overview: &cohors_core::Overview,
    roots: &[String],
    first_run: bool,
    color: bool,
) -> String {
    // On a first run the folders get their own section; the masthead only
    // counts them, so one long list cannot push the headline off the line.
    let where_at = if first_run && roots.len() > 1 {
        format!("gathered from {}", plural(roots.len(), "folder", "folders"))
    } else {
        roots.join(", ")
    };
    let mut out = masthead(&overview.headline, &where_at, color);

    if overview.actions.is_empty() {
        out.push_str(&section("do next", "nothing needs you right now", color));
    } else {
        out.push_str(&section("do next", "most urgent first", color));
        for action in overview.actions.iter().take(LAUNCH_STEPS) {
            out.push_str(&format!(
                "    {}  {}\n",
                severity_mark(action.severity, color),
                bold(color, &action.title),
            ));
        }
        let more = overview.actions.len().saturating_sub(LAUNCH_STEPS);
        if more > 0 {
            out.push_str(&format!(
                "    {}  {}\n",
                " ".repeat(BLOCK_W),
                dim(color, &format!("and {more} more in the dashboard"))
            ));
        }
    }
    out.push('\n');

    if first_run {
        out.push_str(&section(
            "looking in",
            "every folder where repositories were found",
            color,
        ));
        for root in roots {
            out.push_str(&guide_row(root, "", 0, color));
        }
        out.push_str(&guide_row(
            "cohors --root <dir>",
            "to look in one folder only",
            0,
            color,
        ));
        out.push('\n');
    }

    out.push_str(&key_bar(
        &[("enter", "open the dashboard"), ("q", "quit")],
        color,
    ));
    out
}

/// Read a key for the launch prompt: Enter / `y` launches, `q` / `Esc` /
/// `Ctrl-C` quits, and any other key is ignored (keep waiting). Raw mode so a
/// lone keypress is enough — no Enter required for the `q`/`y` shortcuts.
fn prompt_launch() -> Result<bool> {
    use ratatui::crossterm::event::{self, Event, KeyCode, KeyModifiers};
    use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode};

    enable_raw_mode().context("entering raw mode for the launch prompt")?;
    let launch = loop {
        match event::read() {
            Ok(Event::Key(k)) => match k.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => break true,
                KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => break false,
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => break false,
                _ => continue,
            },
            Ok(_) => continue,
            Err(_) => break false,
        }
    };
    let _ = disable_raw_mode();
    println!(); // land the cursor on a fresh line before the TUI / shell prompt
    Ok(launch)
}

/// The launch sequence, in the plain terminal *before* the dashboard takes the
/// screen: scan, show what was found and what to do next, then wait for Enter
/// (open the dashboard) or `q` (quit).
///
/// Scanning first is what makes the screen worth reading: it reports real
/// counts instead of only naming a folder. The scan is also written to the
/// cache, so the dashboard opens already populated rather than scanning again
/// from nothing.
///
/// An empty scan skips the screen: the dashboard's own empty state explains
/// what to do (and offers to adopt the folders it detected).
fn launch(scanner: &crate::scan::Scanner, roots: &[String], first_run: bool) -> Result<Onboard> {
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let color = wants_color(std::io::stdout().is_terminal());

    // Discovery walks the disk, which on a large fleet is a beat or two of
    // silence. Say what is happening, then wipe the line.
    let announce = interactive && std::io::stderr().is_terminal();
    if announce {
        eprint!("  {} {}…", dim(color, "scanning"), roots.join(", "));
        let _ = std::io::stderr().flush();
    }
    let repos = scanner.scan();
    if announce {
        eprint!("\r\x1b[2K"); // carriage return, then erase the line
        let _ = std::io::stderr().flush();
    }
    crate::cache::save(&repos);
    if repos.is_empty() {
        return Ok(Onboard::Proceed);
    }

    let home = cohors_fleet::launchpad::launchpad_home();
    let overview = cohors_fleet::overview(&repos, home.as_deref(), now_secs());
    print!("{}", launch_screen(&overview, roots, first_run, color));
    let _ = std::io::stdout().flush();

    // A real terminal is needed both to read the key and to run the dashboard
    // at all; without one the screen was printed as a report and we stop.
    if interactive && prompt_launch()? {
        Ok(Onboard::Proceed)
    } else {
        Ok(Onboard::Quit)
    }
}

/// First run of a bare `cohors` (no `--root`, no config). cohors auto-detects
/// where the user keeps repos (the super fleet). When nothing is found it
/// prints guidance instead of launching an empty dashboard.
///
/// `watch` skips the launch screen. `--watch` is a live-monitoring mode people
/// re-run in a loop, so a "press Enter" gate there is a keypress on every
/// iteration. The empty-fleet guard still applies: watching nothing is worse
/// than being told why.
fn onboard(scanner: &Arc<crate::scan::Scanner>, watch: bool) -> Result<Onboard> {
    let home = cohors_config::paths::home_dir().ok();
    let detected = cohors_fleet::detect_roots(home.as_deref());

    if detected.is_empty() {
        let color = wants_color(std::io::stdout().is_terminal());
        print!("{}", tui_welcome(&scanner.roots(), color));
        let _ = std::io::stdout().flush();
        return Ok(Onboard::Quit);
    }
    if watch {
        return Ok(Onboard::Proceed);
    }
    let roots: Vec<String> = detected.iter().map(|r| crate::ui::abbrev_home(r)).collect();
    launch(scanner, &roots, true)
}

/// `cohors --root <dir>`: the launch screen for the folder(s) named. As with
/// [`onboard`], `--watch` skips it and goes straight to the live dashboard.
fn scoped_onboard(scanner: &Arc<crate::scan::Scanner>, watch: bool) -> Result<Onboard> {
    if watch {
        return Ok(Onboard::Proceed);
    }
    let roots: Vec<String> = scanner
        .roots()
        .iter()
        .map(|r| crate::ui::abbrev_home(r))
        .collect();
    launch(scanner, &roots, false)
}

/// Make sure Trunk (the WASM bundler) is available, installing it via Cargo when
/// missing (unless `install` is false). Falls back to pointing at a binary
/// install when Cargo isn't present (a prebuilt-binary, non-Rust setup).
fn ensure_trunk(install: bool) -> Result<()> {
    if trunk_available() {
        return Ok(());
    }
    if !install {
        bail!(
            "the web app needs Trunk (the WASM bundler). Install it with:\n\
             \n    cargo install trunk      # or: brew install trunk\n\n\
             …or just run `cohors web` (it installs Trunk for you unless you pass --no-install)."
        );
    }
    if cargo_available() {
        eprintln!(
            "Trunk (the WASM bundler the web app needs) isn't installed — installing it with \
             `cargo install trunk` (one-time, a few minutes)…"
        );
        let status = std::process::Command::new("cargo")
            .args(["install", "trunk"])
            .status()
            .context("running `cargo install trunk`")?;
        if !status.success() {
            bail!(
                "`cargo install trunk` failed — install it manually (`brew install trunk`), then retry"
            );
        }
        Ok(())
    } else {
        bail!(
            "the web app needs Trunk, and Cargo isn't available to install it automatically.\n\
             Install Trunk with `brew install trunk` (or see https://trunkrs.dev), then re-run `cohors web`."
        )
    }
}

/// The first free loopback port at or after `preferred` (scanning a small range),
/// so a busy port doesn't sink `cohors web`. Falls back to `preferred` if none
/// in range bind (then the server reports the real error).
fn pick_port(preferred: u16) -> u16 {
    (preferred..preferred.saturating_add(20))
        .find(|&p| std::net::TcpListener::bind(("127.0.0.1", p)).is_ok())
        .unwrap_or(preferred)
}

/// Poll until `path` exists, or the timeout elapses (waiting for Trunk's first build).
fn wait_for_file(path: &Utf8Path, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    false
}

/// Walk up from the current directory to find the `cohors-web` crate.
fn find_web_crate() -> Option<Utf8PathBuf> {
    let cwd = Utf8PathBuf::from_path_buf(std::env::current_dir().ok()?).ok()?;
    cwd.ancestors()
        .map(|d| d.join("crates/cohors-web"))
        .find(|d| d.join("Cargo.toml").exists())
}

/// Is the `trunk` CLI on PATH?
fn trunk_available() -> bool {
    std::process::Command::new("trunk")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Is `cargo` on PATH? (Used to decide whether we can auto-install Trunk, or must
/// point the user at a binary install instead — e.g. a non-Rust, prebuilt-binary
/// distribution.)
fn cargo_available() -> bool {
    std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Poll loopback `port` until it accepts a TCP connection (the server is up), or
/// the timeout elapses. Used to open the browser only once the page will load.
fn wait_for_port(port: u16, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    false
}

/// Open a URL in the user's default browser (best-effort, non-blocking).
fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url).spawn().map(|_| ())
}

/// Bare `cohors` — launch the interactive dashboard.
pub fn run_tui(cli: &Cli) -> Result<()> {
    let scanner = Arc::new(crate::scan::from_cli(cli)?);

    // The launch screen lives in the plain terminal, not the alt-screen. `--root`
    // gets the scoped-view greeting; a bare first run (no `--root`, no config)
    // gets the super-fleet one; a user who's already configured a config file goes
    // straight to the dashboard (no pre-scan cost, no repeat greeting). `--watch`
    // opts out of the greeting entirely — see [`onboard`].
    let scanner = if !cli.root.is_empty() {
        match scoped_onboard(&scanner, cli.watch)? {
            Onboard::Proceed => scanner,
            Onboard::Quit => return Ok(()),
        }
    } else if config_path(cli).map(|p| p.exists()).unwrap_or(false) {
        scanner
    } else {
        match onboard(&scanner, cli.watch)? {
            Onboard::Proceed => scanner,
            Onboard::Quit => return Ok(()),
        }
    };

    crate::tui::run(scanner, cli, !cli.no_cache, cli.watch)
}

/// `cohors demo` — launch the dashboard on a built-in sample fleet. No config,
/// no scanning, no disk access; a zero-setup way to try the tool.
pub fn run_demo() -> Result<()> {
    crate::tui::run_demo()
}

/// What `cohors mcp` prints when a human runs it at a terminal: what the server
/// is, why nothing appeared to happen, and the copy-pasteable line that registers
/// it. The absolute path matters — agents spawn with a stripped `PATH`, so a bare
/// `cohors mcp` often fails to resolve and the agent silently ends up with no
/// tools at all.
fn mcp_guide(exe: &str, caps: cohors_mcp::Caps, roots: &[String], color: bool) -> String {
    let (writes, run, open, launchpad) = (
        caps.allow_writes,
        caps.allow_run,
        caps.allow_open,
        caps.allow_launchpad,
    );
    let mut out = masthead(
        "mcp: your fleet, for a coding agent",
        "it speaks JSON-RPC over stdio, so a client spawns it, not you",
        color,
    );

    out.push_str(&section(
        "nothing is broken",
        "run by hand it just waits on a client that is never going to speak",
        color,
    ));
    out.push('\n');

    out.push_str(&section(
        "wire it up",
        "use the absolute path: agents spawn with a stripped PATH, so a bare `cohors` may not resolve",
        color,
    ));
    // Claude Code has a registration CLI; every other stdio client (Claude
    // Desktop, Cursor, Zed, Windsurf, …) takes the same command and args as a
    // JSON block instead, so offer both rather than assuming one client.
    // The registration carries the flags this was run with, so pasting it
    // gives the client the server that was just described.
    let mut args: Vec<String> = vec!["mcp".to_string()];
    for (on, flag) in [
        (caps.allow_writes, "--allow-writes"),
        (caps.allow_run, "--allow-run"),
        (caps.allow_open, "--allow-open"),
        (caps.allow_launchpad, "--allow-launchpad"),
    ] {
        if on {
            args.push(flag.to_string());
        }
    }
    for root in roots {
        args.push("--root".to_string());
        args.push(root.clone());
    }
    let shell_args = args
        .iter()
        .map(|a| {
            if a.chars()
                .any(|c| c.is_whitespace() || "'\"$`\\".contains(c))
            {
                format!("'{}'", a.replace('\'', "'\\''"))
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let json_args = args
        .iter()
        .map(|a| serde_json::to_string(a).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(", ");
    let ways = [
        (
            format!("claude mcp add cohors -- {exe} {shell_args}"),
            "Claude Code, in your shell",
        ),
        (
            format!("\"cohors\": {{ \"command\": \"{exe}\", \"args\": [{json_args}] }}"),
            "any other client, in mcpServers",
        ),
    ];
    let w = ways
        .iter()
        .map(|(c, _)| c.chars().count())
        .max()
        .unwrap_or(0);
    for (cmd, why) in &ways {
        out.push_str(&guide_row(cmd, why, w, color));
    }
    out.push('\n');

    let state = |on: bool| if on { "on now" } else { "off, opt-in" };
    let rows: Vec<(String, String)> = vec![
        (
            "reads".to_string(),
            "overview, list_repos, get_repo, fleet_summary, standup, search, changes, repo_path, list_prs, ci_status"
                .to_string(),
        ),
        (
            "prompts".to_string(),
            "setup-repo, declare-flows, add-agent, track-tasks, plan-day".to_string(),
        ),
        (
            "launchpad reads".to_string(),
            "launchpad, read_contract, read_board".to_string(),
        ),
        (
            "--allow-launchpad".to_string(),
            format!(
                "write_repo_contribution, write_board, record_activity ({})",
                state(launchpad)
            ),
        ),
        (
            "--allow-writes".to_string(),
            format!("fetch, pull, push, commit, stash ({})", state(writes)),
        ),
        (
            "--allow-run".to_string(),
            format!("run shell commands across repos ({})", state(run)),
        ),
        (
            "--allow-open".to_string(),
            format!("the local-only open tool ({})", state(open)),
        ),
    ];
    let w = rows
        .iter()
        .map(|(c, _)| c.chars().count())
        .max()
        .unwrap_or(0);
    out.push_str(&section(
        "what an agent gets",
        "reads and prompts are always on, anything that writes is opt-in",
        color,
    ));
    for (what, why) in &rows {
        out.push_str(&guide_row(what, why, w, color));
    }
    out.push('\n');
    out
}

/// `cohors mcp` — speak the Model Context Protocol over stdio. Read-only unless
/// the matching `--allow-*` flags are passed.
pub fn run_mcp(cli: &Cli, caps: cohors_mcp::Caps) -> Result<()> {
    // An MCP client always pipes stdin. A terminal on stdin therefore means a
    // human ran this by hand, and would otherwise stare at a blank line forever
    // waiting for a client that is never going to speak. Explain and leave.
    if std::io::stdin().is_terminal() {
        let color = wants_color(std::io::stdout().is_terminal());
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(str::to_string))
            .unwrap_or_else(|| "cohors".to_string());
        print!("{}", mcp_guide(&exe, caps, &cli.root, color));
        return Ok(());
    }

    let scanner = crate::scan::from_cli(cli)?;
    let roots = scanner.roots();
    let config_path = scanner.config_path();
    let token = scanner.github_token();
    let actions_config = scanner.actions_config();
    let scan = || scanner.scan();
    // The server only reports a newer cohors; installing is left to the
    // dashboard or `cohors update`. It does the once-a-day lookup in the
    // background, so an agent-only user hears about new releases too.
    let mode = crate::update::effective_mode(scanner.updates_mode());
    if mode != cohors_config::UpdatesMode::Off {
        std::thread::spawn(move || crate::update::newer_release(false, now_secs()));
    }
    let update = move || crate::update::remembered(mode);
    // Started by the Claude Code plugin, the server lives exactly as long as
    // the session, so it notes the session's start and end on the launchpad's
    // activity log itself (the plugin ships no hooks). Best-effort: a failure
    // is logged and never stops the server.
    let note = |phase: &str| {
        if caps.plugin
            && let Err(err) = launchpad::record(phase, "claude-code", None, None)
        {
            tracing::warn!("recording the session {phase} failed: {err:#}");
        }
    };
    note("start");
    let served = cohors_mcp::serve_stdio(
        &scan,
        token.as_deref(),
        &roots,
        &config_path,
        caps,
        &actions_config.run_allowlist,
        actions_config.max_action_targets,
        scanner.author_email().as_deref(),
        &update,
    );
    note("end");
    served
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_passthrough() {
        let sel = parse_selector(r#"{"behind": true, "name": "api"}"#).unwrap();
        assert!(sel.behind);
        assert_eq!(sel.name.as_deref(), Some("api"));
    }

    #[test]
    fn unknown_token_errors() {
        assert!(parse_selector("frobnicate").is_err());
        assert!(parse_selector("ci:sideways").is_err());
    }

    #[test]
    fn web_setup_hint_guides_to_root_and_super_fleet() {
        // Plain (color = false) so we assert on content, not escapes.
        let hint = web_setup_hint(&["/home/me/projects".to_string()], false);
        assert!(hint.contains("No git repositories found"));
        assert!(hint.contains("looked under /home/me/projects")); // where it looked
        assert!(hint.contains("cohors web --root ~/code")); // the sample command
        assert!(hint.contains("cohors init") && hint.contains("cohors demo"));
        assert!(hint.contains("super fleet")); // the model note
        assert!(!hint.contains('\x1b')); // no ANSI when color is off
    }

    #[test]
    fn tui_welcome_guides_to_root_and_super_fleet() {
        let w = tui_welcome(&["/home/me/projects".to_string()], false);
        assert!(w.contains("No git repositories found"));
        assert!(w.contains("looked under /home/me/projects"));
        assert!(!w.contains("cohors web")); // the bare-cohors variant, not web
        assert!(w.contains("cohors --root ~/code"));
        assert!(w.contains("super fleet"));
        assert!(!w.contains('\x1b'));
    }

    #[test]
    fn hint_ansi_is_gated_on_color() {
        assert!(web_setup_hint(&["p".to_string()], true).contains('\x1b')); // styled…
        assert!(!web_setup_hint(&["p".to_string()], false).contains('\x1b')); // …only when asked
    }

    /// The launch screen reports what the scan found and the first things
    /// worth doing, in the dashboard's own words, before any key is pressed.
    /// The start screen in every look (ADR-060), Unicode and ASCII, colour off
    /// so the snapshot shows the structure each look draws with characters.
    #[test]
    fn snapshot_launch_screen_in_every_look() {
        let now = 1_700_000_000;
        let fleet = cohors_core::demo::fleet(now);
        let (entries, board) = cohors_core::launchpad::demo();
        let o = cohors_core::overview(&fleet, &entries, &board, now);
        let roots = ["~/Developer".to_string()];
        for style in look::Style::ALL {
            for ascii in [false, true] {
                let current = look::Look {
                    style,
                    ascii,
                    width: 80,
                };
                let screen = look::with(current, || launch_screen(&o, &roots, false, false));
                let glyphs = if ascii { "ascii" } else { "unicode" };
                insta::assert_snapshot!(format!("launch_screen_{}_{glyphs}", style.name()), screen);
            }
        }
    }

    /// The first-run start screen lists the folders it gathered, in a list
    /// the look draws (a bullet per folder), in the default look.
    #[test]
    fn snapshot_first_run_launch_screen() {
        let now = 1_700_000_000;
        let fleet = cohors_core::demo::fleet(now);
        let o = cohors_core::overview(&fleet, &[], &Default::default(), now);
        let roots = ["~/code".to_string(), "~/work".to_string()];
        let current = look::Look {
            style: look::DEFAULT_STYLE,
            ascii: false,
            width: 80,
        };
        let screen = look::with(current, || launch_screen(&o, &roots, true, false));
        insta::assert_snapshot!("launch_screen_first_run", screen);
    }

    #[test]
    fn launch_screen_shows_the_headline_next_steps_and_keys() {
        let now = 1_700_000_000;
        let fleet = cohors_core::demo::fleet(now);
        let (entries, board) = cohors_core::launchpad::demo();
        let o = cohors_core::overview(&fleet, &entries, &board, now);
        let roots = ["~/Main/code".to_string()];

        let w = launch_screen(&o, &roots, false, false);
        assert!(w.contains(&o.headline)); // real counts, not just a folder name
        assert!(w.contains("~/Main/code")); // where it looked
        assert!(w.contains("Do next"));
        // The first steps are listed, most urgent first, with a shape mark…
        for action in o.actions.iter().take(LAUNCH_STEPS) {
            assert!(w.contains(&action.title), "{}", action.title);
        }
        assert!(w.find(&o.actions[0].title) < w.find(&o.actions[1].title));
        assert!(w.contains(" RISK ")); // the severity block, as in the dashboard
        // …and the rest are counted, not dumped.
        let more = o.actions.len() - LAUNCH_STEPS;
        assert!(w.contains(&format!("and {more} more in the dashboard")));
        assert!(w.contains("enter  open the dashboard") && w.contains("q  quit"));
        assert!(!w.contains("Looking in")); // only the first run lists folders
        assert!(!w.contains('\x1b'));
        assert!(launch_screen(&o, &roots, false, true).contains('\x1b'));
    }

    /// A first run (no config) also says which folders were gathered and how
    /// to look in just one of them.
    #[test]
    fn first_run_launch_screen_lists_the_folders_it_gathered() {
        let now = 1_700_000_000;
        let o = cohors_core::overview(
            &cohors_core::demo::fleet(now),
            &[],
            &Default::default(),
            now,
        );
        let roots = ["~/projects".to_string(), "~/code".to_string()];
        let w = launch_screen(&o, &roots, true, false);
        assert!(w.contains("gathered from 2 folders"));
        assert!(w.contains("Looking in"));
        assert!(w.contains("~/projects") && w.contains("~/code")); // each folder listed
        assert!(w.contains("cohors --root <dir>")); // the scoped alternative
        assert!(w.contains("enter  open the dashboard"));
    }

    /// A fleet with nothing to do still gets a launch screen that says so.
    #[test]
    fn launch_screen_with_nothing_to_do_says_so() {
        let o = cohors_core::overview(&[], &[], &Default::default(), 0);
        let w = launch_screen(&o, &["~/code".to_string()], false, false);
        assert!(w.contains("nothing needs you right now"));
        assert!(w.contains("enter  open the dashboard"));
    }

    /// The report groups by urgency: anything wanting attention is listed by
    /// name, calm repos collapse to a count, and actionable counts become the
    /// command that clears them.
    #[test]
    fn scan_report_sections_by_urgency_and_suggests_next_steps() {
        use cohors_core::Severity as S;
        let now = 1_700_000_000;
        let took = std::time::Duration::from_millis(840);
        let snaps = cohors_core::demo::fleet(now);
        let roots = ["/home/me/projects".to_string()];
        let r = scan_report(&snaps, &roots, took, now, 140, false, None);

        assert!(r.contains("cohors")); // branded masthead
        // The report says what it actually did, not just what it found.
        assert!(r.contains(&format!("scanned {} repositories in 840ms", snaps.len())));
        let mut calm = 0;
        for s in &snaps {
            let a = cohors_core::assess(s, now);
            if a.severity == S::Ok {
                calm += 1;
            } else {
                assert!(
                    r.contains(&s.name),
                    "{} wants attention, should be listed",
                    s.name
                );
            }
        }
        // Calm repos collapse instead of repeating "up to date" a dozen times.
        assert_eq!(r.contains("up to date, nothing to do"), calm > 0);

        // Each "Do next" step from the shared model appears as the command
        // that starts it, and only the steps this fleet actually has.
        let overview = cohors_core::overview(&snaps, &[], &Default::default(), now);
        assert!(!overview.actions.is_empty());
        for action in &overview.actions {
            assert!(r.contains(action_command(action)), "{}", action.title);
            assert!(r.contains(&action.title));
        }
        use cohors_core::ActionKind as K;
        let has = |kind: K| overview.actions.iter().any(|a| a.kind == kind);
        assert_eq!(
            r.contains("cohors pull --select 'behind,!diverged'"),
            has(K::Pull)
        );
        assert_eq!(
            r.contains("cohors push --select 'ahead,!diverged'"),
            has(K::Push)
        );

        // The command a step suggests selects exactly the repos the step
        // counted: no more (a repo with no remote must not be "published"),
        // no fewer.
        for action in &overview.actions {
            let Some(query) = action_command(action).split("--select ").nth(1) else {
                continue; // a step with no selector, e.g. `cohors launchpad`
            };
            if action.verb.is_none() {
                continue; // a listing to look at, not an exact target set
            }
            let selector = parse_selector(query.trim_matches('\'')).unwrap();
            let mut picked: Vec<String> =
                cohors_core::resolve(&snaps, &selector, SortMode::Name, now)
                    .into_iter()
                    .map(|id| id.0)
                    .collect();
            let mut wanted = action.repos.clone();
            picked.sort();
            wanted.sort();
            assert_eq!(picked, wanted, "`{query}` vs the step \"{}\"", action.title);
        }
        // The sync column says what is true in words (the old report printed
        // "local" for anything without an upstream).
        for line in overview.repos.iter().filter(|l| !l.reasons.is_empty()) {
            let words = sync_words(line);
            assert!(words.is_empty() || r.contains(&words), "{words}");
        }
        assert!(r.contains("level") || r.contains("ahead") || r.contains("behind"));
        assert!(r.contains("Work at risk") && r.contains("Do next"));
        // A repo with several reasons lists all of them.
        let busy = overview.repos.iter().find(|l| l.reasons.len() > 1).unwrap();
        assert!(r.contains(&busy.reasons.join("; ")));

        // No line is ever wider than the terminal, at any width: a wrapped
        // table cannot be read. Narrow terminals drop the branch column.
        for width in [60, 80, 100, 110, 140, 200] {
            let report = scan_report(&snaps, &roots, took, now, width, false, None);
            for line in report.lines() {
                assert!(
                    line.chars().count() <= width,
                    "{} columns at width {width}: {line}",
                    line.chars().count()
                );
            }
        }
        let branch = &overview
            .repos
            .iter()
            .find(|l| l.branch.contains('/'))
            .unwrap()
            .branch;
        assert!(scan_report(&snaps, &roots, took, now, 200, false, None).contains(branch.as_str()));
        assert!(!scan_report(&snaps, &roots, took, now, 80, false, None).contains(branch.as_str()));
        assert!(!r.contains('\x1b')); // no ANSI when colour is off
        assert!(scan_report(&snaps, &roots, took, now, 140, true, None).contains('\x1b')); // …and some when it is
    }

    /// A missing selector teaches instead of erroring: what the verb does, the
    /// whole selector vocabulary wired to that verb, and a safe place to start.
    #[test]
    fn action_guide_explains_the_verb_and_how_to_select() {
        let g = action_guide(
            "fetch",
            "",
            "git fetch across the repos you pick",
            "safe anywhere: it never touches your working tree",
            false,
        );
        assert!(g.contains("cohors")); // masthead
        assert!(g.contains("fetch: git fetch across the repos you pick"));
        assert!(g.contains("Pick what to act on")); // sentence-case headings
        assert!(g.contains("never touches your working tree")); // the safety note
        for (sel, _) in SELECTORS {
            assert!(
                g.contains(&format!("cohors fetch --select {sel}")),
                "selector `{sel}` should be offered for this verb"
            );
        }
        assert!(g.contains("cohors scan")); // the read-only starting point
        assert!(g.contains("--dry-run")); // look before you leap
        assert!(!g.contains('\x1b'));
    }

    /// Verbs that need more than a selector carry that argument through every
    /// worked example, so `commit` and `run` show a usable command, not a stub.
    #[test]
    fn action_guide_carries_the_extra_argument_into_examples() {
        let commit = action_guide("commit", "-m \"your message\"", "does", "safe", false);
        assert!(commit.contains("cohors commit -m \"your message\" --select dirty"));
        assert!(commit.contains("cohors commit -m \"your message\" --select dirty --dry-run"));

        let run = action_guide("run", "\"your command\"", "does", "safe", false);
        assert!(run.contains("cohors run \"your command\" --select all"));
    }

    /// The overview report leads with what to do, names the command that does
    /// it, and stays free of ANSI when colour is off.
    #[test]
    fn overview_report_leads_with_actions_and_names_the_command() {
        let now = 1_700_000_000;
        let fleet = cohors_core::demo::fleet(now);
        let (entries, board) = cohors_core::launchpad::demo();
        let o = cohors_core::overview(&fleet, &entries, &board, now);
        let plain = overview_report(&o, &["/code".to_string()], now, 120, false);
        assert!(!plain.contains('\x1b'), "no ANSI without colour");
        let do_next = plain.find("Do next").expect("a do-next section");
        let glance = plain.find("At a glance").expect("an at-a-glance section");
        assert!(do_next < glance, "actions come before figures");
        assert!(plain.contains(&o.headline));
        assert!(plain.contains(&o.actions[0].title));
        assert!(plain.contains("not backed up") && plain.contains("commits, 7 days"));
        // Every step shows the command that starts it.
        for action in &o.actions {
            assert!(plain.contains(action_command(action)), "{}", action.title);
        }
        // It fits the terminal at any width: long reasons wrap, nothing spills.
        for width in [60, 80, 100, 140] {
            for line in overview_report(&o, &["/code".to_string()], now, width, false).lines() {
                assert!(
                    line.chars().count() <= width,
                    "{} columns at width {width}: {line}",
                    line.chars().count()
                );
            }
        }
        assert_eq!(wrap("one two three four", 9), ["one two", "three", "four"]);
        assert_eq!(wrap("", 9), Vec::<String>::new());
        assert!(overview_report(&o, &[], now, 120, true).contains('\x1b'));
    }

    #[test]
    fn overview_report_handles_an_empty_fleet() {
        let o = cohors_core::overview(&[], &[], &Default::default(), 0);
        let plain = overview_report(&o, &["/code".to_string()], 0, 120, false);
        assert!(plain.contains("No repositories found."));
        assert!(plain.contains("cohors init"));
    }

    #[test]
    fn scan_report_handles_an_empty_fleet() {
        let took = std::time::Duration::from_millis(3);
        let r = scan_report(
            &[],
            &["/home/me/projects".to_string()],
            took,
            0,
            140,
            false,
            None,
        );
        assert!(r.contains("scanned 0 repositories in 3ms"));
        assert!(r.contains("nothing to report"));
        assert!(!r.contains("Do next")); // no sections at all for an empty fleet
        assert!(!r.contains('\x1b'));
    }

    /// `cohors discover` (ADR-058): repos listed under their root, the root
    /// itself when it is a repo, nested and deep-only ones marked, plain text
    /// with color off.
    #[test]
    fn discover_report_lists_repos_by_root_and_marks_nested_ones() {
        let p = |s: &str| Utf8PathBuf::from(s);
        let found = cohors_fleet::Discovery {
            repos: vec![
                p("/home/me/code"),
                p("/home/me/code/archive/old-site"),
                p("/home/me/code/archive/old-site/tools/linter"),
                p("/home/me/code/projects/api-dashboard"),
                p("/home/me/oss/rg"),
            ],
            deep_only: vec![p("/home/me/code/archive/old-site/tools/linter")],
        };
        let roots = [p("/home/me/code"), p("/home/me/oss")];
        let took = std::time::Duration::from_millis(4);
        let r = discover_report(&found, &roots, took, false, false);
        assert!(!r.contains('\x1b'), "no ANSI with color off");
        assert!(r.contains("found 5 repositories in 4ms"), "{r}");
        assert!(r.contains("4 repositories"), "{r}");
        assert!(
            r.contains(". ") && r.contains("this folder is a repo"),
            "{r}"
        );
        assert!(
            r.contains("projects/api-dashboard") && r.contains("inside code"),
            "{r}"
        );
        assert!(r.contains("deep  inside old-site"), "{r}");
        assert!(r.contains("Includes 1 repository remembered"), "{r}");

        let deep = discover_report(&found, &roots, took, true, false);
        assert!(deep.contains(", searching deep"), "{deep}");
        assert!(
            deep.contains("the dashboard shows it from now on"),
            "{deep}"
        );
    }

    /// A folder name is repository-controlled: an escape sequence in it must
    /// not reach the terminal through the human list.
    #[test]
    fn discover_report_strips_control_characters_from_paths() {
        let p = |s: &str| Utf8PathBuf::from(s);
        let found = cohors_fleet::Discovery {
            repos: vec![p("/c"), p("/c/evil\x1b[2J\x07name")],
            deep_only: vec![],
        };
        let took = std::time::Duration::from_millis(1);
        let r = discover_report(&found, &[p("/c")], took, false, false);
        assert!(!r.contains('\x1b') && !r.contains('\x07'), "{r:?}");
        assert!(r.contains("evil[2Jname"), "{r}");
    }

    /// Under --select the masthead counts both what was scanned and what
    /// matched, and a clean match is listed by name rather than as a count.
    #[test]
    fn scan_report_under_select_counts_both_and_lists_clean_matches() {
        let now = 1_700_000_000;
        let took = std::time::Duration::from_millis(5);
        let fleet = cohors_core::demo::fleet(now);
        let clean: Vec<_> = fleet
            .iter()
            .filter(|s| !s.is_dirty() && s.error.is_none() && s.ahead() == 0 && s.behind() == 0)
            .take(1)
            .cloned()
            .collect();
        assert_eq!(clean.len(), 1, "the demo fleet has a calm repo");
        let roots = ["/home/me/code".to_string()];
        let r = scan_report(&clean, &roots, took, now, 140, false, Some(("name:x", 24)));
        assert!(
            r.contains("scanned 24 repositories in 5ms, 1 matches --select name:x"),
            "{r}"
        );
        assert!(r.contains(&clean[0].name), "the match is named: {r}");

        let none = scan_report(&[], &roots, took, now, 140, false, Some(("conflicted", 24)));
        assert!(none.contains("0 match --select conflicted") && none.contains("nothing matched"));
    }

    /// The registration lines carry the flags `cohors mcp` was run with, so
    /// pasting them registers the server that the guide just described.
    #[test]
    fn mcp_guide_registers_the_flags_it_was_given() {
        let caps = cohors_mcp::Caps {
            allow_writes: true,
            ..Default::default()
        };
        let g = mcp_guide("/bin/cohors", caps, &["~/my code".to_string()], false);
        assert!(
            g.contains(
                "claude mcp add cohors -- /bin/cohors mcp --allow-writes --root '~/my code'"
            ),
            "{g}"
        );
        assert!(
            g.contains(r#""args": ["mcp", "--allow-writes", "--root", "~/my code"]"#),
            "{g}"
        );
    }

    #[test]
    fn deep_note_suggests_the_deep_search_when_nothing_is_remembered() {
        let n = deep_note(false, 0, "scan", false);
        assert!(n.contains("cohors scan --deep"));
        assert!(deep_note(true, 0, "scan", false).contains("nothing a normal search misses"));
        assert!(deep_note(true, 3, "scan", false).contains("Remembered 3 repositories"));
    }

    #[test]
    fn nested_in_names_the_closest_enclosing_repo() {
        let p = |s: &str| Utf8PathBuf::from(s);
        let repos = [p("/c"), p("/c/a"), p("/c/a/b"), p("/c/ab")];
        assert_eq!(nested_in(&p("/c/a/b"), &repos), Some(&p("/c/a")));
        assert_eq!(
            nested_in(&p("/c/ab"), &repos),
            Some(&p("/c")),
            "a/ab is not nesting"
        );
        assert_eq!(nested_in(&p("/c"), &repos), None);
    }
}
