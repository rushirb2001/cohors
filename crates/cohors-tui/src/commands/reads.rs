//! `cohors path`, `show`, `changes`, `search` and `prs`: the fleet's read
//! tools for a person at a terminal.
//!
//! The MCP server gives an agent `repo_path`, `get_repo`, `changes`, `search`,
//! `list_prs` and `ci_status`. These are the same reads for a person, so
//! nobody has to fall back to `cd` and `git`. Each command calls the
//! `cohors_fleet` function its MCP tool calls, with the same defaults, limits
//! and secret-file rules, so the two surfaces always agree.
//!
//! Like `scan`, a report is styled for a human at a terminal and JSON when
//! piped or given `--json`. `path` prints only the path, for
//! `cd "$(cohors path payments)"`.
//!
//! Anything printed that came out of a repository (paths, branch names,
//! commit subjects, PR titles, matching lines) goes through
//! [`crate::ui::text::safe`] first: an escape sequence planted in a commit
//! message must not reach the terminal.

use std::collections::HashMap;
use std::io::IsTerminal;

use anyhow::Result;
use cohors_core::lookup::{Resolved, close_names, resolve_repo};
use cohors_core::{
    Branch, ChangeState, ChangedFile, CiStatus, RemoteDetail, RemoteInfo, RepoChanges, RepoDetail,
    RepoSnapshot, Selector, SortMode,
};
use cohors_fleet::ContentHit;
use rayon::prelude::*;
use serde_json::{Value, json};

use super::{
    ExitCode, bold, brand, clip, dim, guide_row, masthead, now_secs, parse_selector, plural,
    section, severity_mark, sgr, term_width, wants_color, wrap,
};
use crate::cli::Cli;
use crate::ui::abbrev_home;
use crate::ui::text::safe;

/// The most matches `search` reports unless asked for more (the MCP default).
pub const DEFAULT_MAX_RESULTS: usize = 200;

/// The most bytes of diff `changes --patch` prints per repo (the MCP default).
pub const DEFAULT_PATCH_BYTES: usize = 20_000;

/// What the MCP remote tools say when there is no token, word for word, so a
/// person and an agent are told the same thing.
const NO_TOKEN_NOTE: &str =
    "No GitHub token found (run `gh auth login` or set $GITHUB_TOKEN); PR/CI data is unavailable.";

/// How many changed files `show` lists before pointing at `changes`.
const SHOW_FILES: usize = 20;

/// How many recent commits `show` lists.
const SHOW_COMMITS: usize = 10;

/// The widest a repo-name column grows; past this a long name is clipped.
const NAME_W: usize = 28;

// ── Picking repos ────────────────────────────────────────────────────────────

/// The selector a read uses: `select` parsed like `scan --select`, or
/// `default` when there is none (or it picks nothing in particular, like
/// `{}`). This is what the MCP read tools do with their optional selector.
fn selector_or(select: Option<&str>, default: Selector) -> Result<Selector> {
    let selector = match select {
        Some(query) => parse_selector(query)?,
        None => Selector::default(),
    };
    Ok(if selector.is_empty() {
        default
    } else {
        selector
    })
}

/// The snapshots `selector` picks, dirty-first, the order every MCP read uses.
fn resolve_in_order<'a>(
    snaps: &'a [RepoSnapshot],
    selector: &Selector,
    now: i64,
) -> Vec<&'a RepoSnapshot> {
    let by_id: HashMap<&str, &RepoSnapshot> = snaps.iter().map(|s| (s.id.0.as_str(), s)).collect();
    cohors_core::resolve(snaps, selector, SortMode::DirtyFirst, now)
        .iter()
        .filter_map(|id| by_id.get(id.0.as_str()).copied())
        .collect()
}

/// What `path` and `show` print on stderr when nothing matches.
fn missing_message(key: &str, close: &[String], roots: &[String], fleet_empty: bool) -> String {
    if fleet_empty {
        let roots: Vec<String> = roots.iter().map(|r| abbrev_home(r)).collect();
        return format!(
            "cohors: no repositories found under {}\n  run `cohors init` to detect your repos, or pass --root <dir>\n",
            roots.join(", ")
        );
    }
    let mut out = format!("cohors: no repository matches `{}`\n", safe(key));
    if close.is_empty() {
        out.push_str("  `cohors scan` lists every repository\n");
    } else {
        out.push_str(&format!("  did you mean: {}\n", close.join(", ")));
    }
    out
}

/// What `path` and `show` print on stderr when several repos match.
fn ambiguous_message(key: &str, found: &[&RepoSnapshot]) -> String {
    let mut found = found.to_vec();
    found.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.0.cmp(&b.id.0)));
    let w = found
        .iter()
        .map(|s| safe(&s.name).chars().count())
        .max()
        .unwrap_or(0)
        .min(NAME_W);
    let mut out = format!(
        "cohors: `{}` matches {}; name one of them (or give its path):\n",
        safe(key),
        plural(found.len(), "repository", "repositories")
    );
    for snap in found {
        let path = snap
            .path
            .as_ref()
            .map(|p| abbrev_home(&safe(p.as_str())))
            .unwrap_or_default();
        out.push_str(&format!("  {:<w$}  {path}\n", clip(&safe(&snap.name), w)));
    }
    out
}

/// Whether `key` should also be tried as a path on disk.
fn path_like(key: &str) -> bool {
    key == "." || key == ".." || key.contains('/')
}

/// Resolve `key` to one repo (by the rules in [`cohors_core::lookup`], which
/// the MCP server shares), or say on stderr why not and fail with the status
/// a script can act on: 1 when nothing matches (with close names), 2 when
/// several do (listing them).
pub(super) fn resolve_or_explain<'a>(
    snaps: &'a [RepoSnapshot],
    key: &str,
    roots: &[String],
) -> Result<&'a RepoSnapshot> {
    let canonical = path_like(key)
        .then(|| std::fs::canonicalize(key).ok())
        .flatten()
        .and_then(|p| p.to_str().map(str::to_string));
    match resolve_repo(snaps, key, canonical.as_deref()) {
        Resolved::One(snap) => Ok(snap),
        Resolved::Many(found) => {
            eprint!("{}", ambiguous_message(key, &found));
            Err(ExitCode(2).into())
        }
        Resolved::Missing => {
            // A repo's name is text from the disk: no escape sequences.
            let close: Vec<String> = close_names(snaps, key).iter().map(|n| safe(n)).collect();
            eprint!("{}", missing_message(key, &close, roots, snaps.is_empty()));
            Err(ExitCode(1).into())
        }
    }
}

// ── cohors path ──────────────────────────────────────────────────────────────

/// `cohors path <repo>` — the repo's absolute path and nothing else, so
/// `cd "$(cohors path payments)"` works. The MCP `repo_path` tool's answer.
pub fn path(cli: &Cli, key: &str) -> Result<()> {
    let scanner = crate::scan::from_cli(cli)?;
    let snapshots = scanner.scan();
    let snap = resolve_or_explain(&snapshots, key, &scanner.roots())?;
    let Some(path) = &snap.path else {
        eprintln!(
            "cohors: repository `{}` has no local path",
            safe(&snap.name)
        );
        return Err(ExitCode(1).into());
    };
    // Captured by `$(…)` the path must be exactly the one on disk; shown at a
    // terminal it is text from the disk, so it is made safe first.
    if std::io::stdout().is_terminal() {
        println!("{}", safe(path.as_str()));
    } else {
        println!("{path}");
    }
    Ok(())
}

// ── cohors show ──────────────────────────────────────────────────────────────

