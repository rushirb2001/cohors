//! Implementations of the CLI subcommands.

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use cohors_core::{Selector, SortMode};

use crate::cli::Cli;

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
pub fn scan(cli: &Cli, select: Option<&str>, json: bool) -> Result<()> {
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
        eprint!("  {} {where_at}…", dim(color, "scanning"));
        let _ = std::io::stderr().flush();
    }
    let started = std::time::Instant::now();
    let mut snapshots = scanner.scan();
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
    print!(
        "{}",
        scan_report(&snapshots, &roots, took, now_secs(), color)
    );
    Ok(())
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

/// A repo's sync cell: `↑2`, `↓5`, `↑2 ↓5`, `ok` when level with upstream, and
/// `local` when there is no upstream at all.
fn sync_label(s: &cohors_core::RepoSnapshot) -> String {
    match &s.upstream {
        None => "local".to_string(),
        Some(u) if u.ahead == 0 && u.behind == 0 => "ok".to_string(),
        Some(u) => {
            let mut t = String::new();
            if u.ahead > 0 {
                t.push_str(&format!("↑{}", u.ahead));
            }
            if u.ahead > 0 && u.behind > 0 {
                t.push(' ');
            }
            if u.behind > 0 {
                t.push_str(&format!("↓{}", u.behind));
            }
            t
        }
    }
}

/// The report's masthead: the mark beside the wordmark, mirroring the launch
/// screen so both surfaces open the same way.
fn masthead(scanned: &str, where_at: &str, color: bool) -> String {
    let mut out = String::from("\n");
    if color {
        let m = &crate::ui::SPIDER_MARK;
        out.push_str(&format!(
            "  {}   {}\n",
            brand(color, m[0]),
            brand(color, "cohors")
        ));
        out.push_str(&format!(
            "  {}   {}\n",
            brand(color, m[1]),
            dim(color, scanned)
        ));
        out.push_str(&format!(
            "  {}   {}\n\n",
            brand(color, m[2]),
            dim(color, where_at)
        ));
    } else {
        out.push_str("  cohors\n");
        out.push_str(&format!("  {scanned}\n"));
        out.push_str(&format!("  {where_at}\n\n"));
    }
    out
}

/// One report section, in the launch screen's grammar: a brand-purple heading
/// over a dim one-line descriptor.
fn section(heading: &str, descriptor: &str, color: bool) -> String {
    format!(
        "  {}\n  {}\n",
        brand(color, heading),
        dim(color, descriptor)
    )
}

/// One repo line: `▸ name  branch  sync  status`. Colour is spent only on what is
/// genuinely urgent, so a long list of ordinary dirty repos reads calm instead of
/// turning the screen into a wall of red.
fn scan_row(
    s: &cohors_core::RepoSnapshot,
    a: &cohors_core::Assessment,
    widths: (usize, usize, usize),
    color: bool,
) -> String {
    use cohors_core::Severity as S;
    let (name_w, branch_w, sync_w) = widths;
    let status = a
        .primary
        .as_ref()
        .map(|r| r.label())
        .unwrap_or_else(|| "up to date".to_string());
    let status = match a.severity {
        S::Risk => sgr(color, "31", &status),
        S::Warn => sgr(color, "33", &status),
        S::Notice => status,
        S::Ok | S::Info => dim(color, &status),
    };
    // Pad before painting: ANSI escapes have no width, so colouring first would
    // throw the column alignment off.
    let sync_cell = format!("{:<sync_w$}", sync_label(s));
    let sync_cell = match &s.upstream {
        Some(u) if u.ahead > 0 || u.behind > 0 => sgr(color, "36", &sync_cell),
        _ => dim(color, &sync_cell),
    };
    format!(
        "    {} {}  {}  {}  {}\n",
        dim(color, "▸"),
        bold(color, &format!("{:<name_w$}", clip(&s.name, name_w))),
        dim(
            color,
            &format!("{:<branch_w$}", clip(&s.branch.label(), branch_w))
        ),
        sync_cell,
        status,
    )
}

