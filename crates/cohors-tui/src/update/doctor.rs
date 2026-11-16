//! `cohors doctor`: which cohors the person ran and whether it handed control
//! to cohors' own install, how each copy is installed, whether it keeps itself
//! up to date and why, every cohors on the PATH, the managed versions, the
//! last update attempt, and what is wrong. Read-only: it writes nothing and
//! installs nothing, and it never sets cohors' own install up. (It runs each
//! cohors it finds with `--version`, with updates switched off.)

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use cohors_config::UpdatesMode;
use cohors_core::update::{
    Auto, Channel, InstallFacts, PathStanding, auto_update, detect_channel, hand_off_to,
    handoff_blocked, path_standing,
};

use super::handoff::{self, Record};
use super::managed::{self, Attempt, EXE, Layout};
use super::{Memo, Seen, cargo_installed, ctx, installer_owns, now_secs, switches_given};

/// Print the report.
pub fn run(configured: UpdatesMode) -> Result<()> {
    let ctx = ctx().ok_or_else(|| anyhow!("could not find your home folder"))?;
    let layout = &ctx.layout;
    let now = now_secs();
    let switches = switches_given(configured, |name| std::env::var(name).ok());
    let env = handoff::Env::from_process(configured);
    let memo = Memo::load(&layout.memo());
    let record = Record::load(layout);
    let on_path: Vec<Found> = holders()
        .iter()
        .map(|path| classify(layout, path))
        .collect();
    let managed = managed::installed_versions(layout);
    let active = managed::active_version(layout);
    let last = managed::last_attempt(layout);
    let running = ctx
        .running
        .as_deref()
        .map(|p| layout.show(p))
        .unwrap_or_else(|| "an unknown path".into());

    // The copy the person ran, and what happened to the hand-off.
    let from = handoff::handed_from();
    let ran = match &from {
        Some((path, version, channel)) => format!(
            "{} ({channel} {version}), which handed control to this copy",
            layout.show(Path::new(path))
        ),
        None => format!("{running} ({})", ctx.installed_as.label()),
    };
    let facts = handoff::facts(ctx, &env, active.as_deref());
    let handoff_line = match (&from, handoff_blocked(&facts), hand_off_to(&facts)) {
        (Some((_, version, _)), ..) => format!("handed off from {version} to {}", ctx.current),
        (None, Some(why), _) => format!("none ({why})"),
        (None, None, Some(version)) => {
            format!("to {version}, which failed its check, so this copy ran (see Problems)")
        }
        (None, None, None) => match &active {
            Some(active) => {
                format!("none (cohors' own install, {active}, is not newer than this copy)")
            }
            None => "none yet: cohors' own install is not set up (the next run sets it up)".into(),
        },
    };
    let set_up = record
        .set_up
        .as_ref()
        .map(|s| format!("from {} {}, {}", s.from, s.version, ago(now, s.at)));

    let mut problems = Vec::new();
    let mut notes = Vec::new();
    if let Some(fallback) = &record.fallback {
        problems.push(format!(
            "The last hand-off, to {}, did not happen ({}): {}",
            fallback.version,
            ago(now, fallback.at),
            fallback.reason
        ));
    }
    if let Some(attempt) = last.as_ref().filter(|a| !a.ok) {
        problems.push(format!(
            "The last update, to {}, failed: {}. To update by hand, run: {}",
            attempt.to,
            attempt
                .reason
                .as_deref()
                .unwrap_or("no reason was recorded"),
            ctx.channel.manual_command()
        ));
    }
    let seen = memo.sources.get(&ctx.lookup.location).cloned();
    if let Some(error) = seen.as_ref().and_then(|s| s.error.as_ref()) {
        problems.push(format!(
            "The last look at {} failed: {error}",
            ctx.lookup.label()
        ));
    }
    if !managed.is_empty() {
        if let Some(foreign) = managed::foreign_launcher(layout) {
            notes.push(format!(
                "{foreign}, so it was left alone; every other cohors hands control to cohors' own install."
            ));
        } else if let Some(standing) = standing(layout) {
            // A copy ahead on the PATH that hands off is fine; one that does
            // not (a build from source) runs instead of the managed copy.
            match standing {
                Standing::Missing => notes.push(format!(
                    "{} is not on your PATH; the other copies hand control to cohors' own install, so that is fine.",
                    layout.show(&layout.bin)
                )),
                Standing::Behind(other) if other.channel.hands_off() => notes.push(format!(
                    "{} ({}) comes first on your PATH; it hands control to cohors' own install.",
                    layout.show(&other.path),
                    other.channel.label()
                )),
                Standing::Behind(other) => problems.push(format!(
                    "{} ({}) comes first on your PATH and never hands off, so typing `cohors` runs it. Put {} before it on your PATH.",
                    layout.show(&other.path),
                    other.channel.label(),
                    layout.show(&layout.bin)
                )),
            }
        }
    }
    if on_path.len() > 1 {
        notes.push(format!(
            "{} copies of cohors are on your PATH; typing `cohors` runs the first.",
            on_path.len()
        ));
    }

    let report = Report {
        ran,
        current: ctx.current.clone(),
        running,
        installed_as: ctx.installed_as.clone(),
        updates_through_managed: ctx.channel != ctx.installed_as,
        handoff: handoff_line,
        set_up,
        auto: auto_update(&switches, &ctx.channel),
        source: ctx.lookup.label(),
        seen,
        last,
        on_path: on_path
            .iter()
            .map(|f| {
                (
                    layout.show(&f.path),
                    f.version.clone(),
                    f.channel.label().to_string(),
                )
            })
            .collect(),
        launcher: layout.show(&layout.launcher()),
        managed,
        active,
        problems,
        notes,
        now,
    };
    print!("{}", render(&report));
    Ok(())
}