/// `cohors show <repo>` — one repo in detail: what the dashboard's detail
/// pane shows, from the same two reads the MCP `get_repo` tool makes.
pub fn show(cli: &Cli, key: &str, json: bool) -> Result<()> {
    let human = super::wants_report(json);
    let color = wants_color(human);
    let scanner = crate::scan::from_cli(cli)?;
    let snapshots = scanner.scan();
    let snap = resolve_or_explain(&snapshots, key, &scanner.roots())?;
    let now = now_secs();

    // The local drill-in (commits, branches, stashes, changed files) and, with
    // a token, the repo's GitHub detail (PRs, issues, release).
    let detail = snap.path.as_deref().map(cohors_fleet::repo_detail);
    let token = scanner.github_token();
    let remote = match (snap.remote_url.as_deref(), token.as_deref()) {
        (Some(url), Some(token)) => cohors_fleet::fetch_repo_detail(token, url),
        _ => None,
    };

    if !human {
        let value = show_json(snap, detail.as_ref(), remote.as_ref(), now);
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    print!(
        "{}",
        show_report(
            snap,
            detail.as_ref(),
            remote.as_ref(),
            token.is_some(),
            now,
            term_width(),
            color
        )
    );
    Ok(())
}

/// The MCP `get_repo` payload: the snapshot as `cohors scan` prints it, its
/// assessment, the local `detail`, and `remote_detail` when GitHub answered.
fn show_json(
    snap: &RepoSnapshot,
    detail: Option<&RepoDetail>,
    remote: Option<&RemoteDetail>,
    now: i64,
) -> Value {
    let mut value = serde_json::to_value(snap).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut value {
        let assessment = cohors_core::assess(snap, now);
        map.insert(
            "assessment".to_string(),
            json!({
                "severity": assessment.severity,
                "needs_attention": assessment.needs_attention(),
                "reasons": assessment.reasons.iter().map(|r| r.label()).collect::<Vec<_>>(),
            }),
        );
        if let Some(detail) = detail {
            map.insert(
                "detail".to_string(),
                serde_json::to_value(detail).unwrap_or(Value::Null),
            );
        }
        if let Some(remote) = remote {
            map.insert(
                "remote_detail".to_string(),
                serde_json::to_value(remote).unwrap_or(Value::Null),
            );
        }
    }
    value
}

/// The human `cohors show` report. Pure (data and a clock in, text out) so it
/// is unit-testable; `color` gates the ANSI.
fn show_report(
    snap: &RepoSnapshot,
    detail: Option<&RepoDetail>,
    remote: Option<&RemoteDetail>,
    has_token: bool,
    now: i64,
    width: usize,
    color: bool,
) -> String {
    let name = safe(&snap.name);
    let where_at = snap.path.as_ref().map_or_else(
        || "no local path".to_string(),
        |p| abbrev_home(&safe(p.as_str())),
    );
    let mut out = masthead(
        &format!("{name} on {}", safe(&snap.branch.label())),
        &where_at,
        color,
    );

    if let Some(error) = &snap.error {
        out.push_str(&section(
            "could not be read",
            "cohors could not open this repository",
            color,
        ));
        for line in wrap(&safe(error), width.saturating_sub(6)) {
            out.push_str(&format!("    {}\n", dim(color, &line)));
        }
        out.push('\n');
        return out;
    }

    // ── Attention: the reasons the dashboard's detail pane leads with.
    let assessment = cohors_core::assess(snap, now);
    if assessment.reasons.is_empty() {
        out.push_str(&section("attention", "nothing needs you here", color));
    } else {
        out.push_str(&section("attention", "most urgent first", color));
        for reason in &assessment.reasons {
            out.push_str(&format!(
                "    {} {}\n",
                severity_mark(reason.severity(), color),
                clip(&reason.phrase(), width.saturating_sub(12))
            ));
        }
    }
    out.push('\n');

    // ── Where it stands: branch, upstream, remote, history.
    out.push_str(&section(
        "where it stands",
        "branch, upstream and remote",
        color,
    ));
    let mut facts: Vec<(&str, String)> = Vec::new();
    let mut branch = safe(&snap.branch.label());
    match (snap.on_default_branch(), &snap.default_branch) {
        (Some(true), _) => branch.push_str(" (the default branch)"),
        (Some(false), Some(default)) => {
            branch.push_str(&format!(" (the default is {})", safe(default)));
        }
        _ => {}
    }
    facts.push(("branch", branch));
    facts.push(("upstream", upstream_words(snap)));
    if let Some(op) = snap.operation {
        facts.push((
            "in progress",
            format!("a {} has stopped partway", op.label()),
        ));
    }
    if let Some(url) = &snap.remote_url {
        facts.push(("remote", safe(&redact_remote(url))));
        facts.push((
            "last fetch",
            snap.last_fetch
                .map_or_else(|| "never".to_string(), |t| ago(t, now)),
        ));
    }
    if let Some(tag) = &snap.latest_tag {
        let since = match tag.commits_since {
            0 => "HEAD is on it".to_string(),
            n => format!("{} since", plural(n as usize, "commit", "commits")),
        };
        facts.push(("latest tag", format!("{}, {since}", safe(&tag.name))));
    }
    if !snap.groups.is_empty() {
        facts.push(("groups", safe(&snap.groups.join(", "))));
    }
    if let Some(c) = &snap.last_commit {
        facts.push((
            "last commit",
            format!(
                "{} {}: {}",
                safe(&c.short_id),
                ago(c.timestamp, now),
                safe(&c.summary)
            ),
        ));
    }
    let room = width.saturating_sub(17).max(8);
    for (key, value) in &facts {
        out.push_str(&format!(
            "    {}  {}\n",
            dim(color, &format!("{key:<11}")),
            clip(value, room)
        ));
    }
    out.push('\n');

    // ── Working tree.
    let tree = if snap.worktree.is_dirty() {
        worktree_words(&snap.worktree)
    } else {
        "clean, nothing uncommitted".to_string()
    };
    out.push_str(&section("working tree", &tree, color));
    if let Some(d) = detail {
        for file in d.changed_files.iter().take(SHOW_FILES) {
            out.push_str(&file_row(file, 4, width, color));
        }
        if d.changed_files.len() > SHOW_FILES {
            let more = format!(
                "and {} more: cohors changes --select name:{}",
                d.changed_files.len() - SHOW_FILES,
                shell_word(&name)
            );
            out.push_str(&format!(
                "    {}\n",
                dim(color, &clip(&more, width.saturating_sub(4)))
            ));
        }
    }
    out.push('\n');

    // ── Recent commits.
    if let Some(d) = detail
        && !d.recent_commits.is_empty()
    {
        out.push_str(&section("recent commits", "newest first", color));
        let commits = &d.recent_commits[..d.recent_commits.len().min(SHOW_COMMITS)];
        let id_w = commits
            .iter()
            .map(|c| c.short_id.chars().count())
            .max()
            .unwrap_or(7)
            .min(12);
        let room = width.saturating_sub(4 + id_w + 2 + 4 + 2);
        for c in commits {
            out.push_str(&format!(
                "    {}  {}  {}\n",
                dim(color, &format!("{:<id_w$}", clip(&safe(&c.short_id), id_w))),
                dim(
                    color,
                    &format!("{:>4}", cohors_core::time::relative(c.timestamp, now))
                ),
                clip(&safe(&c.summary), room)
            ));
        }
        out.push('\n');
    }

    // ── Branches, each against its own upstream.
    if let Some(d) = detail
        && !d.branches.is_empty()
    {
        out.push_str(&section(
            "branches",
            &format!(
                "{}; * is checked out",
                plural(d.branches.len(), "branch", "branches")
            ),
            color,
        ));
        let bw = d
            .branches
            .iter()
            .map(|b| safe(&b.name).chars().count())
            .max()
            .unwrap_or(0)
            .min(width / 2);
        for b in &d.branches {
            let label = format!("{:<bw$}", clip(&safe(&b.name), bw));
            let (mark, label) = if b.is_current {
                ("*", bold(color, &label))
            } else {
                (" ", label)
            };
            let mut sync = branch_sync(b);
            if let Some(c) = &b.last_commit {
                sync.push_str(&format!(", last commit {}", ago(c.timestamp, now)));
            }
            out.push_str(&format!(
                "    {mark} {label}  {}\n",
                dim(color, &clip(&sync, width.saturating_sub(8 + bw)))
            ));
        }
        out.push('\n');
    }

    // ── Stashes.
    if let Some(d) = detail
        && !d.stashes.is_empty()
    {
        out.push_str(&section(
            "stashes",
            &format!(
                "{}, newest first",
                plural(d.stashes.len(), "stash", "stashes")
            ),
            color,
        ));
        for (i, stash) in d.stashes.iter().enumerate() {
            out.push_str(&format!(
                "    {}  {}\n",
                dim(color, &format!("stash@{{{i}}}")),
                clip(&safe(stash), width.saturating_sub(18))
            ));
        }
        out.push('\n');
    }

    // ── GitHub.
    out.push_str(&github_section(snap, remote, has_token, width, color));

    // ── Next.
    let word = shell_word(&name);
    let mut steps = vec![(
        format!("cd \"$(cohors path {word})\""),
        "a shell in this repo",
    )];
    if snap.is_dirty() {
        steps.push((
            format!("cohors changes --select name:{word} --patch"),
            "its uncommitted diff",
        ));
    }
    out.push_str(&section("next", "commands for this repo", color));
    out.push_str(&guide_rows(&steps, width, color));
    out.push('\n');
    out
}

/// The GitHub part of `show`: PRs, issues, the latest run and release when
/// GitHub answered; how to connect it when there is no token; nothing for a
/// repo that is not on GitHub.
fn github_section(
    snap: &RepoSnapshot,
    remote: Option<&RemoteDetail>,
    has_token: bool,
    width: usize,
    color: bool,
) -> String {
    let on_github = snap.remote_kind() == Some(cohors_core::RemoteKind::GitHub);
    let mut out = String::new();
    match remote {
        Some(r) => {
            let mut summary = vec![
                plural(r.prs.len(), "open PR", "open PRs"),
                plural(r.open_issues as usize, "open issue", "open issues"),
            ];
            if r.assigned_issues > 0 {
                summary.push(format!("{} assigned to you", r.assigned_issues));
            }
            out.push_str(&section("GitHub", &summary.join(", "), color));
            let mut facts: Vec<(&str, String)> = Vec::new();
            if let Some(run) = r.latest_run {
                facts.push(("latest run", ci_word(run).to_string()));
            }
            if let Some(release) = &r.latest_release {
                facts.push(("release", safe(release)));
            }
            if let Some(protected) = r.default_branch_protected {
                let words = if protected {
                    "the default branch is protected"
                } else {
                    "the default branch is not protected"
                };
                facts.push(("protection", words.to_string()));
            }
            for (key, value) in &facts {
                out.push_str(&format!(
                    "    {}  {}\n",
                    dim(color, &format!("{key:<11}")),
                    clip(value, width.saturating_sub(17).max(8))
                ));
            }
            for pr in &r.prs {
                let mut tail = vec![safe(&pr.author)];
                if pr.draft {
                    tail.push("draft".to_string());
                }
                if !pr.requested_reviewers.is_empty() {
                    tail.push(format!(
                        "waiting on {}",
                        safe(&pr.requested_reviewers.join(", "))
                    ));
                }
                // indent + `#1234` + gaps; the title takes what the tail leaves.
                let avail = width.saturating_sub(4 + 6 + 2 + 2);
                let tail = clip(&tail.join(", "), avail / 3);
                let title = clip(
                    &safe(&pr.title),
                    avail.saturating_sub(tail.chars().count()).max(8),
                );
                out.push_str(&format!(
                    "    {}  {title}  {}\n",
                    brand(color, &format!("{:>6}", format!("#{}", pr.number))),
                    dim(color, &tail)
                ));
            }
        }
        None if on_github && !has_token => {
            out.push_str(&section("GitHub", "not connected", color));
            for line in wrap(NO_TOKEN_NOTE, width.saturating_sub(6)) {
                out.push_str(&format!("    {}\n", dim(color, &line)));
            }
        }
        None if on_github => {
            out.push_str(&section("GitHub", "no answer from GitHub right now", color));
        }
        None => return out,
    }
    out.push('\n');
    out
}

/// Where the checked-out branch stands against its upstream, in words.
fn upstream_words(snap: &RepoSnapshot) -> String {
    match (&snap.upstream, &snap.branch) {
        (Some(u), _) => format!("{}, {}", safe(&u.name), sync_words(u.ahead, u.behind)),
        (None, Branch::Detached(_)) => "none, HEAD is detached".to_string(),
        (None, Branch::Unborn) => "none, no commits yet".to_string(),
        (None, Branch::Named(_)) if snap.remote_url.is_some() => {
            "none, this branch was never pushed".to_string()
        }
        (None, Branch::Named(_)) => "none, the repo has no remote".to_string(),
    }
}

/// `level`, `2 ahead`, `3 behind`, `2 ahead, 3 behind`.
fn sync_words(ahead: u32, behind: u32) -> String {
    match (ahead, behind) {
        (0, 0) => "level".to_string(),
        (a, 0) => format!("{a} ahead"),
        (0, b) => format!("{b} behind"),
        (a, b) => format!("{a} ahead, {b} behind"),
    }
}

/// One local branch against its own upstream, in words.
fn branch_sync(b: &cohors_core::BranchInfo) -> String {
    if b.has_upstream {
        sync_words(b.ahead, b.behind)
    } else {
        "no upstream".to_string()
    }
}

/// `1 staged, 2 modified, 1 untracked` from the snapshot's counts.
fn worktree_words(w: &cohors_core::WorktreeStatus) -> String {
    [
        (w.conflicted, "conflicted"),
        (w.staged, "staged"),
        (w.modified, "modified"),
        (w.untracked, "untracked"),
    ]
    .iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, word)| format!("{n} {word}"))
    .collect::<Vec<_>>()
    .join(", ")
}