/// The human-facing `cohors scan` report: a branded header, one line per repo in
/// the dashboard's colour language, then the fleet's headline counts and the
/// commands that would actually clear them. `color` gates the ANSI; otherwise
/// pure so it's unit-testable.
fn scan_report(
    snaps: &[cohors_core::RepoSnapshot],
    roots: &[String],
    took: std::time::Duration,
    now: i64,
    color: bool,
) -> String {
    let where_at = roots
        .iter()
        .map(|r| crate::ui::abbrev_home(r))
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = masthead(
        &format!(
            "scanned {} repo{} in {}",
            snaps.len(),
            if snaps.len() == 1 { "" } else { "s" },
            took_label(took)
        ),
        &where_at,
        color,
    );
    if snaps.is_empty() {
        out.push_str(&format!("  {}\n\n", dim(color, "nothing to report")));
        return out;
    }

    // Most urgent first, then alphabetical so the order is stable run to run.
    // Sorted descending by severity, the three buckets are contiguous slices.
    use cohors_core::Severity as S;
    let mut rows: Vec<(&cohors_core::RepoSnapshot, cohors_core::Assessment)> = snaps
        .iter()
        .map(|s| (s, cohors_core::assess(s, now)))
        .collect();
    rows.sort_by(|a, b| {
        b.1.severity
            .cmp(&a.1.severity)
            .then_with(|| a.0.name.cmp(&b.0.name))
    });
    let n_risk = rows.iter().filter(|(_, a)| a.severity == S::Risk).count();
    let n_urgent = rows.iter().filter(|(_, a)| a.severity >= S::Warn).count();
    let n_minor = rows
        .iter()
        .filter(|(_, a)| matches!(a.severity, S::Notice | S::Info))
        .count();
    let n_calm = rows.len() - n_urgent - n_minor;

    // Size the columns to the rows actually printed (capped), so a short fleet
    // stays tight and one long branch name cannot push Status off the screen.
    let listed = &rows[..n_urgent + n_minor];
    let widths = (
        listed
            .iter()
            .map(|(s, _)| s.name.chars().count())
            .max()
            .unwrap_or(4)
            .clamp(4, 28),
        listed
            .iter()
            .map(|(s, _)| s.branch.label().chars().count())
            .max()
            .unwrap_or(6)
            .clamp(6, 22),
        listed
            .iter()
            .map(|(s, _)| sync_label(s).chars().count())
            .max()
            .unwrap_or(5)
            .clamp(5, 8),
    );

    if n_urgent > 0 {
        let mut what = Vec::new();
        if n_risk > 0 {
            what.push(format!("{n_risk} at risk"));
        }
        if n_urgent - n_risk > 0 {
            what.push(format!("{} warning", n_urgent - n_risk));
        }
        out.push_str(&section("needs you", &what.join(" · "), color));
        for (s, a) in &rows[..n_urgent] {
            out.push_str(&scan_row(s, a, widths, color));
        }
        out.push('\n');
    }
    if n_minor > 0 {
        out.push_str(&section(
            "minor",
            &format!("{n_minor} with uncommitted work or small drift"),
            color,
        ));
        for (s, a) in &rows[n_urgent..n_urgent + n_minor] {
            out.push_str(&scan_row(s, a, widths, color));
        }
        out.push('\n');
    }
    // The calm ones collapse to a count: listing a dozen "up to date" rows is the
    // noise this report exists to remove.
    if n_calm > 0 {
        out.push_str(&section(
            "calm",
            &format!("{n_calm} up to date, nothing to do"),
            color,
        ));
        out.push('\n');
    }

    let sum = cohors_core::fleet_summary(snaps, now);

    // Turn the counts into the commands that clear them, listing only what is
    // actually true right now. Both are safe verbs (pull is fast-forward-only,
    // push never force-pushes).
    let mut next: Vec<(String, String)> = Vec::new();
    if sum.behind > 0 {
        next.push((
            "cohors pull --select behind".to_string(),
            format!("fast-forward {} behind", sum.behind),
        ));
    }
    if sum.unpushed > 0 {
        next.push((
            "cohors push --select ahead".to_string(),
            format!("send {} with unpushed commits", sum.unpushed),
        ));
    }
    if !next.is_empty() {
        out.push_str(&section("next", "the commands that clear this", color));
        let cmd_w = next
            .iter()
            .map(|(c, _)| c.chars().count())
            .max()
            .unwrap_or(0);
        for (cmd, why) in next {
            out.push_str(&format!(
                "    {} {}  {}\n",
                dim(color, "▸"),
                bold(color, &format!("{cmd:<cmd_w$}")),
                dim(color, &why),
            ));
        }
        out.push('\n');
    }
    out
}