/// After `cohors install`: whether typing `cohors` runs the managed copy, and
/// what to change if not.
pub(crate) fn path_advice(layout: &Layout) -> String {
    let bin = layout.show(&layout.bin);
    match standing(layout) {
        None => format!("  Typing `cohors` runs it: {bin} comes first on your PATH."),
        Some(Standing::Missing) => format!(
            "  {bin} is not on your PATH. Add this line to your shell's profile (~/.zshrc, ~/.bashrc), then open a new terminal:\n    export PATH=\"$HOME/.local/bin:$PATH\""
        ),
        Some(Standing::Behind(other)) if other.channel.hands_off() => format!(
            "  {} ({}) comes first on your PATH; it hands control to this copy whenever this one is newer.",
            layout.show(&other.path),
            other.channel.label()
        ),
        Some(Standing::Behind(other)) => format!(
            "  {} ({}) comes first on your PATH and never hands off, so typing `cohors` runs it. Remove it{}, or put {bin} before it on your PATH.",
            layout.show(&other.path),
            other.channel.label(),
            other
                .channel
                .uninstall_command()
                .map(|c| format!(" ({c})"))
                .unwrap_or_default(),
        ),
    }
}

/// Where the launcher stands on the PATH, when it is not first.
enum Standing {
    Missing,
    Behind(Found),
}

fn standing(layout: &Layout) -> Option<Standing> {
    let real = |p: &Path| {
        std::fs::canonicalize(p)
            .unwrap_or(p.to_path_buf())
            .to_string_lossy()
            .into_owned()
    };
    let path: Vec<String> = path_dirs().iter().map(|d| real(d)).collect();
    let holders: Vec<String> = holders()
        .iter()
        .filter_map(|file| file.parent().map(real))
        .collect();
    match path_standing(&path, &holders, &real(&layout.bin)) {
        PathStanding::First => None,
        PathStanding::Missing => Some(Standing::Missing),
        PathStanding::Behind(first) => Some(Standing::Behind(classify(
            layout,
            &Path::new(&first).join(EXE),
        ))),
    }
}

/// The PATH's folders, absolute ones only. An empty entry or a relative one
/// (`.`, `bin`) means "the current folder", which is often a repository
/// someone else wrote: a `cohors` there must never be counted, let alone run
/// (doctor runs each copy it finds with `--version`).
fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| absolute_dirs(&path))
        .unwrap_or_default()
}