/// `3 hours ago`-style age, short: `3h ago`, or `just now`.
fn ago(timestamp: i64, now: i64) -> String {
    match cohors_core::time::relative(timestamp, now).as_str() {
        "now" => "just now".to_string(),
        age => format!("{age} ago"),
    }
}

/// A remote URL that is safe to print: a password or token written into it
/// (`https://user:TOKEN@host/…`) is replaced, so `show` never echoes a
/// credential. The usual `git@` user of an SSH remote is kept.
fn redact_remote(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string(); // scp-style `git@host:owner/repo`: no secret
    };
    let (authority, tail) = match rest.find('/') {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    match authority.rsplit_once('@') {
        Some((user, host)) if user != "git" => format!("{scheme}://***@{host}{tail}"),
        _ => url.to_string(),
    }
}

/// `s` as one shell word: as it is when that is safe, single-quoted otherwise.
fn shell_word(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-+@/:,=".contains(c));
    if plain {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// A list of commands and why you would run them, aligned.
fn guide_rows(steps: &[(String, &str)], width: usize, color: bool) -> String {
    let w = steps
        .iter()
        .map(|(c, _)| c.chars().count())
        .max()
        .unwrap_or(0);
    steps
        .iter()
        .map(|(cmd, why)| {
            guide_row(
                cmd,
                &clip(why, width.saturating_sub(w + 8).max(8)),
                w,
                color,
            )
        })
        .collect()
}

// ── Changed files, shared by `show` and `changes` ────────────────────────────

/// A file's two-letter git status (`git status --short`): the staged half in
/// green, the unstaged half in red, untracked dim, a conflict bold red.
fn status_code(file: &ChangedFile, color: bool) -> String {
    let code: String = format!("{:<2}", safe(&file.status))
        .chars()
        .take(2)
        .collect();
    if !color {
        return code;
    }
    if file.is_conflicted() {
        return sgr(color, "1;31", &code);
    }
    if file.is_untracked() {
        return dim(color, &code);
    }
    code.chars()
        .zip(["32", "31"])
        .map(|(c, sgr_code)| sgr(color, sgr_code, &c.to_string()))
        .collect()
}

/// One changed file: its status and path. A secret file (`.env`, key
/// material, credential stores) is named, but tagged: its contents are never
/// shown, not even in `--patch`.
fn file_row(file: &ChangedFile, indent: usize, width: usize, color: bool) -> String {
    let secret = cohors_core::is_sensitive_path(&file.path);
    let tag = "secret: contents never shown";
    let room = width
        .saturating_sub(indent + 4 + if secret { tag.len() + 2 } else { 0 })
        .max(8);
    let path = clip(&safe(&file.path), room);
    let tag = if secret {
        format!("  {}", dim(color, tag))
    } else {
        String::new()
    };
    format!(
        "{}{}  {path}{tag}\n",
        " ".repeat(indent),
        status_code(file, color)
    )
}

/// `3 files: 1 staged, 2 modified`, from the files themselves.
fn file_tally(files: &[ChangedFile]) -> String {
    let count = |keep: &dyn Fn(&ChangedFile) -> bool| files.iter().filter(|f| keep(f)).count();
    let conflicted = count(&|f| f.is_conflicted());
    let staged = count(&|f| f.is_staged());
    let unstaged = count(&|f| {
        !f.is_untracked() && !f.is_conflicted() && f.worktree_state() != ChangeState::Clean
    });
    let untracked = count(&|f| f.is_untracked());
    let parts: Vec<String> = [
        (conflicted, "conflicted"),
        (staged, "staged"),
        (unstaged, "modified"),
        (untracked, "untracked"),
    ]
    .iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, word)| format!("{n} {word}"))
    .collect();
    format!(
        "{}: {}",
        plural(files.len(), "file", "files"),
        parts.join(", ")
    )
}