/// A bulk git action issued from the CLI. Mirrors the TUI verbs and the MCP
/// action tools — one core (`crate::action`), three surfaces.
pub enum CliAction {
    Fetch,
    Pull,
    Push,
    /// `None` means the message was omitted, which prints the guide.
    Commit(Option<String>),
    Stash,
}

impl CliAction {
    /// The verb used in the human-facing summary lines.
    fn verb(&self) -> &'static str {
        match self {
            CliAction::Fetch => "fetch",
            CliAction::Pull => "pull",
            CliAction::Push => "push",
            CliAction::Commit(_) => "commit",
            CliAction::Stash => "stash",
        }
    }

    /// Present participle, for the "doing it now" heading.
    fn doing(&self) -> &'static str {
        match self {
            CliAction::Fetch => "fetching",
            CliAction::Pull => "pulling",
            CliAction::Push => "pushing",
            CliAction::Commit(_) => "committing",
            CliAction::Stash => "stashing",
        }
    }

    /// The extra argument this verb needs beyond a selector, shown in the guide's
    /// worked examples so the shape of a real invocation is obvious.
    fn arg_hint(&self) -> &'static str {
        match self {
            CliAction::Commit(_) => "-m \"your message\"",
            _ => "",
        }
    }

    /// Whether a required argument other than the selector is still missing, which
    /// (like a missing selector) sends the user to the guide rather than an error.
    fn missing_arg(&self) -> bool {
        matches!(self, CliAction::Commit(None))
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
            CliAction::Commit(_) => (
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
    format!(
        "    {} {}  {}\n",
        dim(color, "▸"),
        bold(color, &format!("{cmd:<width$}")),
        dim(color, why),
    )
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

    // A flag list tells a new reader what exists, not what to do next. These four
    // are the shortest paths to actually seeing their own fleet.
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
        ("cohors scan", "the same sweep as a one-shot report, no TUI"),
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
const SELECTORS: [(&str, &str); 6] = [
    ("dirty", "repos with uncommitted work"),
    ("behind", "repos behind their upstream"),
    ("ahead", "repos with unpushed commits"),
    ("attention", "anything that wants you"),
    ("name:pay*", "by name, globs allowed"),
    ("all", "the whole fleet, deliberately"),
];

/// What a bulk action prints when no selector was given: what the verb does, how
/// to pick repos, and where to start if you are new. A selector is required so an
/// action never silently hits every repo you own, and a missing one is a question
/// to answer, not an error to punish.
fn action_guide(verb: &str, arg: &str, does: &str, safety: &str, color: bool) -> String {
    let mut out = masthead(&format!("{verb} · {does}"), safety, color);

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
    print!("{}", masthead(&format!("{verb} · {does}"), safety, color));

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
        }
        println!();
        return Ok(());
    }

    // Pull/stash/commit mutate the worktree: a conflicted or mid-operation
    // repo is reported and skipped (same `worktree_block` rule every surface
    // applies); fetch/push never touch the worktree.
    let guard_worktree = matches!(
        action,
        CliAction::Pull | CliAction::Stash | CliAction::Commit(_)
    );
    print!(
        "{}",
        section(action.doing(), &format!("{n} repo{plural}"), color)
    );
    let mut ok = 0usize;
    let mut skipped = 0usize;
    for s in &targets {
        let path = s.path.as_ref().unwrap();
        if guard_worktree && let Some(reason) = cohors_actions::worktree_block(s) {
            skipped += 1;
            println!("    {} {}: {reason}", dim(color, "·"), dim(color, &s.name));
            continue;
        }
        let result = match &action {
            CliAction::Fetch => crate::action::fetch(path, &s.name),
            CliAction::Pull => crate::action::pull_ff(path, &s.name),
            CliAction::Push => crate::action::push(path, &s.name),
            // Guarded above: a commit with no message printed the guide and left.
            CliAction::Commit(message) => {
                crate::action::commit(path, &s.name, message.as_deref().unwrap_or_default())
            }
            CliAction::Stash => crate::action::stash_push(path, &s.name),
        };
        match result {
            Ok(message) => {
                ok += 1;
                println!("    {} {message}", sgr(color, "32", "✓"));
            }
            Err(message) => println!("    {} {message}", sgr(color, "31", "✗")),
        }
    }
    let mut tail = format!("{ok} of {n} ok");
    if skipped > 0 {
        tail.push_str(&format!(" · {skipped} skipped"));
    }
    println!("\n  {}\n", bold(color, &tail));
    Ok(())
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
    print!("{}", masthead(&format!("run · {does}"), safety, color));

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
    let results = cohors_actions::run_each(&owned, command, timeout);
    let mut ok = 0usize;
    for (s, r) in targets.iter().zip(&results) {
        if r.ok {
            ok += 1;
        }
        let (mark, tag) = if r.timed_out {
            (sgr(color, "33", "!"), "timed out")
        } else if r.ok {
            (sgr(color, "32", "✓"), "ok")
        } else {
            (sgr(color, "31", "✗"), "failed")
        };
        println!("\n  {mark} {}  {}", bold(color, &s.name), dim(color, tag));
        if !r.stdout.trim().is_empty() {
            print!("{}", r.stdout);
        }
        if !r.stderr.trim().is_empty() {
            eprint!("{}", r.stderr);
        }
    }
    println!("\n  {}\n", bold(color, &format!("{ok} of {n} ok")));
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

/// `cohors web` — one command to build, serve, and open the web dashboard.
///
/// It locates the `cohors-web` crate, ensures Trunk (the WASM bundler) is
/// installed, starts the dev server bound to loopback, waits until it's actually
/// accepting connections, then prints + opens the branded local URL
/// (`http://cohors.localhost:<port>`). Blocks until Ctrl-C. Must run from inside
/// the cohors repository, since Trunk builds the app from source.
pub fn run_web(
    cli: &Cli,
    port: u16,
    open: bool,
    install: bool,
    allow_writes: bool,
    allow_run: bool,
    demo: bool,
) -> Result<()> {
    // `cohors web` builds the dashboard from source, so it needs the repo. This
    // is the *developer* path. End users won't run this at all: once the
    // dashboard is deployed (v0.5 slice 4), an installed `cohors web` outside a
    // checkout will simply open the hosted URL — no local build, no Trunk, no
    // Cargo. So Trunk is a dev-only dependency, never something a distributed
    // binary must ship (TODO: wire the hosted-URL branch when we deploy).
    let web_dir = find_web_crate().context(
        "couldn't find `crates/cohors-web` — `cohors web` builds the dashboard from source, so \
         run it inside the cohors repository (a hosted version arrives with the deploy milestone)",
    )?;
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

    ensure_trunk(install)?;

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
            "web · the dashboard in your browser",
            "building the WASM bundle, this takes a moment",
            color
        )
    );
    let mut watcher = std::process::Command::new("trunk")
        .arg("watch")
        .current_dir(&web_dir)
        .spawn()
        .context("starting `trunk watch` (is Trunk installed and on PATH?)")?;

    let dist = web_dir.join("dist");
    if !wait_for_file(
        &dist.join("index.html"),
        std::time::Duration::from_secs(240),
    ) {
        let _ = watcher.kill();
        bail!("the WASM build didn't finish in time");
    }

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
    let roots = scanner.roots();
    let caps = cohors_web::Caps {
        allow_writes,
        allow_run,
    };
    let scan: std::sync::Arc<cohors_web::ScanFn> = if demo {
        let now = now_secs();
        std::sync::Arc::new(move || cohors_core::demo::fleet(now))
    } else {
        let scanner = scanner.clone();
        std::sync::Arc::new(move || scanner.scan())
    };
    let result = cohors_web::serve(
        &dist,
        port,
        scan,
        roots,
        token,
        cli.watch,
        caps,
        actions_config.run_allowlist,
        actions_config.max_action_targets,
    );
    let _ = watcher.kill();
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