fn absolute_dirs(path: &std::ffi::OsStr) -> Vec<PathBuf> {
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .collect()
}

/// May doctor run this file to ask its version? Not when the file, or the
/// folder it really lives in, can be changed by other users (group- or
/// world-writable, as in `/tmp`): anyone could have put it there.
#[cfg(unix)]
fn safe_to_run(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Ok(real) = std::fs::canonicalize(path) else {
        return false;
    };
    let writable_by_others =
        |p: &Path| std::fs::metadata(p).map_or(true, |m| m.permissions().mode() & 0o022 != 0);
    real.is_absolute()
        && !writable_by_others(&real)
        && real.parent().is_some_and(|dir| !writable_by_others(dir))
}

#[cfg(not(unix))]
fn safe_to_run(path: &Path) -> bool {
    path.is_absolute()
}

/// Every `cohors` on the PATH, in PATH order, each file once.
pub(crate) fn holders() -> Vec<PathBuf> {
    let mut seen = Vec::new();
    let mut found = Vec::new();
    for dir in path_dirs() {
        let file = dir.join(EXE);
        if !file.is_file() {
            continue;
        }
        let real = std::fs::canonicalize(&file).unwrap_or(file.clone());
        if !seen.contains(&real) {
            seen.push(real);
            found.push(file);
        }
    }
    found
}

/// One cohors found on the PATH.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    pub path: PathBuf,
    pub version: Option<String>,
    pub channel: Channel,
}

pub(crate) fn classify(layout: &Layout, path: &Path) -> Found {
    let version = version_of(path);
    classify_as(layout, path, version)
}

/// Where the cohors at `path`, at `version`, came from: by where its file
/// really lives, whether it is pip's Python launcher, and the installer's and
/// cargo's records.
fn classify_as(layout: &Layout, path: &Path, version: Option<String>) -> Found {
    let real = std::fs::canonicalize(path).unwrap_or(path.to_path_buf());
    let exe = real.to_string_lossy().into_owned();
    let managed = std::fs::canonicalize(layout.versions())
        .unwrap_or(layout.versions())
        .to_string_lossy()
        .into_owned();
    let v = version.as_deref().unwrap_or("");
    let channel = detect_channel(&InstallFacts {
        exe: &exe,
        launcher: python_launcher(&real).then_some("pip"),
        python: None,
        managed_versions: Some(&managed),
        receipt: installer_owns(layout, &exe, v),
        crates_io: cargo_installed(layout, &exe, v),
    });
    Found {
        path: path.to_path_buf(),
        version,
        channel,
    }
}

/// Is this pip's `cohors` launcher (a Python script), not the binary?
fn python_launcher(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 128];
    let read = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .unwrap_or(0);
    let head = String::from_utf8_lossy(&head[..read]);
    head.starts_with("#!") && head.lines().next().is_some_and(|l| l.contains("python"))
}

