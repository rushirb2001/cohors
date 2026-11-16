//! Implementations of the CLI subcommands.

pub mod agent;
pub mod launchpad;
pub mod look;
pub mod next;
pub mod reads;
pub mod watch;

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
    // Already set up: say where, and what it holds, rather than failing.
    if path.exists() && !force {
        let roots = cohors_config::Config::load(Some(&path))
            .map(|c| c.roots)
            .unwrap_or_default();
        println!("A config already exists at {path}");
        if !roots.is_empty() {
            println!("Roots: {}", roots.join(", "));
        }
        println!(
            "Edit it, or run `cohors init --force` to start over (the old one is kept as .bak)."
        );
        return Ok(());
    }
    // `--root` seeds the roots as given; otherwise detect where repos live.
    let roots = if cli.root.is_empty() {
        let home = cohors_config::paths::home_dir().ok();
        cohors_fleet::detect_roots(home.as_deref())
    } else {
        cli.root.clone()
    };
    if path.exists() {
        let backup = Utf8PathBuf::from(format!("{path}.bak"));
        std::fs::copy(&path, &backup)
            .with_context(|| format!("keeping the old config as {backup}"))?;
        println!("Kept the old config as {backup}");
    }
    cohors_config::write_starter(&path, true, &roots)
        .with_context(|| format!("writing config to {path}"))?;
    println!("Wrote starter config to {path}");
    if roots.is_empty() {
        println!("No repos auto-detected — edit `roots` to point at your code, then run `cohors`.");
    } else {
        println!("Roots: {}", roots.join(", "));
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
    let human = wants_report(json);
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
    let human = wants_report(json);
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
                    "nested_in": found.nested_in(path),
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
    if let Some(outer) = found.nested_in(path) {
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
        K::Worktrees => "cohors scan --select worktree-work",
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
    let _width = look::narrow_to(width);
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
pub fn overview(cli: &Cli, select: Option<&str>, json: bool) -> Result<()> {
    let human = wants_report(json);
    let color = wants_color(human);
    let scanner = crate::scan::from_cli(cli)?;
    let snapshots = selected(scanner.scan(), select)?;
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

/// Keep the snapshots `select` matches (all of them without one).
fn selected(
    snapshots: Vec<cohors_core::RepoSnapshot>,
    select: Option<&str>,
) -> Result<Vec<cohors_core::RepoSnapshot>> {
    let Some(query) = select else {
        return Ok(snapshots);
    };
    let keep: std::collections::HashSet<String> = action_targets(&snapshots, query)?
        .into_iter()
        .map(|s| s.id.0.clone())
        .collect();
    Ok(snapshots
        .into_iter()
        .filter(|s| keep.contains(&s.id.0))
        .collect())
}

/// `--since`: `yesterday`, `Nd` (N days back), or a date `YYYY-MM-DD` (UTC).
fn parse_since(text: &str, now: i64) -> Result<i64> {
    const DAY: i64 = 24 * 60 * 60;
    let text = text.trim().to_ascii_lowercase();
    let midnight = now - now.rem_euclid(DAY);
    if text == "yesterday" {
        return Ok(midnight - DAY);
    }
    if text == "today" {
        return Ok(midnight);
    }
    if let Some(days) = text.strip_suffix('d').and_then(|n| n.parse::<i64>().ok()) {
        return Ok(now - days.max(0) * DAY);
    }
    let parts: Vec<i64> = text.split('-').filter_map(|p| p.parse().ok()).collect();
    if let [y, m, d] = parts[..]
        && (1..=12).contains(&m)
        && (1..=31).contains(&d)
    {
        // Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        return Ok((era * 146_097 + doe - 719_468) * DAY);
    }
    bail!("--since takes `yesterday`, `3d` (days back) or a date like 2026-11-01, not `{text}`")
}

/// `cohors standup` — your commits across every repo, as a digest.
///
/// At a terminal it prints the markdown digest (the one the dashboard copies);
/// with `--json` it prints the commits grouped by repo. `today` narrows the
/// window from the last seven days to today.
pub fn standup(
    cli: &Cli,
    today: bool,
    since: Option<&str>,
    select: Option<&str>,
    json: bool,
) -> Result<()> {
    use cohors_core::StandupWindow;
    let scanner = crate::scan::from_cli(cli)?;
    let Some(author) = scanner.author_email() else {
        bail!(
            "no git identity: set `git config --global user.email` so cohors knows which commits are yours"
        );
    };
    let now = now_secs();
    let window = match since {
        Some(text) => StandupWindow::Custom {
            since: parse_since(text, now)?,
            until: now,
        },
        None if today => StandupWindow::Today,
        None => StandupWindow::Week,
    };
    let (since, until) = window.range(now);
    let fleet = selected(scanner.scan(), select)?;
    let commits = cohors_fleet::fleet_commits(&fleet, &author, since, until);
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
    let _width = look::narrow_to(width);
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
/// The groups `cohors --help` lists its commands in, each with what the
/// group is for. Every command is in one of them (a test makes sure), so a
/// new command cannot quietly go missing from the help.
pub const COMMAND_GROUPS: &[(&str, &str, &[&str])] = &[
    (
        "See what needs you",
        "reports and reads; nothing changes",
        &[
            "overview", "next", "show", "changes", "prs", "standup", "search", "path", "scan",
            "discover", "watch",
        ],
    ),
    (
        "Change your repositories",
        "each lists the repositories and asks before it acts",
        &["fetch", "pull", "push", "commit", "stash", "run"],
    ),
    (
        "Agents and the launchpad",
        "start a coding agent, or serve your fleet to one",
        &["agent", "launchpad", "mcp"],
    ),
    (
        "Other ways to look",
        "run `cohors` alone for the terminal dashboard",
        &["web", "demo"],
    ),
    (
        "Set up and keep cohors current",
        "config, install, updates, completions",
        &[
            "init",
            "install",
            "update",
            "doctor",
            "uninstall",
            "completions",
            "help",
        ],
    ),
];

/// The commands block of `cohors --help`: [`COMMAND_GROUPS`], each heading
/// with its line of purpose, then its commands and what each does (from the
/// command's own description, without the `(no TUI)` the group heading
/// already says), aliases kept.
pub fn grouped_commands(command: &clap::Command) -> String {
    let color = wants_color(std::io::stdout().is_terminal());
    let about = |name: &str| -> Option<String> {
        if name == "help" {
            return Some("Print this message or the help of the given command".into());
        }
        let sub = command.find_subcommand(name)?;
        let text = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
        let text = text.replace(" (no TUI)", "");
        let aliases: Vec<&str> = sub.get_visible_aliases().collect();
        Some(if aliases.is_empty() {
            text
        } else {
            format!("{text} [alias: {}]", aliases.join(", "))
        })
    };
    let width = COMMAND_GROUPS
        .iter()
        .flat_map(|(_, _, names)| names.iter())
        .map(|n| n.len())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for (i, (title, purpose, names)) in COMMAND_GROUPS.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "{}  {}\n",
            brand(color, &bold(color, title)),
            dim(color, purpose)
        ));
        for name in *names {
            if let Some(text) = about(name) {
                out.push_str(&format!(
                    "  {}  {text}\n",
                    bold(color, &format!("{name:<width$}"))
                ));
            }
        }
    }
    out
}

/// `help` with its flat `Commands:` block swapped for `grouped`. `None` when
/// it has no top-level command list (a subcommand's own help), which is
/// then printed as clap wrote it.
pub fn regroup_help(help: &str, grouped: &str) -> Option<String> {
    let plain = |line: &str| strip_ansi(line).trim_end().to_string();
    let lines: Vec<&str> = help.split_inclusive('\n').collect();
    let start = lines.iter().position(|l| plain(l) == "Commands:")?;
    // The top-level list, not a subcommand's (`cohors agent --help` has one).
    let block: Vec<String> = lines[start..].iter().map(|l| plain(l)).collect();
    let end = block
        .iter()
        .position(|l| l.is_empty())
        .unwrap_or(block.len());
    let names: Vec<&str> = block[1..end]
        .iter()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    if !(names.contains(&"overview") && names.contains(&"fetch")) {
        return None;
    }
    let mut out: String = lines[..start].concat();
    out.push_str(grouped);
    out.push_str(&lines[start + end..].concat());
    Some(out)
}

/// `text` without its ANSI colour codes.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            // Skip to the end of the escape sequence (a letter).
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

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
            "cohors agent setup-repo",
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
    // Scripts and agents branch on these; every command keeps to them.
    after.push_str(&section("exit codes", "what a script can branch on", color));
    for (code, means) in [
        ("0", "done"),
        ("1", "something failed, or nothing matched a name"),
        (
            "2",
            "usage: a missing argument, or a name that fits several repos",
        ),
        ("130", "interrupted (Ctrl-C)"),
    ] {
        after.push_str(&guide_row(code, means, 3, color));
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
pub fn run_action(
    cli: &Cli,
    action: CliAction,
    select: Option<&str>,
    repos: &[String],
    dry_run: bool,
    json: bool,
    yes: bool,
) -> Result<()> {
    let human = !json;
    let color = human && wants_color(std::io::stdout().is_terminal());
    let verb = action.verb();
    let (does, safety) = action.blurb();
    // What picked the repos, as the person typed it (for messages).
    let picked = match select {
        Some(s) => format!("--select {s}"),
        None => repos.join(" "),
    };

    // A selector but no usable message is a mistake, not a question: name it.
    if (select.is_some() || !repos.is_empty())
        && let CliAction::Commit(message, _) = &action
        && message.as_deref().is_none_or(|m| m.trim().is_empty())
    {
        eprintln!("cohors: commit needs a message: cohors commit -m \"your message\" {picked}");
        return Err(ExitCode(2).into());
    }
    // No repos named at all is a question, not an error: say what this verb
    // does and how to choose repos, rather than a bare "required argument".
    if (select.is_none() && repos.is_empty()) || action.missing_arg() {
        print!(
            "{}",
            action_guide(verb, action.arg_hint(), does, safety, color)
        );
        return Ok(());
    }

    let scanner = crate::scan::from_cli(cli)?;
    let snapshots = scanner.scan();
    let targets: Vec<&cohors_core::RepoSnapshot> = if repos.is_empty() {
        action_targets(&snapshots, select.unwrap_or_default())?
    } else {
        // Repos named directly: each must name exactly one (a typo or an
        // ambiguous name stops here, with close names or the candidates).
        let roots = scanner.roots();
        let mut picked: Vec<&cohors_core::RepoSnapshot> = Vec::new();
        for key in repos {
            let found = reads::resolve_or_explain(&snapshots, key, &roots)?;
            if !picked.iter().any(|p| p.id == found.id) {
                picked.push(found);
            }
        }
        picked
    };
    if human {
        print!("{}", masthead(&format!("{verb}: {does}"), safety, color));
    }

    if targets.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::json!({ "verb": verb, "dry_run": dry_run, "results": [], "ok": 0, "failed": 0, "skipped": 0 })
            );
        } else {
            out_nothing_matched(verb, select.unwrap_or_default(), color);
        }
        return Ok(());
    }
    let n = targets.len();
    let plural = if n == 1 { "" } else { "s" };
    if dry_run {
        if json {
            let rows: Vec<serde_json::Value> = targets
                .iter()
                .map(|s| {
                    let mut row =
                        serde_json::json!({ "repo": s.name, "id": s.id.0, "path": s.path });
                    if let (CliAction::Commit(_, opts), Some(path)) = (&action, &s.path) {
                        let files = cohors_actions::paths_to_commit(path, opts.staged_only)
                            .unwrap_or_default();
                        row["secrets"] = serde_json::json!(cohors_actions::secret_paths(&files));
                        row["files"] = serde_json::json!(files);
                    }
                    row
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "verb": verb, "dry_run": true, "targets": rows })
                )?
            );
            return Ok(());
        }
        print!(
            "{}",
            section(
                "dry run",
                &format!("would {verb} {n} repo{plural}, nothing will change"),
                color
            )
        );
        print_targets(&targets, Some(&action), color);
        println!();
        return Ok(());
    }

    // At a terminal, nothing changes until the person has seen what will and
    // said yes (ADR-069). `--yes`, and scripts, go straight on.
    if should_ask(dry_run, json, yes) {
        print!(
            "{}",
            section(&format!("will {verb}"), &format!("{n} repo{plural}"), color)
        );
        print_targets(&targets, Some(&action), color);
        if !confirm(
            &format!(
                "{} {n} repositor{}?",
                capitalize(verb),
                if n == 1 { "y" } else { "ies" }
            ),
            color,
        ) {
            println!("  {}\n", dim(color, "Nothing done."));
            return Ok(());
        }
        println!();
    }

    // Pull/stash/commit mutate the worktree: a conflicted or mid-operation
    // repo is reported and skipped (same `worktree_block` rule every surface
    // applies); fetch/push never touch the worktree.
    let guard_worktree = matches!(
        action,
        CliAction::Pull | CliAction::Stash | CliAction::Commit(..)
    );
    if human {
        print!(
            "{}",
            section(action.doing(), &format!("{n} repo{plural}"), color)
        );
    }
    // One record per repo, for the summary and for --json.
    let mut results: Vec<serde_json::Value> = Vec::new();
    let (mut ok, mut skipped) = (0usize, 0usize);
    // Skips first (they need no git), then every repo at once on the shared
    // bounded pool the MCP server and the web use (ADR-050), each line printed
    // the moment its repo finishes, so a big fleet shows progress, not a pause.
    let (tx, rx) = std::sync::mpsc::channel::<(String, String, Result<String, String>)>();
    let mut started = 0usize;
    for s in &targets {
        if guard_worktree && let Some(reason) = cohors_actions::worktree_block(s) {
            skipped += 1;
            if human {
                println!("    {} {}: {reason}", dim(color, "·"), dim(color, &s.name));
            }
            results.push(serde_json::json!({ "repo": s.name, "id": s.id.0, "ok": false, "skipped": true, "message": reason }));
            continue;
        }
        let path = s.path.clone().unwrap_or_default();
        let (name, id) = (s.name.clone(), s.id.0.clone());
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
            let _ = tx.send((name, id, result));
        });
    }
    drop(tx);
    let mut refused_secret = false;
    for (name, id, result) in rx.iter().take(started) {
        let (good, message) = match result {
            Ok(m) => (true, m),
            Err(m) => (false, m),
        };
        if good {
            ok += 1;
        } else {
            refused_secret |= message.contains("looks like a secret");
        }
        if human {
            let mark = if good {
                sgr(color, "32", "✓")
            } else {
                sgr(color, "31", "✗")
            };
            println!("    {mark} {message}");
            let _ = std::io::stdout().flush();
        }
        results.push(serde_json::json!({ "repo": name, "id": id, "ok": good, "skipped": false, "message": message }));
    }
    let failed = n - ok - skipped;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "verb": verb, "dry_run": false, "results": results,
                "ok": ok, "failed": failed, "skipped": skipped,
            }))?
        );
    } else {
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
    }
    // Scripts and CI read the status: a repo that failed fails the command.
    // Skipped repos (conflicted, mid-operation) were left alone on purpose.
    if failed > 0 {
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
/// The command `run` was given, as one shell line: a single argument is
/// already a shell string (`cohors run "npm test"`); several (the words after
/// `--`) are quoted where a word needs it, so `-- git commit -m "two words"`
/// reaches the shell intact. `None` when no command was given.
pub fn shell_line(words: &[String]) -> Option<String> {
    match words {
        [] => None,
        [one] => Some(one.clone()),
        many => Some(
            many.iter()
                .map(|w| {
                    let plain = !w.is_empty()
                        && w.chars()
                            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,@%+".contains(c));
                    if plain {
                        w.clone()
                    } else {
                        format!("'{}'", w.replace('\'', "'\\''"))
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
        ),
    }
}

pub fn run_command_action(
    cli: &Cli,
    select: Option<&str>,
    command: Option<&str>,
    timeout: u64,
    dry_run: bool,
    json: bool,
    yes: bool,
) -> Result<()> {
    let human = !json;
    let color = human && wants_color(std::io::stdout().is_terminal());
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
    if human {
        print!("{}", masthead(&format!("run: {does}"), safety, color));
    }

    if targets.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::json!({ "command": command, "results": [], "ok": 0, "failed": 0 })
            );
        } else {
            out_nothing_matched("run", select, color);
        }
        return Ok(());
    }
    let n = targets.len();
    let plural = if n == 1 { "" } else { "s" };
    if dry_run && json {
        let rows: Vec<serde_json::Value> = targets
            .iter()
            .map(|s| serde_json::json!({ "repo": s.name, "id": s.id.0, "path": s.path }))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({ "command": command, "dry_run": true, "targets": rows })
            )?
        );
        return Ok(());
    }
    if dry_run {
        print!(
            "{}",
            section(
                "dry run",
                &format!("would run `{command}` in {n} repo{plural}, nothing will change"),
                color
            )
        );
        print_targets(&targets, None, color);
        println!();
        return Ok(());
    }

    // As for the git verbs: at a terminal, see where it will run, then say yes.
    if should_ask(dry_run, json, yes) {
        print!(
            "{}",
            section(
                "will run",
                &format!("`{command}` in {n} repo{plural}"),
                color
            )
        );
        print_targets(&targets, None, color);
        if !confirm(
            &format!(
                "Run `{command}` in {n} repositor{}?",
                if n == 1 { "y" } else { "ies" }
            ),
            color,
        ) {
            println!("  {}\n", dim(color, "Nothing done."));
            return Ok(());
        }
        println!();
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
    let ok = results.iter().filter(|r| r.ok).count();
    if json {
        let rows: Vec<serde_json::Value> = targets
            .iter()
            .zip(&results)
            .map(|(s, r)| {
                let mut row = serde_json::to_value(r).unwrap_or_default();
                row["name"] = serde_json::json!(s.name);
                row
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "command": command, "results": rows, "ok": ok, "failed": n - ok,
            }))?
        );
        if ok < n {
            return Err(ExitCode(1).into());
        }
        return Ok(());
    }
    for (s, r) in targets.iter().zip(&results) {
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
    // A newer cohors, for the page's header. Like the MCP server, the web
    // server looks in the background, now and every few minutes, and only
    // reports: installing is left to the dashboard or `cohors update`. The
    // demo shows sample data and says nothing about this machine's install.
    let update = (!demo).then(|| {
        let mode = crate::update::effective_mode(scanner.updates_mode());
        crate::update::watch(mode, false, |_| true);
        let remembered: std::sync::Arc<cohors_web::UpdateFn> =
            std::sync::Arc::new(move || crate::update::remembered(mode));
        cohors_web::UpdateSource(remembered)
    });
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
        // The page offers a setup tip and notes that it was opened (ADR-065).
        tips: !demo,
        update,
    };
    // A real fleet is live: the fleet watch keeps the scan current and the
    // page hears about each change over `/api/events`.
    let source = if demo {
        let now = now_secs();
        let scan: std::sync::Arc<cohors_web::ScanFn> =
            std::sync::Arc::new(move || cohors_core::demo::fleet(now));
        cohors_web::Source::from(scan)
    } else {
        cohors_web::Source::live(scanner.clone())
    };
    let result = cohors_web::serve(assets, source, options);
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
    is_tty && std::env::var_os("NO_COLOR").is_none() && !report_on_a_pipe()
}