/// The shared brand header: the spider mark beside a greeting + tagline (the same
/// mark the TUI header draws). The block-glyph mark appears only with colour on;
/// plain/piped output keeps a clean text greeting.
fn welcome_header(color: bool) -> String {
    let mut out = String::from("\n");
    if color {
        let m = &crate::ui::SPIDER_MARK;
        out.push_str(&format!(
            "  {}   Welcome to {}\n",
            brand(color, m[0]),
            brand(color, "cohors")
        ));
        out.push_str(&format!(
            "  {}   {}\n",
            brand(color, m[1]),
            dim(color, "All your git repositories at a glance")
        ));
        out.push_str(&format!("  {}\n\n", brand(color, m[2])));
    } else {
        out.push_str("  Welcome to cohors\n");
        out.push_str("  All your git repositories at a glance\n\n");
    }
    out
}

/// The shared super-fleet note: by default cohors gathers every folder it
/// detects into one fleet; `--root` scopes it to a single folder.
fn super_fleet_note(color: bool) -> String {
    let mut out = String::new();
    out.push_str(&dim(
        color,
        "  By default cohors gathers every folder it detects into one super fleet;",
    ));
    out.push('\n');
    out.push_str(&dim(
        color,
        "  pass --root <dir> to scope it to a single folder instead.",
    ));
    out.push('\n');
    out
}