/// `<path> --version`, with updates and the hand-off off, given at most five
/// seconds. `None` (version unknown) for a file it is not safe to run.
fn version_of(path: &Path) -> Option<String> {
    if !safe_to_run(path) {
        return None;
    }
    let mut child = Command::new(path)
        .arg("--version")
        .env("COHORS_DISABLE_UPDATES", "1")
        .env("COHORS_NO_UPDATE", "1")
        .env(handoff::HANDOFF_VAR, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().ok()?.is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().ok()?;
    let said = String::from_utf8_lossy(&output.stdout);
    let first = said.lines().next()?.trim();
    first.strip_prefix("cohors ").map(|v| v.trim().to_string())
}

/// What the report shows, gathered first so drawing it is testable.
struct Report {
    /// The copy the person ran.
    ran: String,
    current: String,
    running: String,
    installed_as: Channel,
    /// A copy that hands off updates cohors' own install.
    updates_through_managed: bool,
    handoff: String,
    set_up: Option<String>,
    auto: (Auto, String),
    source: String,
    seen: Option<Seen>,
    last: Option<Attempt>,
    /// Each cohors on the PATH: where, its version, where it came from.
    on_path: Vec<(String, Option<String>, String)>,
    launcher: String,
    managed: Vec<String>,
    active: Option<String>,
    problems: Vec<String>,
    notes: Vec<String>,
    now: i64,
}

fn ago(now: i64, at: i64) -> String {
    let secs = (now - at).max(0);
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

fn render(r: &Report) -> String {
    let mut out = String::from("cohors doctor\n\n");
    let row = |out: &mut String, label: &str, text: &str| {
        out.push_str(&format!("  {label:<13} {text}\n"));
    };
    row(&mut out, "You ran", &r.ran);
    let through = if r.updates_through_managed {
        "; updates cohors' own install"
    } else {
        ""
    };
    row(
        &mut out,
        "Running",
        &format!(
            "cohors {} at {}, installed with {}{through}",
            r.current,
            r.running,
            r.installed_as.label()
        ),
    );
    row(&mut out, "Hand-off", &r.handoff);
    row(
        &mut out,
        "Set up",
        r.set_up.as_deref().unwrap_or("not by a first run"),
    );
    let (auto, why) = &r.auto;
    let state = match auto {
        Auto::Installs => "enabled",
        Auto::Tells => "notify only",
        Auto::Off => "disabled",
    };
    row(&mut out, "Auto-updates", &format!("{state} ({why})"));
    let newest = match &r.seen {
        Some(seen) if seen.checked_at > 0 => {
            let latest = seen.latest.as_deref().unwrap_or("unknown");
            format!(
                "{latest} on {}, asked {}",
                r.source,
                ago(r.now, seen.checked_at)
            )
        }
        _ => format!("not asked yet ({})", r.source),
    };
    row(&mut out, "Newest", &newest);
    let last = match &r.last {
        Some(a) => format!(
            "{} → {} with {}, {}: {}",
            a.from,
            a.to,
            a.channel,
            ago(r.now, a.at),
            if a.ok { "ok" } else { "failed" }
        ),
        None => "none yet".into(),
    };
    row(&mut out, "Last update", &last);

    out.push_str("\n  On your PATH (typing `cohors` runs the first)\n");
    if r.on_path.is_empty() {
        out.push_str("    none\n");
    }
    let width = r
        .on_path
        .iter()
        .map(|(p, ..)| p.chars().count())
        .max()
        .unwrap_or(0);
    for (n, (path, version, from)) in r.on_path.iter().enumerate() {
        out.push_str(&format!(
            "    {}. {path:<width$}  {:<8} {from}\n",
            n + 1,
            version.as_deref().unwrap_or("?"),
        ));
    }

    out.push_str("\n  cohors' own install\n");
    if r.managed.is_empty() {
        out.push_str(
            "    not set up; the first run of a released cohors sets it up (or `cohors install`)\n",
        );
    } else {
        let active = r.active.as_deref().unwrap_or("nothing");
        out.push_str(&format!("    launcher  {} → {active}\n", r.launcher));
        let versions: Vec<String> = r
            .managed
            .iter()
            .map(|v| {
                if Some(v) == r.active.as_ref() {
                    format!("{v} (active)")
                } else {
                    v.clone()
                }
            })
            .collect();
        out.push_str(&format!("    versions  {}\n", versions.join(", ")));
    }

    if !r.notes.is_empty() {
        out.push_str("\n  Notes\n");
        for note in &r.notes {
            out.push_str(&format!("    · {note}\n"));
        }
    }
    if r.problems.is_empty() {
        out.push_str("\n  No problems found.\n");
    } else {
        out.push_str("\n  Problems\n");
        for problem in &r.problems {
            out.push_str(&format!("    · {problem}\n"));
        }
    }
    out
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn file(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn real_install_layouts_are_recognised() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let layout = Layout::under(&root.join("home"));
        let at = |path: &Path, version: &str| {
            classify_as(&layout, path, Some(version.to_string())).channel
        };

        // Homebrew: bin/cohors links into the Cellar.
        let cellar = root.join("brew/Cellar/cohors/0.7.7/bin/cohors");
        file(&cellar, "binary");
        let brew_bin = root.join("brew/bin/cohors");
        std::fs::create_dir_all(brew_bin.parent().unwrap()).unwrap();
        symlink("../Cellar/cohors/0.7.7/bin/cohors", &brew_bin).unwrap();
        assert_eq!(at(&brew_bin, "0.7.7"), Channel::Homebrew);

        // npm -g: bin/cohors links to the package's launcher script.
        let script = root.join("npm/lib/node_modules/@cohors/cli/run-cohors.js");
        file(&script, "#!/usr/bin/env node\n");
        let npm_bin = root.join("npm/bin/cohors");
        std::fs::create_dir_all(npm_bin.parent().unwrap()).unwrap();
        symlink("../lib/node_modules/@cohors/cli/run-cohors.js", &npm_bin).unwrap();
        assert_eq!(at(&npm_bin, "0.7.7"), Channel::Npm);

        // pipx: ~/.local/bin/cohors links to the venv's Python launcher.
        let pipx = root.join("home/.local/pipx/venvs/cohors/bin/cohors");
        file(
            &pipx,
            "#!/home/a/.local/pipx/venvs/cohors/bin/python\nimport sys\n",
        );
        assert_eq!(at(&pipx, "0.7.7"), Channel::Pip { python: None });

        // uv tool: the same launcher, in uv's tool folder.
        let uv = root.join("home/.local/share/uv/tools/cohors/bin/cohors");
        file(&uv, "#!/home/a/.local/share/uv/tools/cohors/bin/python\n");
        assert_eq!(at(&uv, "0.7.7"), Channel::UvTool);

        // The shell installer: a receipt claiming ~/.cargo at this version.
        let cargo_bin = layout.cargo_home.join("bin/cohors");
        file(&cargo_bin, "binary");
        let prefix = std::fs::canonicalize(&layout.cargo_home).unwrap();
        file(
            &layout.config_home.join("cohors/cohors-receipt.json"),
            &format!(
                r#"{{"install_prefix": "{}", "install_layout": "cargo-home", "version": "0.7.7"}}"#,
                prefix.display()
            ),
        );
        assert_eq!(at(&cargo_bin, "0.7.7"), Channel::Installer);
        // The receipt is for another version: not the installer's binary. Cargo's
        // record says crates.io put this one there.
        file(
            &layout.cargo_home.join(".crates2.json"),
            r#"{"installs": {"cohors 0.7.6 (registry+https://github.com/rust-lang/crates.io-index)": {}}}"#,
        );
        assert_eq!(at(&cargo_bin, "0.7.6"), Channel::Cargo);
        assert_eq!(at(&cargo_bin, "0.7.5"), Channel::Source);

        // cohors' own install, through its launcher.
        let managed = layout.versions().join("0.7.8").join(EXE);
        file(&managed, "binary");
        std::fs::create_dir_all(&layout.bin).unwrap();
        symlink(&managed, layout.launcher()).unwrap();
        assert_eq!(at(&layout.launcher(), "0.7.8"), Channel::Managed);
    }

    #[test]
    fn the_report_says_what_ran_what_handed_off_and_what_is_wrong() {
        let report = Report {
            ran: "/opt/homebrew/bin/cohors (Homebrew 0.7.7), which handed control to this copy"
                .into(),
            current: "0.7.8".into(),
            running: "~/.local/share/cohors/versions/0.7.8/cohors".into(),
            installed_as: Channel::Managed,
            updates_through_managed: false,
            handoff: "handed off from 0.7.7 to 0.7.8".into(),
            set_up: Some("from Homebrew 0.7.7, 2 h ago".into()),
            auto: (Auto::Installs, "cohors' own install updates itself".into()),
            source: "GitHub releases".into(),
            seen: Some(Seen {
                latest: Some("0.7.8".into()),
                etag: None,
                checked_at: 1_000 - 180,
                error: None,
            }),
            last: Some(Attempt {
                at: 1_000 - 7200,
                from: "0.7.7".into(),
                to: "0.7.8".into(),
                channel: "cohors install".into(),
                ok: true,
                reason: None,
            }),
            on_path: vec![
                (
                    "/opt/homebrew/bin/cohors".into(),
                    Some("0.7.7".into()),
                    "Homebrew".into(),
                ),
                (
                    "~/.local/bin/cohors".into(),
                    Some("0.7.8".into()),
                    "cohors install".into(),
                ),
            ],
            launcher: "~/.local/bin/cohors".into(),
            managed: vec!["0.7.8".into(), "0.7.7".into()],
            active: Some("0.7.8".into()),
            problems: vec!["The last hand-off, to 0.7.9, did not happen".into()],
            notes: vec!["2 copies of cohors are on your PATH".into()],
            now: 1_000,
        };
        let text = render(&report);
        for wanted in [
            "You ran       /opt/homebrew/bin/cohors (Homebrew 0.7.7), which handed control to this copy",
            "Running       cohors 0.7.8 at ~/.local/share/cohors/versions/0.7.8/cohors, installed with cohors install",
            "Hand-off      handed off from 0.7.7 to 0.7.8",
            "Set up        from Homebrew 0.7.7, 2 h ago",
            "Auto-updates  enabled (cohors' own install updates itself)",
            "Newest        0.7.8 on GitHub releases, asked 3 min ago",
            "Last update   0.7.7 → 0.7.8 with cohors install, 2 h ago: ok",
            "1. /opt/homebrew/bin/cohors  0.7.7    Homebrew",
            "launcher  ~/.local/bin/cohors → 0.7.8",
            "versions  0.7.8 (active), 0.7.7",
            "Notes\n    · 2 copies of cohors are on your PATH",
            "Problems\n    · The last hand-off, to 0.7.9, did not happen",
        ] {
            assert!(text.contains(wanted), "missing {wanted:?} in:\n{text}");
        }
        let empty = Report {
            seen: None,
            last: None,
            set_up: None,
            on_path: Vec::new(),
            managed: Vec::new(),
            active: None,
            problems: Vec::new(),
            notes: Vec::new(),
            ..report
        };
        let text = render(&empty);
        assert!(text.contains("not set up; the first run"), "{text}");
        assert!(text.contains("Set up        not by a first run"), "{text}");
        assert!(text.contains("No problems found."), "{text}");
        assert!(!text.contains("Notes"), "{text}");
        assert!(text.contains("not asked yet (GitHub releases)"), "{text}");
    }
}

#[cfg(test)]
mod path_safety {
    use super::*;

    #[test]
    fn only_absolute_path_folders_are_searched() {
        // An empty entry and `.` both mean the current folder.
        let dirs = absolute_dirs(std::ffi::OsStr::new("/usr/bin::.:bin:/opt/homebrew/bin"));
        assert_eq!(
            dirs,
            [
                PathBuf::from("/usr/bin"),
                PathBuf::from("/opt/homebrew/bin")
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_copy_others_can_change_is_never_run() {
        use std::os::unix::fs::PermissionsExt;
        let set =
            |p: &Path, mode| std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode));
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(EXE);
        std::fs::write(&file, "#!/bin/sh\necho cohors 9.9.9\n").unwrap();
        set(dir.path(), 0o755).unwrap();
        set(&file, 0o755).unwrap();
        assert!(safe_to_run(&file));
        assert_eq!(version_of(&file).as_deref(), Some("9.9.9"));
        // A folder anyone can write to, as /tmp is.
        set(dir.path(), 0o777).unwrap();
        assert!(!safe_to_run(&file));
        assert_eq!(version_of(&file), None);
        set(dir.path(), 0o755).unwrap();
        // A file others can rewrite.
        set(&file, 0o777).unwrap();
        assert!(!safe_to_run(&file));
        // A relative path is never run.
        assert!(!safe_to_run(Path::new("cohors")));
    }
}