/// A diff, line by line and indented, in git's colours. Tabs become spaces
/// and control characters are dropped, line by line, so the diff of a file
/// holding an escape sequence cannot drive the terminal.
fn patch_lines(patch: &str, truncated: bool, color: bool) -> String {
    let mut out = String::new();
    for raw in patch.lines() {
        let line = safe(&raw.replace('\t', "    "));
        let header = ["diff ", "index ", "+++", "---", "new file", "deleted file"]
            .iter()
            .any(|p| line.starts_with(p));
        let styled = if header {
            bold(color, &line)
        } else if line.starts_with("@@") {
            sgr(color, "36", &line)
        } else if line.starts_with('+') {
            sgr(color, "32", &line)
        } else if line.starts_with('-') {
            sgr(color, "31", &line)
        } else {
            line
        };
        out.push_str(&format!("          {styled}\n"));
    }
    if truncated {
        out.push_str(&format!(
            "          {}\n",
            dim(color, "the diff stops here: --max-bytes raises the cap")
        ));
    }
    out
}

// ── cohors changes ───────────────────────────────────────────────────────────

/// One repo's uncommitted work, as `changes` reports it.
struct ChangeSet {
    name: String,
    id: String,
    path: String,
    changes: RepoChanges,
}

/// `cohors changes` — every uncommitted file, repo by repo: the MCP `changes`
/// tool's read, with the same default (every dirty repo), the same diff cap,
/// and the same rule that a secret file's contents never leave the disk.
pub fn changes(
    cli: &Cli,
    select: Option<&str>,
    patch: bool,
    max_bytes: usize,
    json: bool,
) -> Result<()> {
    let human = super::wants_report(json);
    let color = wants_color(human);
    let selector = selector_or(
        select,
        Selector {
            dirty: true,
            ..Default::default()
        },
    )?;
    let scanner = crate::scan::from_cli(cli)?;
    let snapshots = scanner.scan();
    let selected = resolve_in_order(&snapshots, &selector, now_secs());
    let max_bytes = max_bytes.max(1);

    // Repo by repo, in parallel (the order is kept). A repo with no local path
    // or nothing uncommitted is left out, as the MCP tool leaves it out.
    let sets: Vec<ChangeSet> = selected
        .par_iter()
        .filter_map(|snap| {
            let path = snap.path.as_deref()?;
            let changes = cohors_fleet::repo_changes(path, patch, max_bytes);
            (!changes.files.is_empty()).then(|| ChangeSet {
                name: snap.name.clone(),
                id: snap.id.0.clone(),
                path: path.to_string(),
                changes,
            })
        })
        .collect();

    if !human {
        let repos: Vec<Value> = sets.iter().map(change_set_json).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "repos": repos }))?
        );
        return Ok(());
    }
    print!(
        "{}",
        changes_report(&sets, select, patch, term_width(), color)
    );
    Ok(())
}

/// One repo in the MCP `changes` shape.
fn change_set_json(set: &ChangeSet) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("repo".into(), json!(set.name));
    obj.insert("id".into(), json!(set.id));
    obj.insert(
        "files".into(),
        serde_json::to_value(&set.changes.files).unwrap_or(Value::Null),
    );
    if let Some(patch) = &set.changes.patch {
        obj.insert("patch".into(), json!(patch));
        obj.insert("truncated".into(), json!(set.changes.truncated));
    }
    Value::Object(obj)
}

/// The human `cohors changes` report. Pure, so it is unit-testable.
fn changes_report(
    sets: &[ChangeSet],
    select: Option<&str>,
    patch: bool,
    width: usize,
    color: bool,
) -> String {
    let files: usize = sets.iter().map(|s| s.changes.files.len()).sum();
    let tally = if sets.is_empty() {
        "nothing uncommitted".to_string()
    } else {
        format!(
            "{} in {}",
            plural(files, "file", "files"),
            plural(sets.len(), "repository", "repositories")
        )
    };
    let mut out = masthead("changes: uncommitted work across your repos", &tally, color);
    if sets.is_empty() {
        let why = match select {
            Some(q) => format!("no repo matching --select {} has uncommitted work", safe(q)),
            None => "every repository is committed".to_string(),
        };
        out.push_str(&section("all clean", &why, color));
        out.push('\n');
        return out;
    }

    out.push_str(&section(
        "uncommitted",
        "XY: staged, then unstaged; ?? untracked",
        color,
    ));
    let name_w = sets
        .iter()
        .map(|s| safe(&s.name).chars().count())
        .max()
        .unwrap_or(0)
        .min(NAME_W);
    for set in sets {
        let why = format!(
            "{}  {}",
            file_tally(&set.changes.files),
            abbrev_home(&safe(&set.path))
        );
        out.push_str(&guide_row(
            &clip(&safe(&set.name), name_w),
            &clip(&why, width.saturating_sub(name_w + 8).max(8)),
            name_w,
            color,
        ));
        for file in &set.changes.files {
            out.push_str(&file_row(file, 8, width, color));
        }
        if let Some(diff) = &set.changes.patch {
            out.push_str(&patch_lines(diff, set.changes.truncated, color));
        }
    }
    out.push('\n');

    let mut steps: Vec<(String, &str)> = Vec::new();
    if !patch {
        steps.push((
            "cohors changes --patch".to_string(),
            "the diff too; secret files stay hidden",
        ));
    }
    steps.push((
        format!("cohors show {}", shell_word(&safe(&sets[0].name))),
        "one repo in full",
    ));
    out.push_str(&section("next", "go further", color));
    out.push_str(&guide_rows(&steps, width, color));
    out.push('\n');
    out
}

// ── cohors search ────────────────────────────────────────────────────────────

/// One repo's matches, as `search` reports them.
struct SearchGroup {
    name: String,
    id: String,
    path: String,
    hits: Vec<ContentHit>,
}