/// The guidance a front-end prints when it finds no repos anywhere: the shared
/// welcome header, a "nothing found under <roots>" line, a nudge to point at a
/// folder, and the super-fleet note. `cmd` is the invoking command (`cohors` or
/// `cohors web`) so the sample line matches. `color` gates the ANSI; otherwise
/// pure so it's unit-testable.
fn nothing_found_hint(cmd: &str, roots: &[String], color: bool) -> String {
    let mut out = welcome_header(color);
    out.push_str(&format!(
        "  {} found no git repositories under {}.\n\n",
        cmd,
        bold(color, &roots.join(", "))
    ));
    out.push_str(&format!(
        "  Create a repo, or point cohors at a folder:   {}\n\n",
        bold(color, &format!("{cmd} --root ~/code"))
    ));
    out.push_str(&super_fleet_note(color));
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

/// One block of a launch screen: a brand-purple heading, a dim descriptor, then
/// the items (each behind a brand `▸`, bold).
fn welcome_block(heading: &str, descriptor: &str, items: &[String], color: bool) -> String {
    let mut out = format!(
        "  {}\n  {}\n",
        brand(color, heading),
        dim(color, descriptor)
    );
    for it in items {
        out.push_str(&format!("    {}  {}\n", brand(color, "▸"), bold(color, it)));
    }
    out.push('\n');
    out
}

/// The Enter/quit line shared by every launch screen.
fn launch_prompt(color: bool) -> String {
    format!(
        "  {} launch   {}   {} quit\n",
        bold(color, "[enter]"),
        dim(color, "·"),
        bold(color, "[q]")
    )
}

/// The bare-`cohors` greeting: the two ways to fly, side by side — the whole super
/// fleet (Enter launches it), or scoping to a single folder with `--root`. `color`
/// gates the ANSI; otherwise pure so it's unit-testable.
fn super_fleet_welcome(detected: &[String], color: bool) -> String {
    let mut out = welcome_header(color);
    out.push_str(&welcome_block(
        "your super fleet",
        "every repo it found, gathered from",
        detected,
        color,
    ));
    // The scoped alternative, kept front and centre: not the super fleet? point it
    // at one folder instead. `<dir>` stays dim, a placeholder to fill in.
    out.push_str(&format!(
        "  {}\n  {}\n    {}  {} {}\n\n",
        brand(color, "or a single folder"),
        dim(color, "scope the dashboard to just one"),
        brand(color, "▸"),
        bold(color, "cohors --root"),
        dim(color, "<dir>"),
    ));
    out.push_str(&launch_prompt(color));
    out
}

/// The `cohors --root <dir>` greeting: the dashboard scoped to the folder(s) you
/// pointed it at.
fn scoped_welcome(roots: &[String], color: bool) -> String {
    let mut out = welcome_header(color);
    out.push_str(&welcome_block(
        "a scoped view",
        "just the repos under",
        roots,
        color,
    ));
    out.push_str(&launch_prompt(color));
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

/// First-run onboarding, in the plain terminal *before* the alt-screen. cohors
/// auto-detects where the user keeps repos (the super fleet). When repos are
/// found we greet, list the folders, and prompt to launch (Enter) or quit; when
/// nothing is found we greet + guide instead of launching an empty dashboard.
/// Detection is a shallow probe, so no full scan happens until the dashboard.
///
/// `watch` skips the greeting *and* the prompt. `--watch` is a live-monitoring
/// mode people re-run in a loop, so a "press Enter" gate there is a keypress on
/// every iteration and the greeting is a flash before the dashboard paints over
/// it. The empty-fleet guard still applies: watching nothing is worse than being
/// told why.
fn onboard(scanner: &Arc<crate::scan::Scanner>, watch: bool) -> Result<Onboard> {
    let home = cohors_config::paths::home_dir().ok();
    let detected = cohors_fleet::detect_roots(home.as_deref());
    let color = wants_color(std::io::stdout().is_terminal());

    if detected.is_empty() {
        print!("{}", tui_welcome(&scanner.roots(), color));
        let _ = std::io::stdout().flush();
        return Ok(Onboard::Quit);
    }
    if watch {
        return Ok(Onboard::Proceed);
    }

    print!("{}", super_fleet_welcome(&detected, color));
    let _ = std::io::stdout().flush();

    // A real terminal is needed both to read the prompt and to run the TUI at
    // all; without one we've printed the welcome as guidance and stop.
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if interactive && prompt_launch()? {
        Ok(Onboard::Proceed)
    } else {
        Ok(Onboard::Quit)
    }
}

/// The `--root` counterpart to [`onboard`]: greet with the scoped-view launch
/// screen (the roots the scanner will actually walk), then prompt to launch or
/// quit. The dashboard needs a terminal anyway, so a non-interactive run just
/// prints the greeting and stops.
///
/// `watch` goes straight to the dashboard, for the same reason as in [`onboard`]:
/// `--root … --watch` is a deliberate, repeated invocation that already names the
/// folder on the command line, so there is nothing left to confirm.
fn scoped_onboard(scanner: &Arc<crate::scan::Scanner>, watch: bool) -> Result<Onboard> {
    if watch {
        return Ok(Onboard::Proceed);
    }
    let color = wants_color(std::io::stdout().is_terminal());
    let roots: Vec<String> = scanner
        .roots()
        .iter()
        .map(|r| crate::ui::abbrev_home(r))
        .collect();
    print!("{}", scoped_welcome(&roots, color));
    let _ = std::io::stdout().flush();
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if interactive && prompt_launch()? {
        Ok(Onboard::Proceed)
    } else {
        Ok(Onboard::Quit)
    }
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

/// `cohors mcp` — speak the Model Context Protocol over stdio. Read-only unless
/// the matching `--allow-*` flags are passed (none have an effect yet — only the
/// read tools are implemented).
/// What `cohors mcp` prints when a human runs it at a terminal: what the server
/// is, why nothing appeared to happen, and the copy-pasteable line that registers
/// it. The absolute path matters — agents spawn with a stripped `PATH`, so a bare
/// `cohors mcp` often fails to resolve and the agent silently ends up with no
/// tools at all.
fn mcp_guide(exe: &str, caps: (bool, bool, bool), color: bool) -> String {
    let (writes, run, open) = caps;
    let mut out = masthead(
        "mcp · your fleet, exposed to a coding agent",
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
    let ways = [
        (
            format!("claude mcp add cohors -- {exe} mcp"),
            "Claude Code, in your shell",
        ),
        (
            format!("\"cohors\": {{ \"command\": \"{exe}\", \"args\": [\"mcp\"] }}"),
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

    let state = |on: bool| if on { "on now" } else { "opt-in" };
    let rows: Vec<(String, String)> = vec![
        (
            "reads".to_string(),
            "list_repos, get_repo, fleet_summary, search, changes, repo_path, list_prs, ci_status"
                .to_string(),
        ),
        (
            "--allow-writes".to_string(),
            format!("fetch, pull, push, commit, stash · {}", state(writes)),
        ),
        (
            "--allow-run".to_string(),
            format!("run shell commands across repos · {}", state(run)),
        ),
        (
            "--allow-open".to_string(),
            format!("the local-only open tool · {}", state(open)),
        ),
    ];
    let w = rows
        .iter()
        .map(|(c, _)| c.chars().count())
        .max()
        .unwrap_or(0);
    out.push_str(&section(
        "what an agent gets",
        "reads are always on, anything that writes is opt-in",
        color,
    ));
    for (what, why) in &rows {
        out.push_str(&guide_row(what, why, w, color));
    }
    out.push('\n');
    out
}

pub fn run_mcp(cli: &Cli, allow_writes: bool, allow_run: bool, allow_open: bool) -> Result<()> {
    // An MCP client always pipes stdin. A terminal on stdin therefore means a
    // human ran this by hand, and would otherwise stare at a blank line forever
    // waiting for a client that is never going to speak. Explain and leave.
    if std::io::stdin().is_terminal() {
        let color = wants_color(std::io::stdout().is_terminal());
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(str::to_string))
            .unwrap_or_else(|| "cohors".to_string());
        print!(
            "{}",
            mcp_guide(&exe, (allow_writes, allow_run, allow_open), color)
        );
        return Ok(());
    }

    let scanner = crate::scan::from_cli(cli)?;
    let caps = cohors_mcp::Caps {
        allow_writes,
        allow_run,
        allow_open,
    };
    let roots = scanner.roots();
    let config_path = scanner.config_path();
    let token = scanner.github_token();
    let actions_config = scanner.actions_config();
    let scan = || scanner.scan();
    cohors_mcp::serve_stdio(
        &scan,
        token.as_deref(),
        &roots,
        &config_path,
        caps,
        &actions_config.run_allowlist,
        actions_config.max_action_targets,
    )
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
        assert!(hint.contains("Welcome to cohors"));
        assert!(hint.contains("cohors web found no git repositories under /home/me/projects"));
        assert!(hint.contains("cohors web --root ~/code")); // the sample command
        assert!(hint.contains("super fleet")); // the model note
        assert!(!hint.contains('\x1b')); // no ANSI when color is off
    }

    #[test]
    fn tui_welcome_guides_to_root_and_super_fleet() {
        let w = tui_welcome(&["/home/me/projects".to_string()], false);
        assert!(w.contains("cohors found no git repositories under /home/me/projects"));
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

    #[test]
    fn super_fleet_welcome_lists_roots_and_prompts_to_launch() {
        let w = super_fleet_welcome(&["~/projects".to_string(), "~/code".to_string()], false);
        assert!(w.contains("Welcome to cohors"));
        assert!(w.contains("super fleet")); // state 1: the fleet heading
        assert!(w.contains("~/projects")); // each detected folder is listed
        assert!(w.contains("~/code"));
        assert!(w.contains("single folder")); // state 2: the scoped alternative
        assert!(w.contains("cohors --root")); // …kept, not removed
        assert!(w.contains("[enter]")); // launch key
        assert!(w.contains("[q]")); // quit key
        assert!(!w.contains('\x1b'));
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
        let r = scan_report(&snaps, &roots, took, now, false);

        assert!(r.contains("cohors")); // branded masthead
        // The report says what it actually did, not just what it found.
        assert!(r.contains(&format!("scanned {} repos in 840ms", snaps.len())));
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

        // Next steps appear exactly when the fleet actually has that problem.
        let sum = cohors_core::fleet_summary(&snaps, now);
        assert_eq!(
            r.contains("cohors pull --select behind"),
            sum.behind > 0,
            "pull suggestion should track the behind count"
        );
        assert_eq!(
            r.contains("cohors push --select ahead"),
            sum.unpushed > 0,
            "push suggestion should track the unpushed count"
        );
        assert!(!r.contains('\x1b')); // no ANSI when colour is off
        assert!(scan_report(&snaps, &roots, took, now, true).contains('\x1b')); // …and some when it is
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
        assert!(g.contains("fetch · git fetch across the repos you pick"));
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

    #[test]
    fn scan_report_handles_an_empty_fleet() {
        let took = std::time::Duration::from_millis(3);
        let r = scan_report(&[], &["/home/me/projects".to_string()], took, 0, false);
        assert!(r.contains("scanned 0 repos in 3ms"));
        assert!(r.contains("nothing to report"));
        assert!(!r.contains("needs you")); // no sections at all for an empty fleet
        assert!(!r.contains('\x1b'));
    }

    #[test]
    fn scoped_welcome_names_the_scoped_folder_and_prompts() {
        let w = scoped_welcome(&["~/Main/code".to_string()], false);
        assert!(w.contains("Welcome to cohors"));
        assert!(w.contains("scoped")); // the mode heading
        assert!(w.contains("~/Main/code")); // the single folder in focus
        assert!(w.contains("[enter]"));
        assert!(w.contains("[q]"));
        assert!(!w.contains('\x1b'));
    }
}