/// `--report`, chosen once at startup (see [`set_report`]).
static REPORT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Record the global `--report` flag. Called once, from `main`, before any
/// command runs; a process-wide switch like [`look::init`], so the dozen
/// commands that print a report do not each take one more argument.
pub fn set_report(on: bool) {
    REPORT.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether a read prints its report (what a person reads) rather than JSON.
///
/// A terminal gets the report and a pipe gets JSON, so scripts keep working.
/// `--json` always wins, and `--report` asks for the report on a pipe too:
/// the full JSON of a view is many times larger than its report, which
/// matters when the reader is a coding agent that pays for every character
/// (ADR-070).
fn wants_report(json: bool) -> bool {
    !json && (REPORT.load(std::sync::atomic::Ordering::Relaxed) || std::io::stdout().is_terminal())
}

/// `--report` on a pipe: the reader is a program, so the report is plain
/// text. Without this a forced report would carry colour codes, since most
/// commands colour whenever they print for a person.
fn report_on_a_pipe() -> bool {
    REPORT.load(std::sync::atomic::Ordering::Relaxed) && !std::io::stdout().is_terminal()
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
    roots: &[(String, String)],
    first_run: bool,
    tip: Option<&cohors_core::suggest::Suggestion>,
    color: bool,
) -> String {
    let groups = found_groups(overview, roots);
    let where_at = match groups.as_slice() {
        [one] => format!("in {}", one.label),
        many => format!(
            "in {}: {}",
            plural(many.len(), "folder", "folders"),
            many.iter()
                .map(|g| g.label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let mut out = masthead(&overview.headline, &where_at, color);
    let width = look::get().width.max(40);
    let name_of = |id: &str| {
        overview
            .repos
            .iter()
            .find(|r| r.id == id)
            .map(|r| crate::ui::text::safe(&r.name))
            .unwrap_or_else(|| id.rsplit('/').next().unwrap_or(id).to_string())
    };

    if overview.actions.is_empty() {
        out.push_str(&section("do next", "nothing needs you right now", color));
    } else {
        out.push_str(&section("do next", "most urgent first", color));
        for action in overview.actions.iter().take(LAUNCH_STEPS) {
            // Which repos it is about, so the step says where, not only what.
            let names: Vec<String> = action.repos.iter().map(|id| name_of(id)).collect();
            let room = width.saturating_sub(4 + BLOCK_W + 2 + action.title.chars().count() + 3);
            let mut repos = String::new();
            if !names.is_empty() && room >= 8 {
                let shown = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
                let more = names.len().saturating_sub(3);
                let text = if more > 0 {
                    format!("{shown} +{more}")
                } else {
                    shown
                };
                repos = format!("  {}", dim(color, &clip(&text, room)));
            }
            out.push_str(&format!(
                "    {}  {}{repos}\n",
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

    // What was found, and where: each folder with its counts and its repos
    // by name, at-risk ones first and marked.
    out.push_str(&section(
        "found",
        &format!(
            "{} in {}",
            plural(overview.repos.len(), "repository", "repositories"),
            plural(groups.len(), "folder", "folders")
        ),
        color,
    ));
    for group in &groups {
        let mut counts = vec![plural(group.repos.len(), "repository", "repositories")];
        if group.nested > 0 {
            counts.push(format!("{} inside another repository", group.nested));
        }
        out.push_str(&format!(
            "    {}  {}\n",
            bold(color, &group.label),
            dim(color, &counts.join(" · "))
        ));
        out.push_str(&found_rows(&group.repos, width, color));
    }
    if first_run && roots.len() > 1 {
        out.push_str(&guide_row(
            "cohors --root <dir>",
            "next time, to show just one of these folders",
            0,
            color,
        ));
    }
    out.push('\n');

    // One quiet line about a part of cohors not used yet (ADR-065).
    if let Some(tip) = tip {
        out.push_str(&tip_line(tip, width, color));
    }
    out.push_str(&key_bar(
        &[("enter", "open the dashboard"), ("q", "quit")],
        color,
    ));
    out
}

/// The start screen's tip: `Tip: Open your fleet in the browser  cohors web`,
/// then a blank line. The command is left to `cohors next` when the line would
/// not fit, and how to hide the tip is added when it does.
fn tip_line(tip: &cohors_core::suggest::Suggestion, width: usize, color: bool) -> String {
    let title = crate::ui::text::safe(&tip.title);
    let command = if tip.copyable() {
        crate::ui::text::safe(&tip.command)
    } else {
        // Keys: they work in the dashboard, which this screen opens.
        match tip.action() {
            action if action.is_empty() => "(in the dashboard)".to_string(),
            action => format!("{} in the dashboard", crate::ui::text::safe(&action)),
        }
    };
    let lead = 2 + "Tip: ".len() + title.chars().count() + 3;
    let (shown, room) = if lead + command.chars().count() <= width {
        (
            brand(color, &command),
            width - lead - command.chars().count(),
        )
    } else {
        let note = "`cohors next` shows how";
        (dim(color, note), width.saturating_sub(lead + note.len()))
    };
    // A first-run tip retires after a day by itself: no need to say how to
    // hide it.
    let hide = tip
        .tip
        .as_deref()
        .filter(|_| !tip.is_first_run_tip())
        .map(|id| format!(" \u{b7} cohors next --dismiss {id} hides it"))
        .filter(|h| h.chars().count() <= room)
        .map(|h| dim(color, &h))
        .unwrap_or_default();
    format!(
        "  {} {}   {shown}{hide}\n\n",
        dim(color, "Tip:"),
        bold(color, &title)
    )
}

/// One folder on the start screen: its label and the repos found under it
/// (most urgent first, the overview's order).
struct FoundGroup<'a> {
    label: String,
    repos: Vec<&'a cohors_core::RepoLine>,
    /// How many of them sit inside another repo found here.
    nested: usize,
}

/// The overview's repos by the root that holds them: `roots` are `(label,
/// path)`, and a repo belongs to the deepest root its path is under. Repos
/// under none of them (remembered from elsewhere) form a group of their own.
fn found_groups<'a>(
    overview: &'a cohors_core::Overview,
    roots: &[(String, String)],
) -> Vec<FoundGroup<'a>> {
    let under = |path: &str, root: &str| {
        path == root || path.starts_with(&format!("{}/", root.trim_end_matches('/')))
    };
    let mut groups: Vec<FoundGroup> = roots
        .iter()
        .map(|(label, _)| FoundGroup {
            label: crate::ui::text::safe(label),
            repos: Vec::new(),
            nested: 0,
        })
        .collect();
    let mut elsewhere: Vec<&cohors_core::RepoLine> = Vec::new();
    for repo in &overview.repos {
        let path = repo.path.as_deref().unwrap_or(&repo.id);
        let home = roots
            .iter()
            .enumerate()
            .filter(|(_, (_, root))| under(path, root))
            .max_by_key(|(_, (_, root))| root.len())
            .map(|(i, _)| i);
        match home {
            Some(i) => groups[i].repos.push(repo),
            None => elsewhere.push(repo),
        }
    }
    if !elsewhere.is_empty() {
        groups.push(FoundGroup {
            label: "elsewhere".to_string(),
            repos: elsewhere,
            nested: 0,
        });
    }
    for group in &mut groups {
        let paths: Vec<&str> = group
            .repos
            .iter()
            .map(|r| r.path.as_deref().unwrap_or(&r.id))
            .collect();
        group.nested = paths
            .iter()
            .filter(|p| paths.iter().any(|q| q != *p && under(p, q)))
            .count();
    }
    groups.retain(|g| !g.repos.is_empty());
    groups
}

/// A folder's repositories in labelled rows, the worst first, so what each
/// name means is said in words and not only in colour:
///
/// ```text
///       at risk      7   cohors  hybrid-flow  personal-website  +4 more
///       to look at  20   bill-splitter  code-archive  cutroom  +17 more
///       clean        2   alpha  gamma
/// ```
///
/// A row with nobody in it is left out. Each row takes at most two lines of
/// names; the rest are counted (`+4 more`), and all of them are in the
/// dashboard.
fn found_rows(repos: &[&cohors_core::RepoLine], width: usize, color: bool) -> String {
    const INDENT: usize = 6;
    /// `to look at` is the longest label.
    const LABEL: usize = 10;
    const MAX_LINES: usize = 2;
    let lead = LABEL + 1 + 3 + 3; // label, space, count, gap
    let room = width.saturating_sub(INDENT + lead).max(20);
    type Pick = fn(&cohors_core::RepoLine) -> bool;
    let rows: [(&str, Pick); 3] = [
        ("at risk", |r| r.at_risk),
        ("to look at", |r| !r.at_risk && !r.reasons.is_empty()),
        ("clean", |r| !r.at_risk && r.reasons.is_empty()),
    ];
    let mut out = String::new();
    for (label, pick) in rows {
        let members: Vec<String> = repos
            .iter()
            .filter(|r| pick(r))
            .map(|r| crate::ui::text::safe(&r.name))
            .collect();
        if members.is_empty() {
            continue;
        }
        // The label in the row's colour: at risk in the risk red, clean dim.
        let painted_label = match label {
            "at risk" => sgr(color, "1;38;2;214;96;84", &format!("{label:<LABEL$}")),
            "clean" => dim(color, &format!("{label:<LABEL$}")),
            _ => bold(color, &format!("{label:<LABEL$}")),
        };
        let paint = |name: &str| match label {
            "at risk" => sgr(color, "38;2;214;96;84", name),
            "clean" => dim(color, name),
            _ => name.to_string(),
        };
        // Lay the names out in up to MAX_LINES lines of `room` columns,
        // keeping room on the last for `+N more` when some are left over.
        let mut lines: Vec<(String, usize)> = vec![(String::new(), 0)];
        let mut shown = 0;
        for (i, name) in members.iter().enumerate() {
            let w = name.chars().count();
            let left_after = members.len() - i - 1;
            let reserve = if left_after > 0 && lines.len() == MAX_LINES {
                format!("  +{left_after} more").len()
            } else {
                0
            };
            let (_, used) = lines.last().unwrap();
            let gap = if *used == 0 { 0 } else { 2 };
            if *used + gap + w + reserve > room {
                if lines.len() == MAX_LINES || *used == 0 {
                    break;
                }
                lines.push((String::new(), 0));
            }
            let (line, used) = lines.last_mut().unwrap();
            if *used > 0 {
                line.push_str("  ");
                *used += 2;
            }
            line.push_str(&paint(name));
            *used += w;
            shown += 1;
        }
        let left = members.len() - shown;
        if left > 0 {
            let (line, _) = lines.last_mut().unwrap();
            line.push_str(&dim(color, &format!("  +{left} more")));
        }
        for (n, (line, _)) in lines.iter().enumerate() {
            let head = if n == 0 {
                format!(
                    "{painted_label} {}   ",
                    dim(color, &format!("{:>3}", members.len()))
                )
            } else {
                " ".repeat(lead)
            };
            out.push_str(&format!("{}{head}{line}\n", " ".repeat(INDENT)));
        }
    }
    out
}

/// The repositories a command is about to act on, one per line with its
/// path; for a commit, each with the files that would go in.
fn print_targets(targets: &[&cohors_core::RepoSnapshot], action: Option<&CliAction>, color: bool) {
    let w = targets
        .iter()
        .map(|s| s.name.chars().count())
        .max()
        .unwrap_or(0);
    for s in targets {
        let path = s.path.as_ref().map(|p| p.as_str()).unwrap_or_default();
        print!("{}", guide_row(&s.name, path, w, color));
        // A commit's preview names the files, so nothing goes in unseen.
        if let (Some(CliAction::Commit(_, opts)), Some(path)) = (action, &s.path) {
            print!("{}", commit_preview(path, *opts, color));
        }
    }
}

/// Whether to ask before acting: a real run (not `--dry-run`), for a person
/// (not `--json`), not told `--yes`, with a terminal at both ends. A script
/// or an agent piping cohors is never asked; it chose its repos already.
fn should_ask(dry_run: bool, json: bool, yes: bool) -> bool {
    !dry_run && !json && !yes && std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Ask `question [y/N]` and wait for the answer: only `y` or `yes` goes
/// ahead; Enter alone, anything else, or a closed input is no.
fn confirm(question: &str, color: bool) -> bool {
    use std::io::Write;
    print!("  {} {} ", bold(color, question), dim(color, "[y/N]"));
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).is_ok() && is_yes(&answer)
}

fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// `pull` → `Pull`.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
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
    // Each root as shown (`~/…`) and as the path its repos sit under.
    let pairs: Vec<(String, String)> = scanner
        .root_dirs()
        .iter()
        .map(|dir| (crate::ui::abbrev_home(dir.as_str()), dir.to_string()))
        .collect();
    // A setup tip for a person at the terminal, not for a report (ADR-065).
    let tip_env = interactive
        .then(cohors_fleet::tips::TipEnv::from_env)
        .flatten();
    let tip = tip_env
        .as_ref()
        .and_then(|env| cohors_fleet::tips::current_with_first_run(env, now_secs(), 1).pop());
    print!(
        "{}",
        launch_screen(&overview, &pairs, first_run, tip.as_ref(), color)
    );
    let _ = std::io::stdout().flush();
    if let (Some(env), Some(tip)) = (&tip_env, &tip) {
        cohors_fleet::tips::note_shown(env, std::slice::from_ref(tip), now_secs());
    }

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

/// What `cohors mcp --allow-open` prints on stderr. The flag was meant for an
/// `open` tool that was never built, so it never did anything; it is still
/// accepted so a server registered with it keeps starting.
pub const ALLOW_OPEN_DEPRECATED: &str = "cohors: --allow-open is deprecated and does nothing (there is no `open` tool: an agent gets a repo's folder from `repo_path`). Remove it from the server's registration; a later release will refuse it.";

/// What `cohors mcp` prints when a human runs it at a terminal: what the server
/// is, why nothing appeared to happen, and the copy-pasteable line that registers
/// it. The absolute path matters — agents spawn with a stripped `PATH`, so a bare
/// `cohors mcp` often fails to resolve and the agent silently ends up with no
/// tools at all.
fn mcp_guide(exe: &str, caps: cohors_mcp::Caps, roots: &[String], color: bool) -> String {
    let (writes, run, launchpad) = (caps.allow_writes, caps.allow_run, caps.allow_launchpad);
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
            "overview, next, list_repos, get_repo, fleet_summary, standup, search, changes, repo_path, discover, list_prs, ci_status"
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
    // dashboard or `cohors update` (an agent starting a server must never set
    // off an install). It looks in the background, now and every few minutes,
    // so an agent-only user hears about new releases too.
    let mode = crate::update::effective_mode(scanner.updates_mode());
    crate::update::watch(mode, false, |_| true);
    let update = move || crate::update::remembered(mode);
    // `next` ends with the setup tips, as `cohors next --json` does. Like the
    // JSON output, the server never counts a tip as shown: only a person
    // reading the list does.
    let tip_env = cohors_fleet::tips::TipEnv::from_env();
    let tips = |now: i64| {
        tip_env
            .as_ref()
            .map(|env| cohors_fleet::tips::current(env, now, cohors_core::suggest::TIPS_AT_ONCE))
            .unwrap_or_default()
    };
    let user_home = cohors_config::paths::home_dir().ok();
    let cwd = std::env::current_dir().ok();
    // The `discover` tool may only look: its deep search never remembers
    // anything (only the user's own `cohors discover --deep` does).
    let discover = |deep: bool| scanner.discover_read_only(deep);
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
        cohors_mcp::Sources {
            update: &update,
            tips: &tips,
            user_home: user_home.as_deref().map(|h| h.as_std_path()),
            cwd: cwd.as_deref(),
            discover: &discover,
            root_overrides: &cli.root,
        },
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
        let roots = [("~/code".to_string(), "~/code".to_string())];
        for style in look::Style::ALL {
            for ascii in [false, true] {
                let current = look::Look {
                    style,
                    ascii,
                    width: 80,
                };
                let screen = look::with(current, || launch_screen(&o, &roots, false, None, false));
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
        let roots = [
            ("~/code".to_string(), "~/code".to_string()),
            ("~/work".to_string(), "~/work".to_string()),
        ];
        let current = look::Look {
            style: look::DEFAULT_STYLE,
            ascii: false,
            width: 80,
        };
        let screen = look::with(current, || launch_screen(&o, &roots, true, None, false));
        insta::assert_snapshot!("launch_screen_first_run", screen);
    }

    #[test]
    fn launch_screen_shows_the_headline_next_steps_and_keys() {
        let now = 1_700_000_000;
        let fleet = cohors_core::demo::fleet(now);
        let (entries, board) = cohors_core::launchpad::demo();
        let o = cohors_core::overview(&fleet, &entries, &board, now);
        let roots = [("~/code".to_string(), "~/code".to_string())];

        let w = launch_screen(&o, &roots, false, None, false);
        assert!(w.contains(&o.headline)); // real counts, not just a folder name
        assert!(w.contains("in ~/code")); // where it looked
        // What was found there: the count, and the repos by name.
        assert!(
            w.contains("Found") && w.contains(&format!("{} repositories", o.repos.len())),
            "{w}"
        );
        assert!(o.repos.iter().take(3).all(|r| w.contains(&r.name)), "{w}");
        assert!(w.contains("Do next"));
        // The first steps are listed, most urgent first, with a shape mark…
        for action in o.actions.iter().take(LAUNCH_STEPS) {
            assert!(w.contains(&action.title), "{}", action.title);
        }
        assert!(w.find(&o.actions[0].title) < w.find(&o.actions[1].title));
        assert!(w.contains("RISK")); // the severity block, as in the dashboard
        // …and the rest are counted, not dumped.
        let more = o.actions.len() - LAUNCH_STEPS;
        assert!(w.contains(&format!("and {more} more in the dashboard")));
        assert!(w.contains("open the dashboard") && w.contains("quit"));
        assert!(!w.contains("Looking in")); // only the first run lists folders
        assert!(!w.contains('\x1b'));
        assert!(launch_screen(&o, &roots, false, None, true).contains('\x1b'));
    }

    /// A setup tip is one line above the keys, with how to hide it when that
    /// fits, and nothing else on the screen moves for it.
    #[test]
    fn the_start_screen_has_one_tip_line_above_the_keys() {
        use cohors_core::suggest::{ClaudeFacts, SetupFacts, setup_tips};
        let now = 1_700_000_000;
        let o = cohors_core::overview(
            &cohors_core::demo::fleet(now),
            &[],
            &Default::default(),
            now,
        );
        let roots = [("~/code".to_string(), "~/code".to_string())];
        let facts = SetupFacts {
            claude: Some(ClaudeFacts {
                cli: "claude".into(),
                has_cohors: Some(false),
            }),
            cohors_path: Some("/opt/homebrew/bin/cohors".into()),
            ..Default::default()
        };
        let tips = setup_tips(&facts);
        let current = look::Look {
            style: look::DEFAULT_STYLE,
            ascii: false,
            width: 96,
        };
        let without = look::with(current, || launch_screen(&o, &roots, false, None, false));
        let with = look::with(current, || {
            launch_screen(&o, &roots, false, Some(&tips[0]), false)
        });
        let line = with.lines().find(|l| l.contains("Tip:")).expect("{with}");
        assert_eq!(
            line,
            "  Tip: Open your fleet in the browser   cohors web \u{b7} cohors next --dismiss web hides it"
        );
        let keys = with
            .lines()
            .position(|l| l.contains("open the dashboard"))
            .unwrap();
        let tip_at = with.lines().position(|l| l.contains("Tip:")).unwrap();
        assert!(tip_at < keys, "{with}");
        assert_eq!(with.replace(&format!("{line}\n\n"), ""), without);

        // Too long for the line: the command is left to `cohors next`.
        let claude = look::with(current, || {
            launch_screen(&o, &roots, false, Some(&tips[1]), false)
        });
        let line = claude.lines().find(|l| l.contains("Tip:")).unwrap();
        assert!(
            line.contains("Let Claude Code see your fleet")
                && line.contains("`cohors next` shows how"),
            "{line}"
        );
        assert!(line.chars().count() <= 96, "{line}");

        // A first-run tip about the dashboard's keys says where they work,
        // and nothing about hiding it (it retires after a day by itself).
        let learn = cohors_core::suggest::learn_tips();
        let keys = look::with(current, || {
            launch_screen(&o, &roots, false, Some(&learn[2]), false)
        });
        let line = keys.lines().find(|l| l.contains("Tip:")).unwrap();
        assert_eq!(
            line,
            "  Tip: Filter the repositories by name as you type   press / in the dashboard"
        );
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
        let roots = [
            ("~/projects".to_string(), "~/projects".to_string()),
            ("~/code".to_string(), "~/code".to_string()),
        ];
        let w = launch_screen(&o, &roots, true, None, false);
        // Only folders where something was found are listed, with counts.
        assert!(w.contains("Found"), "{w}");
        assert!(w.contains("~/code") && w.contains("repositories"), "{w}");
        assert!(w.contains("cohors --root <dir>")); // the scoped alternative
        assert!(w.contains("enter") && w.contains("open the dashboard"));
    }

    /// Repos are grouped under the folder that holds them, nested ones are
    /// counted, at-risk ones come first and are marked, and each step names
    /// the repos it is about.
    #[test]
    fn the_start_screen_says_what_was_found_where() {
        let now = 1_700_000_000;
        let fleet = cohors_core::demo::fleet(now);
        let mut o = cohors_core::overview(&fleet, &[], &Default::default(), now);
        // Move two repos to a second folder, one of them nested in the other.
        let (a, b) = (o.repos[0].name.clone(), o.repos[1].name.clone());
        o.repos[0].path = Some("~/work/outer".to_string());
        o.repos[1].path = Some("~/work/outer/inner".to_string());
        let roots = [
            ("~/code".to_string(), "~/code".to_string()),
            ("~/work".to_string(), "~/work".to_string()),
        ];
        let w = launch_screen(&o, &roots, false, None, false);
        assert!(w.contains("in 2 folders: ~/code, ~/work"), "{w}");
        assert!(
            w.contains("2 repositories") && w.contains("1 inside another repository"),
            "{w}"
        );
        assert!(w.contains(&a) && w.contains(&b), "{w}");
        // At-risk repos are listed on a row that says so in words.
        let risky = o.repos.iter().find(|r| r.at_risk).expect("demo has risk");
        let row = w.lines().find(|l| l.contains(&risky.name)).unwrap();
        assert!(row.trim_start().starts_with("at risk"), "{row}");
        // A step names its repos.
        let step = &o.actions[0];
        let first = o.repos.iter().find(|r| r.id == step.repos[0]).unwrap();
        let row = w.lines().find(|l| l.contains(&step.title)).unwrap();
        assert!(row.contains(&first.name), "{row}");
    }

    /// `cohors --help` lists every command once, in a group that says what it
    /// is for; a subcommand's own help is left as clap writes it.
    #[test]
    fn the_help_groups_every_command_once() {
        use clap::CommandFactory;
        let command = crate::cli::Cli::command();
        let mut listed: Vec<&str> = COMMAND_GROUPS
            .iter()
            .flat_map(|(_, _, names)| names.iter().copied())
            .collect();
        let visible: Vec<String> = command
            .get_subcommands()
            .filter(|s| !s.is_hide_set())
            .map(|s| s.get_name().to_string())
            .chain(["help".to_string()])
            .collect();
        for name in &visible {
            assert!(listed.contains(&name.as_str()), "`{name}` is in no group");
        }
        listed.sort_unstable();
        let total = listed.len();
        listed.dedup();
        assert_eq!(listed.len(), total, "a command is in two groups");
        assert!(
            listed.iter().all(|n| visible.iter().any(|v| v == n)),
            "{listed:?}"
        );

        let grouped = grouped_commands(&command);
        let help = command.clone().render_help().to_string();
        let regrouped = regroup_help(&help, &grouped).expect("the top-level help");
        assert!(!regrouped.contains("\nCommands:\n"), "{regrouped}");
        for (title, purpose, _) in COMMAND_GROUPS {
            assert!(regrouped.contains(title) && regrouped.contains(purpose));
        }
        assert!(
            regrouped.contains("Options:") || regrouped.contains("options"),
            "{regrouped}"
        );
        let agent = command
            .clone()
            .find_subcommand_mut("agent")
            .unwrap()
            .render_help()
            .to_string();
        assert!(regroup_help(&agent, &grouped).is_none());
    }

    /// Only a plain yes goes ahead: Enter alone, a no, or anything unclear
    /// changes nothing.
    #[test]
    fn only_y_or_yes_goes_ahead() {
        for answer in ["y", "Y", "yes", "YES\n", " y \n"] {
            assert!(is_yes(answer), "{answer:?}");
        }
        for answer in ["", "\n", "n", "no", "yep", "sure", "y es"] {
            assert!(!is_yes(answer), "{answer:?}");
        }
        assert_eq!(capitalize("pull"), "Pull");
        // Asking needs a person: never with --dry-run, --json or --yes.
        assert!(!should_ask(true, false, false));
        assert!(!should_ask(false, true, false));
        assert!(!should_ask(false, false, true));
    }

    /// A fleet with nothing to do still gets a launch screen that says so.
    #[test]
    fn launch_screen_with_nothing_to_do_says_so() {
        let o = cohors_core::overview(&[], &[], &Default::default(), 0);
        let w = launch_screen(
            &o,
            &[("~/code".to_string(), "~/code".to_string())],
            false,
            None,
            false,
        );
        assert!(w.contains("nothing needs you right now"));
        assert!(w.contains("enter") && w.contains("open the dashboard"));
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
    fn allow_open_still_parses_but_is_hidden_and_explained() {
        use clap::{CommandFactory, Parser};
        // A registration that passes it must keep starting the server.
        let cli = crate::cli::Cli::try_parse_from(["cohors", "mcp", "--allow-open"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(crate::cli::Command::Mcp {
                allow_open: true,
                ..
            })
        ));
        // But nobody is offered it any more.
        let mut cmd = crate::cli::Cli::command();
        let help = cmd
            .find_subcommand_mut("mcp")
            .unwrap()
            .render_long_help()
            .to_string();
        assert!(!help.contains("allow-open"), "{help}");
        let guide = mcp_guide("/bin/cohors", cohors_mcp::Caps::default(), &[], false);
        assert!(
            !guide.contains("--allow-open") && !guide.contains("open tool"),
            "{guide}"
        );
        assert!(ALLOW_OPEN_DEPRECATED.contains("deprecated and does nothing"));
    }

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
    fn since_takes_yesterday_days_and_dates() {
        let now = 1_700_000_000; // 2023-11-14T22:13:20Z
        let midnight = 1_699_920_000; // 2023-11-14T00:00:00Z
        assert_eq!(parse_since("yesterday", now).unwrap(), midnight - 86_400);
        assert_eq!(parse_since("3d", now).unwrap(), now - 3 * 86_400);
        assert_eq!(parse_since("2023-11-14", now).unwrap(), midnight);
        assert_eq!(parse_since("1970-01-02", now).unwrap(), 86_400);
        assert!(parse_since("last tuesday", now).is_err());
    }
}