/// `cohors search <text>` — a fixed string across every selected repo: the MCP
/// `search` tool's content search, with the same cap and the same rule that
/// secret files are never opened.
pub fn search(
    cli: &Cli,
    query: &str,
    select: Option<&str>,
    max_results: usize,
    json: bool,
) -> Result<()> {
    if query.is_empty() {
        eprintln!("cohors: search needs some text to look for, e.g. cohors search \"TODO\"");
        return Err(ExitCode(2).into());
    }
    let human = super::wants_report(json);
    let color = wants_color(human);
    let selector = selector_or(
        select,
        Selector {
            all: true,
            ..Default::default()
        },
    )?;
    let scanner = crate::scan::from_cli(cli)?;
    let snapshots = scanner.scan();
    let selected = resolve_in_order(&snapshots, &selector, now_secs());
    let max = max_results.max(1);

    // Each repo is asked for one hit more than the cap, so whether anything
    // was dropped is known; the first `max` are kept, in fleet order.
    let found: Vec<(&RepoSnapshot, Vec<ContentHit>)> = selected
        .par_iter()
        .map(|snap| {
            let hits = snap
                .path
                .as_deref()
                .map(|p| cohors_fleet::search_content(p, query, max + 1))
                .unwrap_or_default();
            (*snap, hits)
        })
        .collect();
    let (groups, truncated) = cap_hits(found, max);

    if !human {
        let hits: Vec<Value> = groups
            .iter()
            .flat_map(|g| {
                g.hits.iter().map(move |h| {
                    json!({
                        "repo": g.id, "name": g.name, "path": h.path.as_str(),
                        "line": h.line, "text": h.text
                    })
                })
            })
            .collect();
        let out = json!({ "hits": hits, "truncated": truncated });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    print!(
        "{}",
        search_report(
            query,
            &groups,
            truncated,
            max,
            selected.len(),
            term_width(),
            color
        )
    );
    Ok(())
}

/// Keep the first `max` hits in fleet order, grouped by repo, and say whether
/// any were dropped (more were found than were kept).
fn cap_hits(found: Vec<(&RepoSnapshot, Vec<ContentHit>)>, max: usize) -> (Vec<SearchGroup>, bool) {
    let total: usize = found.iter().map(|(_, hits)| hits.len()).sum();
    let mut left = max;
    let groups = found
        .into_iter()
        .filter_map(|(snap, mut hits)| {
            hits.truncate(left);
            left -= hits.len();
            (!hits.is_empty()).then(|| SearchGroup {
                name: snap.name.clone(),
                id: snap.id.0.clone(),
                path: snap
                    .path
                    .as_ref()
                    .map(|p| p.to_string())
                    .unwrap_or_default(),
                hits,
            })
        })
        .collect();
    (groups, total > max)
}

/// `text` with the first `query` in it picked out.
fn highlight(text: &str, query: &str, color: bool) -> String {
    match text.find(query) {
        Some(at) if color && !query.is_empty() => {
            let (before, rest) = text.split_at(at);
            let (hit, after) = rest.split_at(query.len());
            format!("{before}{}{after}", brand(color, hit))
        }
        _ => text.to_string(),
    }
}

/// The human `cohors search` report. Pure, so it is unit-testable.
fn search_report(
    query: &str,
    groups: &[SearchGroup],
    truncated: bool,
    max: usize,
    searched: usize,
    width: usize,
    color: bool,
) -> String {
    let total: usize = groups.iter().map(|g| g.hits.len()).sum();
    let mut out = masthead(
        &format!("search: \"{}\" across your repos", safe(query)),
        &format!(
            "{} in {}, {} searched",
            plural(total, "match", "matches"),
            plural(groups.len(), "repository", "repositories"),
            searched
        ),
        color,
    );
    if groups.is_empty() {
        out.push_str(&section(
            "no matches",
            "exact text, case-sensitive; secret files skipped",
            color,
        ));
        out.push('\n');
        return out;
    }

    out.push_str(&section(
        "matches",
        "path:line, then the line; secret files skipped",
        color,
    ));
    let name_w = groups
        .iter()
        .map(|g| safe(&g.name).chars().count())
        .max()
        .unwrap_or(0)
        .min(NAME_W);
    for group in groups {
        let why = format!(
            "{}  {}",
            plural(group.hits.len(), "match", "matches"),
            abbrev_home(&safe(&group.path))
        );
        out.push_str(&guide_row(
            &clip(&safe(&group.name), name_w),
            &clip(&why, width.saturating_sub(name_w + 8).max(8)),
            name_w,
            color,
        ));
        let locs: Vec<String> = group
            .hits
            .iter()
            .map(|h| format!("{}:{}", safe(h.path.as_str()), h.line))
            .collect();
        let loc_w = locs
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0)
            .min(width / 2);
        let room = width.saturating_sub(8 + loc_w + 2).max(8);
        for (hit, loc) in group.hits.iter().zip(&locs) {
            let line = safe(&hit.text.replace('\t', " "));
            out.push_str(&format!(
                "        {}  {}\n",
                dim(color, &format!("{:<loc_w$}", clip(loc, loc_w))),
                highlight(&clip(line.trim(), room), query, color)
            ));
        }
    }
    if truncated {
        out.push('\n');
        let note = format!(
            "stopped at {}: narrow it with --select, or raise --max-results",
            plural(max, "match", "matches")
        );
        out.push_str(&format!(
            "  {}\n",
            dim(color, &clip(&note, width.saturating_sub(2)))
        ));
    }
    out.push('\n');
    out
}

// ── cohors prs ───────────────────────────────────────────────────────────────

/// One repo on GitHub, as `prs` reports it.
struct PrRow {
    name: String,
    id: String,
    remote: RemoteInfo,
}

/// `cohors prs` — open pull requests and CI per repo: the MCP `list_prs` and
/// `ci_status` read (the fleet enriched from GitHub, then selected, so
/// `--select ci:failing` works). With no token it says so and lists nothing.
pub fn prs(cli: &Cli, select: Option<&str>, json: bool) -> Result<()> {
    let human = super::wants_report(json);
    let color = wants_color(human);
    let selector = selector_or(
        select,
        Selector {
            all: true,
            ..Default::default()
        },
    )?;
    let scanner = crate::scan::from_cli(cli)?;
    let Some(token) = scanner.github_token() else {
        if human {
            print!("{}", no_token_report(term_width(), color));
        } else {
            let out = json!({ "repos": [], "github_token": false, "note": NO_TOKEN_NOTE });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        return Ok(());
    };

    // This asks GitHub, which takes a while on a big fleet: at a terminal,
    // say what is happening (only reading) and count the answers on stderr,
    // rather than sitting silent.
    let progress = human && std::io::stderr().is_terminal();
    if progress {
        eprint!("{}", progress_line("Reading your repositories…", color));
    }
    let snapshots = scanner.scan();
    // Only the selected repositories are asked about.
    let mut selected: Vec<RepoSnapshot> = resolve_in_order(&snapshots, &selector, now_secs())
        .into_iter()
        .cloned()
        .collect();
    let asking = selected
        .iter()
        .filter(|s| cohors_fleet::on_github(s))
        .count();
    let show = |done: usize, total: usize| {
        if progress {
            let what = format!(
                "Asking GitHub about {} (read-only; Ctrl-C stops it)…  {done} of {total}",
                plural_repos(asking)
            );
            eprint!("{}", progress_line(&what, color));
        }
    };
    cohors_fleet::enrich_with_progress(&mut selected, Some(&token), &show);
    if progress {
        eprint!("\r\x1b[2K");
    }
    let rows: Vec<PrRow> = selected
        .iter()
        .filter_map(|s| {
            s.remote.clone().map(|remote| PrRow {
                name: s.name.clone(),
                id: s.id.0.clone(),
                remote,
            })
        })
        .collect();
    // Selected repos with no GitHub data: no GitHub remote, or GitHub did not
    // answer for them. Counted, so "18 repos but 14 rows" is explained.
    let excluded = selected.len() - rows.len();

    if !human {
        let repos: Vec<Value> = rows.iter().map(pr_json).collect();
        let mut out = json!({ "repos": repos, "excluded": excluded, "github_token": true });
        if excluded > 0 {
            out["note"] = json!(format!(
                "{excluded} selected repo(s) have no GitHub remote and were omitted from the results."
            ));
        }
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    print!("{}", prs_report(&rows, excluded, term_width(), color));
    Ok(())
}

/// `N repository` / `N repositories`.
fn plural_repos(n: usize) -> String {
    format!("{n} {}", if n == 1 { "repository" } else { "repositories" })
}

/// A line that says what a slow command is doing, written over itself on
/// stderr (`\r`, then clear the line), dim when colour is on.
fn progress_line(text: &str, color: bool) -> String {
    let text = if color {
        format!("\x1b[2m{text}\x1b[0m")
    } else {
        text.to_string()
    };
    format!("\r\x1b[2K{text}")
}

/// One repo in the shape of the MCP `list_prs` and `ci_status` rows, merged.
fn pr_json(row: &PrRow) -> Value {
    let r = &row.remote;
    let mut obj = serde_json::Map::new();
    obj.insert("repo".into(), json!(row.id));
    obj.insert("name".into(), json!(row.name));
    obj.insert(
        "ci".into(),
        serde_json::to_value(r.ci).unwrap_or(Value::Null),
    );
    if !r.ci_by_branch.is_empty() {
        obj.insert(
            "ci_by_branch".into(),
            serde_json::to_value(&r.ci_by_branch).unwrap_or(Value::Null),
        );
    }
    obj.insert("open_prs".into(), json!(r.open_prs));
    obj.insert(
        "open_prs_by_base".into(),
        serde_json::to_value(&r.open_prs_by_base).unwrap_or(Value::Null),
    );
    obj.insert("awaiting_review".into(), json!(r.prs_awaiting_review));
    Value::Object(obj)
}

/// A CI status in words.
fn ci_word(ci: CiStatus) -> &'static str {
    match ci {
        CiStatus::Passing => "passing",
        CiStatus::Failing => "failing",
        CiStatus::Pending => "pending",
        CiStatus::None => "no checks",
    }
}

/// A CI status as a fixed-width cell: the word carries it, colour helps.
fn ci_cell(ci: CiStatus, color: bool) -> String {
    let cell = format!("{:<9}", ci_word(ci));
    match ci {
        CiStatus::Passing => sgr(color, "32", &cell),
        CiStatus::Failing => sgr(color, "1;31", &cell),
        CiStatus::Pending => sgr(color, "33", &cell),
        CiStatus::None => dim(color, &cell),
    }
}

/// `2 open PRs (main 1, dev 1), 1 awaiting your review`.
fn pr_words(r: &RemoteInfo) -> String {
    let mut words = if r.open_prs == 0 {
        "no open PRs".to_string()
    } else {
        plural(r.open_prs as usize, "open PR", "open PRs")
    };
    if r.open_prs > 0 && !r.open_prs_by_base.is_empty() {
        let mut bases: Vec<(&String, &u32)> = r.open_prs_by_base.iter().collect();
        bases.sort();
        let bases: Vec<String> = bases
            .iter()
            .map(|(base, n)| format!("{} {n}", safe(base)))
            .collect();
        words.push_str(&format!(" ({})", bases.join(", ")));
    }
    if r.prs_awaiting_review > 0 {
        words.push_str(&format!(", {} awaiting your review", r.prs_awaiting_review));
    }
    words
}

/// The human `cohors prs` report. Pure, so it is unit-testable.
fn prs_report(rows: &[PrRow], excluded: usize, width: usize, color: bool) -> String {
    let open: usize = rows.iter().map(|r| r.remote.open_prs as usize).sum();
    let failing = rows
        .iter()
        .filter(|r| r.remote.ci == CiStatus::Failing)
        .count();
    let mut out = masthead(
        "prs: open pull requests and CI across your repos",
        &format!(
            "{} on GitHub, {}, {failing} failing",
            plural(rows.len(), "repository", "repositories"),
            plural(open, "open PR", "open PRs")
        ),
        color,
    );
    if rows.is_empty() {
        out.push_str(&section(
            "nothing on GitHub",
            "no selected repo has GitHub data",
            color,
        ));
    } else {
        out.push_str(&section(
            "pull requests and checks",
            "CI on the default branch, then open PRs",
            color,
        ));
        let name_w = rows
            .iter()
            .map(|r| safe(&r.name).chars().count())
            .max()
            .unwrap_or(0)
            .min(NAME_W);
        for row in rows {
            let r = &row.remote;
            let name = format!("{:<name_w$}", clip(&safe(&row.name), name_w));
            out.push_str(&format!(
                "    {}  {}  {}\n",
                bold(color, &name),
                ci_cell(r.ci, color),
                clip(
                    &pr_words(r),
                    width.saturating_sub(4 + name_w + 2 + 9 + 2).max(8)
                )
            ));
            if !r.ci_by_branch.is_empty() {
                let mut branches: Vec<(&String, &CiStatus)> = r.ci_by_branch.iter().collect();
                branches.sort_by(|a, b| a.0.cmp(b.0));
                let by_branch: Vec<String> = branches
                    .iter()
                    .map(|(branch, ci)| format!("{} {}", safe(branch), ci_word(**ci)))
                    .collect();
                let line = format!("by branch: {}", by_branch.join(", "));
                out.push_str(&format!(
                    "    {}  {}\n",
                    " ".repeat(name_w),
                    dim(color, &clip(&line, width.saturating_sub(6 + name_w).max(8)))
                ));
            }
        }
    }
    out.push('\n');
    if excluded > 0 {
        let note = format!(
            "{} with no GitHub data left out (no GitHub remote, or no answer)",
            plural(excluded, "selected repository", "selected repositories")
        );
        out.push_str(&format!(
            "  {}\n\n",
            dim(color, &clip(&note, width.saturating_sub(2)))
        ));
    }
    if failing > 0 {
        out.push_str(&section("next", "go further", color));
        let steps = [(
            "cohors scan --select ci:failing".to_string(),
            "the repos whose checks fail",
        )];
        out.push_str(&guide_rows(&steps, width, color));
        out.push('\n');
    }
    out
}

/// What `prs` prints at a terminal when there is no GitHub token: the MCP
/// note, and nothing listed.
fn no_token_report(width: usize, color: bool) -> String {
    let mut out = masthead(
        "prs: open pull requests and CI across your repos",
        "GitHub is not connected",
        color,
    );
    out.push_str(&section("no GitHub token", "nothing to list", color));
    for line in wrap(NO_TOKEN_NOTE, width.saturating_sub(6)) {
        out.push_str(&format!("    {line}\n"));
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cohors_core::{
        BranchInfo, CommitMeta, PullRequest, RepoId, TagInfo, Upstream, WorktreeStatus,
    };

    const NOW: i64 = 1_700_000_000;

    /// A readable repo on `main` at `path`, named `name`.
    fn snap(name: &str, path: &str) -> RepoSnapshot {
        let mut s = RepoSnapshot::errored(RepoId(path.into()), name, Some(path.into()), "");
        s.error = None;
        s.branch = Branch::Named("main".into());
        s
    }

    fn fleet() -> Vec<RepoSnapshot> {
        vec![
            snap("payments", "/code/payments"),
            snap("payroll", "/code/payroll"),
            snap("web", "/code/web"),
            // Shown under an alias; its directory is `legacy-api`.
            snap("api", "/code/legacy-api"),
        ]
    }

    /// Every line fits in `width` columns (the reports are built colourless
    /// here, so characters are columns).
    fn fits(report: &str, width: usize) -> bool {
        report.lines().all(|l| l.chars().count() <= width)
    }

    fn file(status: &str, path: &str) -> ChangedFile {
        ChangedFile {
            status: status.to_string(),
            path: path.to_string(),
        }
    }

    #[test]
    fn duplicate_names_are_ambiguous_and_listed_with_their_paths() {
        let mut repos = fleet();
        repos.push(snap("web", "/other/web"));
        let Resolved::Many(found) = resolve_repo(&repos, "web", None) else {
            panic!("two repos named web");
        };
        let msg = ambiguous_message("web", &found);
        assert!(msg.contains("matches 2 repositories"), "{msg}");
        assert!(
            msg.contains("/code/web") && msg.contains("/other/web"),
            "{msg}"
        );
        assert!(msg.contains("give its path"), "{msg}");
    }

    #[test]
    fn a_miss_says_so_and_offers_the_close_names() {
        let repos = fleet();
        let close = close_names(&repos, "paymnets");
        let msg = missing_message("paymnets", &close, &[], false);
        assert!(msg.contains("no repository matches `paymnets`"), "{msg}");
        assert!(msg.contains("did you mean: payments"), "{msg}");
        let msg = missing_message("x", &[], &[], false);
        assert!(msg.contains("cohors scan"), "{msg}");
        let msg = missing_message("x", &[], &["/nowhere".into()], true);
        assert!(
            msg.contains("no repositories found under /nowhere"),
            "{msg}"
        );
    }

    #[test]
    fn a_remote_credential_is_never_shown() {
        assert_eq!(
            redact_remote("https://x-access-token:ghp_SECRET@github.com/o/r.git"),
            "https://***@github.com/o/r.git"
        );
        assert_eq!(
            redact_remote("https://ghp_SECRET@github.com/o/r"),
            "https://***@github.com/o/r"
        );
        assert_eq!(
            redact_remote("ssh://git@github.com/o/r.git"),
            "ssh://git@github.com/o/r.git"
        );
        assert_eq!(
            redact_remote("git@github.com:o/r.git"),
            "git@github.com:o/r.git"
        );
        assert_eq!(
            redact_remote("https://github.com/o/r"),
            "https://github.com/o/r"
        );
    }

    #[test]
    fn shell_words_are_quoted_only_when_they_need_it() {
        assert_eq!(shell_word("payments"), "payments");
        assert_eq!(shell_word("my repo"), "'my repo'");
        assert_eq!(shell_word("it's"), "'it'\\''s'");
    }

    /// A repo with something in every section `show` draws.
    fn busy_repo() -> (RepoSnapshot, RepoDetail, RemoteDetail) {
        let mut s = snap("payments", "/code/payments");
        s.upstream = Some(Upstream {
            name: "origin/main".into(),
            ahead: 2,
            behind: 1,
        });
        s.default_branch = Some("main".into());
        s.remote_url = Some("https://user:ghp_SECRET@github.com/acme/payments.git".into());
        s.last_fetch = Some(NOW - 3 * 86_400);
        s.latest_tag = Some(TagInfo {
            name: "v1.2.0".into(),
            commits_since: 5,
        });
        s.worktree = WorktreeStatus {
            staged: 1,
            modified: 1,
            untracked: 1,
            conflicted: 0,
        };
        s.stash_count = 1;
        let commit = CommitMeta {
            short_id: "a1b2c3d".into(),
            author: "Dev".into(),
            timestamp: NOW - 7_200,
            summary: "fix: retry \u{1b}[31mcharge\u{7}".into(),
        };
        s.last_commit = Some(commit.clone());
        let detail = RepoDetail {
            current_branch: Some("main".into()),
            recent_commits: vec![commit],
            changed_files: vec![
                file("A ", "src/new.rs"),
                file(" M", "src/charge.rs"),
                file("??", ".env"),
            ],
            branches: vec![
                BranchInfo {
                    name: "main".into(),
                    ahead: 2,
                    behind: 1,
                    has_upstream: true,
                    is_current: true,
                    last_commit: None,
                },
                BranchInfo {
                    name: "spike/webhooks".into(),
                    ..BranchInfo::default()
                },
            ],
            stashes: vec!["WIP on main: tweaks".into()],
        };
        let remote = RemoteDetail {
            prs: vec![PullRequest {
                number: 42,
                title: "feat: idempotency keys".into(),
                author: "ana".into(),
                draft: true,
                branch: "feat/idem".into(),
                url: "https://github.com/acme/payments/pull/42".into(),
                requested_reviewers: vec!["bo".into()],
            }],
            open_issues: 3,
            latest_release: Some("v1.2.0".into()),
            latest_run: Some(CiStatus::Failing),
            ..RemoteDetail::default()
        };
        (s, detail, remote)
    }

    #[test]
    fn show_draws_every_section_of_the_detail_pane() {
        let (s, detail, remote) = busy_repo();
        let r = show_report(&s, Some(&detail), Some(&remote), true, NOW, 100, false);
        for want in [
            "payments on main",
            "/code/payments",
            "Attention",
            "diverged, 2 ahead and 1 behind",
            "origin/main, 2 ahead, 1 behind",
            "main (the default branch)",
            "3d ago",
            "v1.2.0, 5 commits since",
            "1 staged, 1 modified, 1 untracked",
            "A   src/new.rs",
            " M  src/charge.rs",
            "a1b2c3d",
            "2 branches; * is checked out",
            "* main",
            "spike/webhooks",
            "no upstream",
            "stash@{0}  WIP on main: tweaks",
            "1 open PR, 3 open issues",
            "#42",
            "feat: idempotency keys",
            "ana, draft, waiting on bo",
            "cd \"$(cohors path payments)\"",
            "cohors changes --select name:payments --patch",
        ] {
            assert!(r.contains(want), "missing {want:?} in:\n{r}");
        }
        // The secret file is named, never opened.
        assert!(r.contains(".env  secret: contents never shown"), "{r}");
        // A token in the remote URL is not echoed.
        assert!(
            !r.contains("ghp_SECRET") && r.contains("https://***@github.com"),
            "{r}"
        );
        // An escape sequence in a commit subject never reaches the terminal.
        assert!(!r.contains('\u{1b}') && !r.contains('\u{7}'), "{r}");
        assert!(r.contains("fix: retry [31mcharge"), "{r}");
        assert!(fits(&r, 100), "{r}");
    }

    #[test]
    fn show_says_how_to_connect_github_without_a_token() {
        let (mut s, detail, _) = busy_repo();
        s.remote_url = Some("git@github.com:acme/payments.git".into());
        let r = show_report(&s, Some(&detail), None, false, NOW, 100, false);
        assert!(r.contains("not connected"), "{r}");
        assert!(r.contains("gh auth login"), "{r}");
        // A repo that is not on GitHub has no GitHub section at all.
        s.remote_url = Some("git@gitlab.com:acme/payments.git".into());
        let r = show_report(&s, Some(&detail), None, false, NOW, 100, false);
        assert!(!r.contains("GitHub") && !r.contains("gh auth login"), "{r}");
    }

    #[test]
    fn show_of_an_unreadable_repo_says_why_and_stops() {
        let mut s = snap("broken", "/code/broken");
        s.error = Some("not a git repository".into());
        let r = show_report(&s, None, None, false, NOW, 80, false);
        assert!(
            r.contains("Could not be read") && r.contains("not a git repository"),
            "{r}"
        );
        assert!(!r.contains("Where it stands"), "{r}");
    }

    #[test]
    fn show_json_is_the_mcp_get_repo_payload() {
        let (s, detail, remote) = busy_repo();
        let v = show_json(&s, Some(&detail), Some(&remote), NOW);
        assert_eq!(v["name"], "payments");
        assert_eq!(v["assessment"]["needs_attention"], true);
        assert!(
            v["assessment"]["reasons"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
        );
        assert_eq!(v["detail"]["current_branch"], "main");
        assert_eq!(v["remote_detail"]["prs"][0]["number"], 42);
        let v = show_json(&s, None, None, NOW);
        assert!(v.get("detail").is_none() && v.get("remote_detail").is_none());
    }

    fn change_set(name: &str, files: Vec<ChangedFile>, patch: Option<&str>) -> ChangeSet {
        ChangeSet {
            name: name.into(),
            id: format!("/code/{name}"),
            path: format!("/code/{name}"),
            changes: RepoChanges {
                files,
                patch: patch.map(str::to_string),
                truncated: false,
            },
        }
    }

    #[test]
    fn changes_lists_files_by_repo_and_tags_secrets() {
        let sets = vec![
            change_set(
                "payments",
                vec![
                    file("MM", "src/charge.rs"),
                    file("??", ".env.local"),
                    file("UU", "src/\u{1b}]0;pwned\u{7}.rs"),
                ],
                None,
            ),
            change_set("web", vec![file(" D", "old.css")], None),
        ];
        let r = changes_report(&sets, None, false, 100, false);
        for want in [
            "4 files in 2 repositories",
            "payments",
            "3 files: 1 conflicted, 1 staged, 1 modified, 1 untracked",
            "MM  src/charge.rs",
            "??  .env.local  secret: contents never shown",
            "1 file: 1 modified",
            " D  old.css",
            "cohors changes --patch",
            "cohors show payments",
        ] {
            assert!(r.contains(want), "missing {want:?} in:\n{r}");
        }
        assert!(!r.contains('\u{1b}') && !r.contains('\u{7}'), "{r}");
        assert!(fits(&r, 100), "{r}");
    }

    #[test]
    fn changes_prints_the_diff_without_control_characters() {
        let patch = "diff --git a/a.txt b/a.txt\n@@ -1 +1 @@\n-one\n+\tTWO\u{1b}[2J\n";
        let sets = vec![change_set(
            "payments",
            vec![file(" M", "a.txt")],
            Some(patch),
        )];
        let r = changes_report(&sets, None, true, 100, false);
        assert!(
            r.contains("+    TWO[2J"),
            "tabs become spaces, escapes go: {r}"
        );
        assert!(r.contains("-one") && r.contains("@@ -1 +1 @@"), "{r}");
        assert!(!r.contains('\u{1b}'), "{r}");
        assert!(
            !r.contains("cohors changes --patch"),
            "already showing it: {r}"
        );
        let json = change_set_json(&sets[0]);
        assert_eq!(json["repo"], "payments");
        assert_eq!(json["truncated"], false);
        assert!(json["patch"].as_str().is_some_and(|p| p.contains("TWO")));
    }

    #[test]
    fn changes_with_nothing_uncommitted_says_so() {
        let r = changes_report(&[], Some("name:pay*"), false, 80, false);
        assert!(r.contains("nothing uncommitted"), "{r}");
        assert!(
            r.contains("no repo matching --select name:pay* has uncommitted work"),
            "{r}"
        );
    }

    fn hit(path: &str, line: u32, text: &str) -> ContentHit {
        ContentHit {
            path: path.into(),
            line,
            text: text.into(),
        }
    }

    #[test]
    fn search_groups_matches_by_repo_with_path_and_line() {
        let groups = vec![
            SearchGroup {
                name: "payments".into(),
                id: "/code/payments".into(),
                path: "/code/payments".into(),
                hits: vec![
                    hit("src/charge.rs", 12, "\t// TODO retry \u{1b}[31m"),
                    hit("README.md", 3, "TODO: docs"),
                ],
            },
            SearchGroup {
                name: "web".into(),
                id: "/code/web".into(),
                path: "/code/web".into(),
                hits: vec![hit("app.js", 7, "x = 1 // TODO")],
            },
        ];
        let r = search_report("TODO", &groups, true, 3, 4, 100, false);
        for want in [
            "search: \"TODO\" across your repos",
            "3 matches in 2 repositories, 4 searched",
            "payments",
            "2 matches",
            "src/charge.rs:12  // TODO retry [31m",
            "README.md:3",
            "app.js:7",
            "stopped at 3 matches",
        ] {
            assert!(r.contains(want), "missing {want:?} in:\n{r}");
        }
        assert!(!r.contains('\u{1b}'), "{r}");
        assert!(fits(&r, 100), "{r}");

        let r = search_report("nope", &[], false, 200, 4, 80, false);
        assert!(r.contains("0 matches in 0 repositories, 4 searched"), "{r}");
        assert!(r.contains("No matches"), "{r}");
    }

    #[test]
    fn the_hit_cap_keeps_fleet_order_and_reports_what_it_dropped() {
        let a = snap("a", "/code/a");
        let b = snap("b", "/code/b");
        let found = vec![
            (&a, vec![hit("x", 1, "q"), hit("y", 2, "q")]),
            (&b, vec![hit("z", 3, "q")]),
        ];
        let (groups, truncated) = cap_hits(found.clone(), 2);
        assert!(truncated);
        assert_eq!(groups.len(), 1, "b's hit fell past the cap");
        assert_eq!(groups[0].hits.len(), 2);
        let (groups, truncated) = cap_hits(found, 3);
        assert!(!truncated);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn a_match_is_picked_out_only_in_colour() {
        assert_eq!(highlight("a TODO b", "TODO", false), "a TODO b");
        let lit = highlight("a TODO b", "TODO", true);
        assert!(
            lit.starts_with("a \u{1b}[") && lit.ends_with(" b"),
            "{lit:?}"
        );
        assert_eq!(highlight("a b", "TODO", true), "a b");
    }

    fn remote(ci: CiStatus, open_prs: u32) -> RemoteInfo {
        RemoteInfo {
            host: "github.com".into(),
            owner: "acme".into(),
            repo: "r".into(),
            default_branch: "main".into(),
            open_prs,
            open_prs_by_base: HashMap::from([("main".to_string(), open_prs)]),
            ci_by_branch: HashMap::new(),
            prs_awaiting_review: 0,
            ci,
            description: None,
            topics: Vec::new(),
            stars: 0,
            forks: 0,
            watchers: 0,
        }
    }

    #[test]
    fn prs_lists_checks_and_open_prs_per_repo() {
        let mut failing = remote(CiStatus::Failing, 2);
        failing.prs_awaiting_review = 1;
        failing.ci_by_branch = HashMap::from([
            ("main".to_string(), CiStatus::Failing),
            ("dev".to_string(), CiStatus::Passing),
        ]);
        let rows = vec![
            PrRow {
                name: "payments".into(),
                id: "/code/payments".into(),
                remote: failing,
            },
            PrRow {
                name: "web".into(),
                id: "/code/web".into(),
                remote: remote(CiStatus::Passing, 0),
            },
        ];
        let r = prs_report(&rows, 3, 100, false);
        for want in [
            "2 repositories on GitHub, 2 open PRs, 1 failing",
            "payments  failing    2 open PRs (main 2), 1 awaiting your review",
            "by branch: dev passing, main failing",
            "web       passing    no open PRs",
            "3 selected repositories with no GitHub data left out",
            "cohors scan --select ci:failing",
        ] {
            assert!(r.contains(want), "missing {want:?} in:\n{r}");
        }
        assert!(fits(&r, 100), "{r}");

        let json = pr_json(&rows[0]);
        assert_eq!(json["ci"], "failing");
        assert_eq!(json["open_prs"], 2);
        assert_eq!(json["awaiting_review"], 1);
        assert_eq!(json["ci_by_branch"]["dev"], "passing");
        assert!(pr_json(&rows[1]).get("ci_by_branch").is_none());
    }

    #[test]
    fn prs_without_a_token_says_so_and_lists_nothing() {
        let r = no_token_report(200, false);
        assert!(r.contains(NO_TOKEN_NOTE), "{r}");
        assert!(r.contains("nothing to list"), "{r}");
    }

    #[test]
    fn a_read_with_no_selector_covers_its_default() {
        let dirty = Selector {
            dirty: true,
            ..Default::default()
        };
        assert!(selector_or(None, dirty.clone()).unwrap().dirty);
        assert!(selector_or(Some("{}"), dirty.clone()).unwrap().dirty);
        let named = selector_or(Some("name:pay*"), dirty).unwrap();
        assert!(!named.dirty && named.name.as_deref() == Some("pay*"));
        assert!(selector_or(Some("no-such-thing"), Selector::default()).is_err());
    }
}
