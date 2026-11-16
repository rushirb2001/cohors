//! Keeping cohors itself up to date: the pure half.
//!
//! Comparing versions, recognising how this copy of cohors was installed,
//! reading each channel's answer to "what is the newest version?", deciding
//! what to do about it, and the words every surface shows. No I/O lives here
//! (the lookups, downloads and installs run in the binary), so all of it is
//! testable and WASM-safe.

use serde::{Deserialize, Serialize};

/// A release version, `MAJOR.MINOR.PATCH`. A leading `v` is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    /// Parse `0.7.1` or `v0.7.1`. A pre-release (`0.8.0-rc.1`) or anything else
    /// that is not three plain numbers gives `None`: cohors only ever offers
    /// stable releases.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.split('.');
        let mut next = || parts.next()?.parse::<u64>().ok();
        let version = Version(next()?, next()?, next()?);
        // A fourth part, or a suffix the number parse already refused, means
        // this is not a plain release.
        parts.next().is_none().then_some(version)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// Is `latest` a stable release newer than `current`? `false` whenever either
/// cannot be read, so a strange tag never produces a notice.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (Version::parse(latest), Version::parse(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

// ── How this copy was installed ─────────────────────────────────────────────

/// How this copy of cohors got onto the machine, which decides where to ask
/// for the newest version and how it updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Channel {
    /// cohors' own install: `~/.local/share/cohors/versions/<version>/cohors`,
    /// with `~/.local/bin/cohors` linked to the active version (set up by
    /// `cohors install`). It downloads, verifies and swaps in new releases
    /// itself, like Claude Code's native install.
    Managed,
    /// A Homebrew formula (the binary lives under `Cellar/cohors/`).
    Homebrew,
    /// `pip install cohors` (a venv, `--user`, or pipx, which installs with
    /// pip). `python` is the interpreter whose pip owns it, when known.
    Pip { python: Option<String> },
    /// Run through `uvx`, from uv's cache. Nothing to install: `uvx
    /// cohors@latest` fetches the newest release when it next starts.
    Uvx,
    /// `uv tool install cohors`. Only told: `uv tool upgrade cohors` is uv's to
    /// run, and uv's tool folders have no pip to run instead.
    UvTool,
    /// `npm install -g @cohors/cli`. The npm package keeps the binary inside
    /// itself (`node_modules/@cohors/cli/node_modules/.bin_real/`). Only told,
    /// never updated in place: a global npm prefix often needs elevated rights.
    Npm,
    /// Run through `npx`, from npm's cache. Like uvx, `npx @cohors/cli@latest`
    /// fetches the newest release when it next starts.
    Npx,
    /// The release's shell installer, which leaves an install receipt.
    Installer,
    /// `cargo install cohors`, from crates.io. Only told, never updated in
    /// place: updating means compiling from source, which takes minutes.
    Cargo,
    /// Built from source (`cargo install --path`, a checkout, a dev build).
    /// Never updated automatically: whoever built it chose what to build.
    Source,
}

/// What the binary knows about itself, gathered by the I/O half.
#[derive(Debug, Clone, Copy, Default)]
pub struct InstallFacts<'a> {
    /// The running binary's path, symlinks resolved.
    pub exe: &'a str,
    /// `COHORS_INSTALLER`: the pip wrapper sets `pip`, or `uvx` when it runs
    /// from uv's folders.
    pub launcher: Option<&'a str>,
    /// `COHORS_PYTHON`: the interpreter the pip wrapper ran under.
    pub python: Option<&'a str>,
    /// The managed install's `versions` folder, symlinks resolved.
    pub managed_versions: Option<&'a str>,
    /// Does the shell installer's receipt claim this binary?
    pub receipt: bool,
    /// Does cargo's record of installs say it put this version here from
    /// crates.io?
    pub crates_io: bool,
}

/// Recognise the channel from where the binary lives and what launched it.
pub fn detect_channel(facts: &InstallFacts) -> Channel {
    let exe = facts.exe.replace('\\', "/");
    let python = facts.python.map(str::to_string);
    // cohors' own install first: a managed copy handed control by a pip or npm
    // launcher still carries that launcher's environment.
    if let Some(versions) = facts.managed_versions {
        let versions = versions.replace('\\', "/");
        let versions = versions.trim_end_matches('/');
        if !versions.is_empty() && exe.starts_with(&format!("{versions}/")) {
            return Channel::Managed;
        }
    }
    // `uv tool install` runs the same wrapper as `uvx`, which says `uvx`.
    if exe.contains("/uv/tools/") {
        return Channel::UvTool;
    }
    if facts.launcher == Some("uvx") || exe.contains("/uv/archive-") {
        return Channel::Uvx;
    }
    if facts.launcher == Some("pip") || exe.contains("/site-packages/cohors/") {
        return Channel::Pip { python };
    }
    // npx runs from npm's cache (`_npx/<hash>/node_modules/@cohors/cli/`), so
    // test for it before the plain npm install it otherwise looks like.
    if exe.contains("/_npx/") {
        return Channel::Npx;
    }
    if exe.contains("/node_modules/@cohors/cli/") {
        return Channel::Npm;
    }
    if exe.contains("/Cellar/cohors/") {
        return Channel::Homebrew;
    }
    if facts.receipt {
        return Channel::Installer;
    }
    if facts.crates_io {
        return Channel::Cargo;
    }
    Channel::Source
}

impl Channel {
    /// Can cohors install an update for this channel by itself?
    pub fn installs_itself(&self) -> bool {
        matches!(
            self,
            Channel::Managed | Channel::Homebrew | Channel::Pip { .. } | Channel::Installer
        )
    }

    /// How this channel is named to a person: "installed with {label}".
    pub fn label(&self) -> &'static str {
        match self {
            Channel::Managed => "cohors install",
            Channel::Homebrew => "Homebrew",
            Channel::Pip { .. } => "pip",
            Channel::Uvx => "uvx",
            Channel::UvTool => "uv tool",
            Channel::Npm => "npm",
            Channel::Npx => "npx",
            Channel::Installer => "the shell installer",
            Channel::Cargo => "cargo",
            Channel::Source => "a build from source",
        }
    }

    /// The command a person would run to update, for the notice when cohors
    /// will not (or could not) do it itself.
    pub fn manual_command(&self) -> &'static str {
        match self {
            Channel::Managed => "cohors update",
            Channel::Homebrew => "brew upgrade cohors",
            Channel::Pip { .. } => "pip install -U cohors",
            Channel::Uvx => "uvx cohors@latest",
            Channel::UvTool => "uv tool upgrade cohors",
            Channel::Npm => "npm i -g @cohors/cli@latest",
            Channel::Npx => "npx @cohors/cli@latest",
            Channel::Installer => {
                "curl --proto '=https' --tlsv1.2 -LsSf https://github.com/rushirb2001/cohors/releases/latest/download/cohors-installer.sh | sh"
            }
            Channel::Cargo => "cargo install cohors",
            Channel::Source => "cargo install --git https://github.com/rushirb2001/cohors cohors",
        }
    }

    /// Where to ask for the newest version: the place this channel installs
    /// from, so cohors never offers a release the channel cannot install yet.
    /// (GitHub's release goes up minutes before the tap, PyPI, npm and
    /// crates.io have it.)
    pub fn source(&self) -> VersionSource {
        match self {
            Channel::Homebrew => VersionSource::HomebrewTap,
            Channel::Pip { .. } | Channel::Uvx | Channel::UvTool => VersionSource::Pypi,
            Channel::Npm | Channel::Npx => VersionSource::Npm,
            Channel::Cargo => VersionSource::CratesIo,
            Channel::Managed | Channel::Installer | Channel::Source => VersionSource::GithubRelease,
        }
    }

    /// Does a copy installed this way hand control to cohors' own install
    /// (and set that up on its first run)? Every way of installing a released
    /// cohors does; cohors' own install is the one handed to, and a build from
    /// source runs exactly what was built.
    pub fn hands_off(&self) -> bool {
        !matches!(self, Channel::Managed | Channel::Source)
    }

    /// The command that removes a copy installed this way, for `cohors
    /// uninstall` and `cohors doctor`. `None` when there is nothing installed
    /// to remove (uvx and npx run from a cache) or nothing to say.
    pub fn uninstall_command(&self) -> Option<&'static str> {
        match self {
            Channel::Homebrew => Some("brew uninstall cohors"),
            Channel::Pip { .. } => Some("pip uninstall cohors (or pipx uninstall cohors)"),
            Channel::UvTool => Some("uv tool uninstall cohors"),
            Channel::Npm => Some("npm rm -g @cohors/cli"),
            Channel::Cargo | Channel::Installer => Some("cargo uninstall cohors"),
            Channel::Managed => Some("cohors uninstall"),
            Channel::Uvx | Channel::Npx | Channel::Source => None,
        }
    }
}

/// How a copy updates: a copy that hands off updates cohors' own install once
/// that is set up (the next run hands off to what it installed), and never runs
/// its package manager. `handing_off` is whether hand-off is allowed for this
/// run at all (not a dev build) and the managed copy exists.
pub fn updates_through(channel: &Channel, handing_off: bool) -> Channel {
    if handing_off && channel.hands_off() {
        Channel::Managed
    } else {
        channel.clone()
    }
}

// ── Where the newest version is read from ───────────────────────────────────

/// One place that answers "what is the newest cohors?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionSource {
    /// GitHub's releases API: the release the shell installer and the managed
    /// install download from.
    GithubRelease,
    /// The Homebrew tap's formula, read raw: what `brew upgrade` will install.
    HomebrewTap,
    /// PyPI's JSON API: what `pip install -U cohors` will install.
    Pypi,
    /// The npm registry's abbreviated metadata for `@cohors/cli`.
    Npm,
    /// crates.io's sparse index entry for `cohors` (the registry's own
    /// machine-readable record, which answers conditional requests).
    CratesIo,
}

impl VersionSource {
    /// The address asked. Every one answers `If-None-Match` with a bodiless
    /// `304 Not Modified` while nothing has changed.
    pub fn url(&self) -> &'static str {
        match self {
            VersionSource::GithubRelease => {
                "https://api.github.com/repos/rushirb2001/cohors/releases/latest"
            }
            VersionSource::HomebrewTap => {
                "https://raw.githubusercontent.com/rushirb2001/homebrew-tap/main/Formula/cohors.rb"
            }
            VersionSource::Pypi => "https://pypi.org/pypi/cohors/json",
            VersionSource::Npm => "https://registry.npmjs.org/@cohors%2fcli",
            VersionSource::CratesIo => "https://index.crates.io/co/ho/cohors",
        }
    }

    /// The `Accept` header that asks for the smallest useful answer.
    pub fn accept(&self) -> &'static str {
        match self {
            VersionSource::GithubRelease => "application/vnd.github+json",
            VersionSource::HomebrewTap => "text/plain",
            VersionSource::Pypi => "application/json",
            // npm's abbreviated metadata: a few kB instead of every README.
            VersionSource::Npm => "application/vnd.npm.install-v1+json",
            VersionSource::CratesIo => "application/json",
        }
    }

    /// How the source is named to a person.
    pub fn label(&self) -> &'static str {
        match self {
            VersionSource::GithubRelease => "GitHub releases",
            VersionSource::HomebrewTap => "the Homebrew tap",
            VersionSource::Pypi => "PyPI",
            VersionSource::Npm => "the npm registry",
            VersionSource::CratesIo => "crates.io",
        }
    }

    /// The newest stable version in this source's answer, `None` when the
    /// answer cannot be read.
    pub fn latest_in(&self, body: &str) -> Option<String> {
        let found = match self {
            VersionSource::GithubRelease => json_str(body, &["tag_name"]),
            VersionSource::HomebrewTap => formula_version(body),
            VersionSource::Pypi => pypi_latest(body),
            VersionSource::Npm => json_str(body, &["dist-tags", "latest"]),
            VersionSource::CratesIo => index_latest(body),
        }?;
        Version::parse(&found).map(|v| v.to_string())
    }
}

/// The newest version in an answer of any of the shapes above, or a bare
/// version on its own line. For the test seam (`COHORS_UPDATE_SOURCE`), which
/// points every lookup at one local file or URL.
pub fn latest_in_any(body: &str) -> Option<String> {
    [
        VersionSource::GithubRelease,
        VersionSource::Npm,
        VersionSource::Pypi,
        VersionSource::HomebrewTap,
        VersionSource::CratesIo,
    ]
    .iter()
    .find_map(|source| source.latest_in(body))
    .or_else(|| {
        let first = body.lines().map(str::trim).find(|l| !l.is_empty())?;
        Version::parse(first).map(|v| v.to_string())
    })
}

fn json_str(body: &str, path: &[&str]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let mut at = &value;
    for key in path {
        at = at.get(key)?;
    }
    at.as_str().map(str::to_string)
}

/// `version "0.7.7"` in a Homebrew formula.
fn formula_version(body: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("version ")?;
        let rest = rest.trim().strip_prefix('"')?;
        rest.split('"').next().map(str::to_string)
    })
}

/// PyPI's newest stable release that still has files and is not yanked;
/// `info.version` when the release list is missing.
fn pypi_latest(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let from_releases = value
        .get("releases")
        .and_then(|r| r.as_object())
        .and_then(|releases| {
            releases
                .iter()
                .filter(|(_, files)| {
                    files.as_array().is_some_and(|files| {
                        files.iter().any(|f| f.get("yanked") != Some(&true.into()))
                    })
                })
                .filter_map(|(version, _)| Version::parse(version))
                .max()
        });
    from_releases
        .map(|v| v.to_string())
        .or_else(|| value.pointer("/info/version")?.as_str().map(str::to_string))
}

/// The sparse index holds one JSON object per published version, one per line.
fn index_latest(body: &str) -> Option<String> {
    body.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|entry| entry.get("yanked") != Some(&true.into()))
        .filter_map(|entry| Version::parse(entry.get("vers")?.as_str()?))
        .max()
        .map(|v| v.to_string())
}

// ── What to do ──────────────────────────────────────────────────────────────

/// A failed install is not tried again by itself for this long, so a broken
/// install never runs on every launch. A newer release is tried at once.
pub const RETRY_FAILED_AFTER: i64 = 24 * 60 * 60;

/// An install that failed: which version, when, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub version: String,
    pub at: i64,
    pub reason: String,
}

/// What is known when deciding.
#[derive(Debug, Clone, Copy)]
pub struct Known<'a> {
    /// This build's version.
    pub current: &'a str,
    /// The newest version the channel's own source lists, if it was reached
    /// (now or earlier).
    pub latest: Option<&'a str>,
    /// A version an earlier run installed that is waiting for a restart.
    pub installed: Option<&'a str>,
    /// The last install that failed.
    pub failed: Option<&'a Failure>,
    /// The version a restart was meant to start.
    pub restarted_into: Option<&'a str>,
    pub now: i64,
}

/// The decision for one look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Nothing newer is known.
    Nothing,
    /// Say this; install nothing.
    Report(UpdateState),
    /// Install this version (saying "updating…" first).
    Install(String),
    /// A restart was meant to start a newer version and this is still the old
    /// one: the install went somewhere this copy is not run from. Remember it
    /// as a failure and say so, instead of installing again in a loop.
    RestartMissed(Failure),
}

/// Decide what one look leads to. `auto` is `updates = "auto"` (with nothing
/// in the environment switching background installs off).
pub fn decide(channel: &Channel, auto: bool, known: &Known) -> Decision {
    let current = known.current;
    let command = channel.manual_command().to_string();
    let Some(latest) = known.latest.filter(|v| is_newer(v, current)) else {
        // The source was not reached (or lists nothing newer), but an earlier
        // run installed a newer version: only the restart is left.
        return match known.installed.filter(|v| is_newer(v, current)) {
            Some(installed) => Decision::Report(UpdateState::Ready {
                version: installed.to_string(),
            }),
            None => Decision::Nothing,
        };
    };
    if known.restarted_into == Some(latest) {
        return Decision::RestartMissed(Failure {
            version: latest.to_string(),
            at: known.now,
            reason: format!(
                "{latest} was installed, but the cohors that started is still {current}"
            ),
        });
    }
    if !auto || !channel.installs_itself() {
        return Decision::Report(UpdateState::Available {
            version: latest.to_string(),
            command,
        });
    }
    if known.installed == Some(latest) {
        return Decision::Report(UpdateState::Ready {
            version: latest.to_string(),
        });
    }
    if let Some(failure) = known
        .failed
        .filter(|f| f.version == latest && known.now - f.at < RETRY_FAILED_AFTER)
    {
        return Decision::Report(UpdateState::Failed {
            version: latest.to_string(),
            command,
            reason: failure.reason.clone(),
        });
    }
    Decision::Install(latest.to_string())
}

/// What a one-off command (or the MCP server) says about a decision: those
/// never install, so "would install" becomes "`cohors update` installs it".
pub fn report_only(decision: Decision, channel: &Channel) -> Option<UpdateState> {
    match decision {
        Decision::Nothing => None,
        Decision::Report(state) => Some(state),
        Decision::Install(version) => Some(UpdateState::Available {
            version,
            command: "cohors update".to_string(),
        }),
        Decision::RestartMissed(failure) => Some(UpdateState::Failed {
            version: failure.version,
            command: channel.manual_command().to_string(),
            reason: failure.reason,
        }),
    }
}

// ── Whether updates run at all ──────────────────────────────────────────────

/// The `updates` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Auto,
    Notify,
    Off,
}

/// The switches that decide whether anything updates in the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switches {
    pub setting: Setting,
    /// `COHORS_DISABLE_UPDATES`: no updates at all, not even by hand.
    pub disable_updates: bool,
    /// `COHORS_NO_UPDATE`: no background checks; `cohors update` still works.
    pub no_update: bool,
    /// Running in CI.
    pub ci: bool,
}

/// What the background check does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Auto {
    /// Looks, and installs newer releases.
    Installs,
    /// Looks, and only says what to run.
    Tells,
    /// Does not look.
    Off,
}

/// What the background check does for this channel, and why, in words
/// `cohors doctor` prints.
pub fn auto_update(switches: &Switches, channel: &Channel) -> (Auto, String) {
    if switches.disable_updates {
        return (
            Auto::Off,
            "COHORS_DISABLE_UPDATES is set: no updates at all, not even `cohors update`".into(),
        );
    }
    if switches.no_update {
        return (
            Auto::Off,
            "COHORS_NO_UPDATE is set: no background checks (`cohors update` still works)".into(),
        );
    }
    if switches.ci {
        return (Auto::Off, "running in CI".into());
    }
    match switches.setting {
        Setting::Off => (Auto::Off, "updates = \"off\" in the config".into()),
        Setting::Notify => (
            Auto::Tells,
            "updates = \"notify\" in the config: says what to run, installs nothing".into(),
        ),
        Setting::Auto => match channel {
            Channel::Managed => (
                Auto::Installs,
                "cohors' own install downloads, verifies and switches to new releases".into(),
            ),
            Channel::Homebrew => (Auto::Installs, "installs through `brew upgrade`".into()),
            Channel::Pip { .. } => (Auto::Installs, "installs through pip".into()),
            Channel::Installer => (
                Auto::Installs,
                "installs with the release's shell installer".into(),
            ),
            other => (
                Auto::Tells,
                format!(
                    "installed with {}: says to run `{}`",
                    other.label(),
                    other.manual_command()
                ),
            ),
        },
    }
}

// ── The managed install ─────────────────────────────────────────────────────

/// How many releases the managed install keeps besides the running one.
pub const KEEP_NEWEST: usize = 2;

/// The release target this build downloads, by cargo-dist's names. `None`
/// for a platform no release is built for.
pub fn release_target(arch: &str, os: &str, musl: bool) -> Option<&'static str> {
    match (arch, os, musl) {
        ("aarch64", "macos", _) => Some("aarch64-apple-darwin"),
        ("x86_64", "macos", _) => Some("x86_64-apple-darwin"),
        ("aarch64", "linux", false) => Some("aarch64-unknown-linux-gnu"),
        ("x86_64", "linux", false) => Some("x86_64-unknown-linux-gnu"),
        _ => None,
    }
}

/// The release archive for a target: `cohors-<target>.tar.xz`, with its
/// checksum beside it as `<archive>.sha256`.
pub fn archive_name(target: &str) -> String {
    format!("cohors-{target}.tar.xz")
}

/// The SHA-256 in a checksum file (`<hex> *<file>`, `<hex>  <file>`, or the
/// hex alone), lowercased. With a file name given, a line naming another file
/// does not count.
pub fn checksum_in(text: &str, file: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hex = parts.next()?;
        let named = parts.next().map(|name| name.trim_start_matches('*'));
        let ok = hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit());
        (ok && named.is_none_or(|name| name == file)).then(|| hex.to_ascii_lowercase())
    })
}

/// Which installed versions to delete: all but the ones in `keep` (the running
/// and the active version) and the `newest` newest. Names that are not plain
/// versions are never touched.
pub fn to_prune(installed: &[String], keep: &[&str], newest: usize) -> Vec<String> {
    let mut versions: Vec<(Version, &String)> = installed
        .iter()
        .filter_map(|name| Version::parse(name).map(|v| (v, name)))
        .collect();
    // Newest first.
    versions.sort_by_key(|(version, _)| std::cmp::Reverse(*version));
    let kept: Vec<Version> = keep.iter().filter_map(|v| Version::parse(v)).collect();
    versions
        .into_iter()
        .enumerate()
        .filter(|(rank, (version, _))| *rank >= newest && !kept.contains(version))
        .map(|(_, (_, name))| name.clone())
        .collect()
}

/// Where the managed launcher stands on the PATH.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathStanding {
    /// Typing `cohors` runs it.
    First,
    /// Its folder is not on the PATH.
    Missing,
    /// Another cohors comes first: this one runs instead.
    Behind(String),
}

/// `path` is the PATH's folders in order; `holders` the ones holding a
/// `cohors`, in the same order; `launcher_dir` the managed launcher's folder.
pub fn path_standing(path: &[String], holders: &[String], launcher_dir: &str) -> PathStanding {
    let same = |a: &str, b: &str| a.trim_end_matches('/') == b.trim_end_matches('/');
    if !path.iter().any(|dir| same(dir, launcher_dir)) {
        return PathStanding::Missing;
    }
    match holders.first() {
        Some(first) if !same(first, launcher_dir) => PathStanding::Behind(first.clone()),
        _ => PathStanding::First,
    }
}

// ── Handing off to cohors' own install ──────────────────────────────────────

/// What a run knows when it decides whether to set up cohors' own install or
/// hand control to it.
#[derive(Debug, Clone, Copy)]
pub struct HandoffFacts<'a> {
    /// How the running copy was installed.
    pub channel: &'a Channel,
    /// A debug build (`cargo build`, `cargo test`): never hands off, never
    /// sets anything up, so working on cohors never touches the real install.
    pub dev_build: bool,
    pub switches: Switches,
    /// `COHORS_HANDOFF` is set: this run was itself handed control.
    pub handed_off: bool,
    /// The running version.
    pub current: &'a str,
    /// The managed copy's active version, if there is one.
    pub managed_active: Option<&'a str>,
}

/// Why a run does not hand off or set up: `None` when it may. In the order a
/// person would want to hear it.
pub fn handoff_blocked(facts: &HandoffFacts) -> Option<&'static str> {
    let s = &facts.switches;
    if facts.handed_off {
        Some("this run was handed control already (COHORS_HANDOFF)")
    } else if *facts.channel == Channel::Managed {
        Some("this is cohors' own install")
    } else if *facts.channel == Channel::Source {
        Some("a build from source runs exactly what was built")
    } else if facts.dev_build {
        Some("a dev build never hands off")
    } else if s.disable_updates {
        Some("COHORS_DISABLE_UPDATES is set")
    } else if s.no_update {
        Some("COHORS_NO_UPDATE is set")
    } else if s.ci {
        Some("running in CI")
    } else if s.setting == Setting::Off {
        Some("updates = \"off\" in the config")
    } else {
        None
    }
}

/// Whether this run sets up cohors' own install (copying itself in): only a
/// run that could hand off, with `updates = "auto"` (`notify` installs
/// nothing), and only once.
pub fn sets_up(facts: &HandoffFacts, already_set_up: bool) -> bool {
    handoff_blocked(facts).is_none() && facts.switches.setting == Setting::Auto && !already_set_up
}

/// The version to hand control to: the managed copy's active version, when it
/// is newer than this one and nothing blocks the hand-off.
pub fn hand_off_to(facts: &HandoffFacts) -> Option<String> {
    if handoff_blocked(facts).is_some() {
        return None;
    }
    facts
        .managed_active
        .filter(|active| is_newer(active, facts.current))
        .map(str::to_string)
}

/// Said once, after the first run set cohors' own install up. `home` is where
/// it lives, as a person reads it (`~/.local/share/cohors`).
pub fn set_up_notice(home: &str) -> String {
    format!("cohors keeps itself up to date from {home} now (cohors doctor for details)")
}

// ── What the person sees ────────────────────────────────────────────────────

/// Where an update stands, as every surface shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum UpdateState {
    /// A newer release exists. `command` is how to get it when cohors will not
    /// install it itself (notify-only, a source build, cargo, uv, npm, npx, or
    /// a one-off command, which leaves installing to `cohors update`).
    Available { version: String, command: String },
    /// Installing in the background.
    Installing { version: String },
    /// Installed; it takes effect when cohors next starts.
    Ready { version: String },
    /// The install failed; `command` is how to do it by hand.
    Failed {
        version: String,
        command: String,
        reason: String,
    },
}

impl UpdateState {
    /// The release this is about.
    pub fn version(&self) -> &str {
        match self {
            UpdateState::Available { version, .. }
            | UpdateState::Installing { version }
            | UpdateState::Ready { version }
            | UpdateState::Failed { version, .. } => version,
        }
    }

    /// The one-line notice for a command's closing line and the MCP server.
    pub fn notice(&self) -> String {
        match self {
            UpdateState::Ready { version } => {
                format!("Updated to {version} — next run is {version}")
            }
            other => other.notice_in_dashboard(),
        }
    }

    /// The one-line notice in the dashboard's header, where `R` restarts.
    pub fn notice_in_dashboard(&self) -> String {
        match self {
            UpdateState::Available { version, command } => {
                format!("Update available: {version} — run {command}")
            }
            UpdateState::Installing { version } => {
                format!("Update available: {version} — updating…")
            }
            UpdateState::Ready { version } => {
                format!("Updated to {version} — restart to use it (R)")
            }
            UpdateState::Failed { .. } => "Auto-update failed · run cohors doctor".to_string(),
        }
    }

    /// The longer story, for `cohors update` and `cohors doctor`: why it
    /// failed and how to do it by hand. `None` for the other states.
    pub fn detail(&self) -> Option<String> {
        match self {
            UpdateState::Failed {
                version,
                command,
                reason,
            } => Some(format!(
                "Installing {version} failed: {reason}. To update by hand, run: {command}"
            )),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number_not_text() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v0.7.1", "0.7.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.7.0", "0.7.0"));
        assert!(!is_newer("0.6.9", "0.7.0"));
    }

    #[test]
    fn odd_tags_never_count_as_newer() {
        assert!(!is_newer("0.8.0-rc.1", "0.7.0"));
        assert!(!is_newer("nightly", "0.7.0"));
        assert!(!is_newer("0.8", "0.7.0"));
        assert!(!is_newer("0.8.0.1", "0.7.0"));
        assert!(!is_newer("0.8.0", "dev"));
        assert_eq!(Version::parse(" v1.2.3 "), Some(Version(1, 2, 3)));
    }

    fn at(exe: &str) -> Channel {
        detect_channel(&InstallFacts {
            exe,
            managed_versions: Some("/Users/a/.local/share/cohors/versions"),
            ..Default::default()
        })
    }

    #[test]
    fn the_channel_follows_where_the_binary_lives() {
        assert_eq!(
            at("/opt/homebrew/Cellar/cohors/0.7.0/bin/cohors"),
            Channel::Homebrew
        );
        assert_eq!(
            at("/home/linuxbrew/.linuxbrew/Cellar/cohors/0.7.0/bin/cohors"),
            Channel::Homebrew
        );
        assert_eq!(
            at("/Users/a/.venv/lib/python3.12/site-packages/cohors/cohors"),
            Channel::Pip { python: None }
        );
        // pipx installs with pip into its own venv.
        assert_eq!(
            at("/Users/a/.local/pipx/venvs/cohors/lib/python3.12/site-packages/cohors/cohors"),
            Channel::Pip { python: None }
        );
        // A `pip install --user` copy.
        assert_eq!(
            at("/Users/a/Library/Python/3.12/lib/python/site-packages/cohors/cohors"),
            Channel::Pip { python: None }
        );
        assert_eq!(
            at("/Users/a/.cache/uv/archive-v0/abc/lib/python3.12/site-packages/cohors/cohors"),
            Channel::Uvx
        );
        assert_eq!(
            at("/Users/a/.local/share/uv/tools/cohors/lib/python3.12/site-packages/cohors/cohors"),
            Channel::UvTool
        );
        assert_eq!(
            at("/opt/homebrew/lib/node_modules/@cohors/cli/node_modules/.bin_real/cohors"),
            Channel::Npm
        );
        assert_eq!(
            at("/Users/a/.npm/_npx/3f2a/node_modules/@cohors/cli/node_modules/.bin_real/cohors"),
            Channel::Npx
        );
        assert_eq!(
            at("/Users/a/.local/share/cohors/versions/0.7.7/cohors"),
            Channel::Managed
        );
        // A folder that only starts like the managed one is not it.
        assert_eq!(
            at("/Users/a/.local/share/cohors/versions-old/cohors"),
            Channel::Source
        );
        assert_eq!(at("/Users/a/.cargo/bin/cohors"), Channel::Source);
        let cargo_bin = |receipt, crates_io| {
            detect_channel(&InstallFacts {
                exe: "/Users/a/.cargo/bin/cohors",
                receipt,
                crates_io,
                ..Default::default()
            })
        };
        assert_eq!(cargo_bin(true, false), Channel::Installer);
        assert_eq!(cargo_bin(false, true), Channel::Cargo);
    }

    #[test]
    fn what_the_launcher_says_wins_but_not_over_the_managed_install() {
        let facts = InstallFacts {
            exe: "/x/cohors",
            launcher: Some("pip"),
            python: Some("/v/bin/python3"),
            receipt: true,
            crates_io: true,
            ..Default::default()
        };
        assert_eq!(
            detect_channel(&facts),
            Channel::Pip {
                python: Some("/v/bin/python3".into())
            }
        );
        let uvx = InstallFacts {
            exe: "/x/cohors",
            launcher: Some("uvx"),
            ..Default::default()
        };
        assert_eq!(detect_channel(&uvx), Channel::Uvx);
        // A managed copy started by the pip wrapper keeps the wrapper's
        // environment; where it lives decides.
        let handed_off = InstallFacts {
            exe: "/h/.local/share/cohors/versions/0.7.8/cohors",
            managed_versions: Some("/h/.local/share/cohors/versions/"),
            ..facts
        };
        assert_eq!(detect_channel(&handed_off), Channel::Managed);
    }

    #[test]
    fn only_the_managed_install_package_managers_and_the_installer_update_themselves() {
        assert!(Channel::Managed.installs_itself());
        assert!(Channel::Homebrew.installs_itself());
        assert!(Channel::Installer.installs_itself());
        assert!(Channel::Pip { python: None }.installs_itself());
        for told in [
            Channel::Uvx,
            Channel::UvTool,
            Channel::Npm,
            Channel::Npx,
            Channel::Cargo,
            Channel::Source,
        ] {
            assert!(!told.installs_itself(), "{told:?}");
        }
    }

    #[test]
    fn each_channel_asks_the_place_it_installs_from() {
        assert_eq!(Channel::Homebrew.source(), VersionSource::HomebrewTap);
        assert_eq!(Channel::Pip { python: None }.source(), VersionSource::Pypi);
        assert_eq!(Channel::UvTool.source(), VersionSource::Pypi);
        assert_eq!(Channel::Npm.source(), VersionSource::Npm);
        assert_eq!(Channel::Cargo.source(), VersionSource::CratesIo);
        assert_eq!(Channel::Managed.source(), VersionSource::GithubRelease);
        assert_eq!(Channel::Installer.source(), VersionSource::GithubRelease);
        assert_eq!(Channel::Npm.manual_command(), "npm i -g @cohors/cli@latest");
        assert_eq!(Channel::Cargo.manual_command(), "cargo install cohors");
    }

    #[test]
    fn every_source_answer_is_read() {
        let github = r#"{"tag_name": "v0.7.7", "name": "0.7.7"}"#;
        assert_eq!(
            VersionSource::GithubRelease.latest_in(github).as_deref(),
            Some("0.7.7")
        );
        let formula = "class Cohors < Formula\n  desc \"x\"\n  version \"0.7.7\"\n  url \"…\"\n";
        assert_eq!(
            VersionSource::HomebrewTap.latest_in(formula).as_deref(),
            Some("0.7.7")
        );
        // PyPI: the newest release with live files, not a yanked one.
        let pypi = r#"{"info": {"version": "0.7.6"}, "releases": {
            "0.7.5": [{"yanked": false}],
            "0.7.6": [{"yanked": false}],
            "0.7.7": [{"yanked": true}],
            "0.8.0rc1": [{"yanked": false}],
            "0.7.10": []
        }}"#;
        assert_eq!(
            VersionSource::Pypi.latest_in(pypi).as_deref(),
            Some("0.7.6")
        );
        assert_eq!(
            VersionSource::Pypi
                .latest_in(r#"{"info": {"version": "0.7.7"}}"#)
                .as_deref(),
            Some("0.7.7")
        );
        let npm = r#"{"name": "@cohors/cli", "dist-tags": {"latest": "0.7.7"}, "versions": {}}"#;
        assert_eq!(VersionSource::Npm.latest_in(npm).as_deref(), Some("0.7.7"));
        let index = concat!(
            r#"{"name":"cohors","vers":"0.7.6","yanked":false}"#,
            "\n",
            r#"{"name":"cohors","vers":"0.7.8","yanked":true}"#,
            "\n",
            r#"{"name":"cohors","vers":"0.7.7","yanked":false}"#,
            "\n"
        );
        assert_eq!(
            VersionSource::CratesIo.latest_in(index).as_deref(),
            Some("0.7.7")
        );
        // Unreadable answers and pre-releases give nothing.
        assert_eq!(VersionSource::GithubRelease.latest_in("<html>"), None);
        assert_eq!(
            VersionSource::Npm.latest_in(r#"{"dist-tags": {"latest": "0.8.0-rc.1"}}"#),
            None
        );
    }

    #[test]
    fn the_test_seam_reads_any_shape() {
        assert_eq!(latest_in_any("0.7.8\n").as_deref(), Some("0.7.8"));
        assert_eq!(
            latest_in_any(r#"{"tag_name": "v0.7.8"}"#).as_deref(),
            Some("0.7.8")
        );
        assert_eq!(
            latest_in_any("  version \"0.7.8\"\n").as_deref(),
            Some("0.7.8")
        );
        assert_eq!(latest_in_any("nonsense"), None);
    }

    fn known<'a>(latest: Option<&'a str>) -> Known<'a> {
        Known {
            current: "0.7.7",
            latest,
            installed: None,
            failed: None,
            restarted_into: None,
            now: 1_000_000,
        }
    }

    #[test]
    fn auto_installs_on_channels_that_can_and_tells_the_rest() {
        let newer = known(Some("0.7.8"));
        for channel in [
            Channel::Managed,
            Channel::Homebrew,
            Channel::Pip { python: None },
            Channel::Installer,
        ] {
            assert_eq!(
                decide(&channel, true, &newer),
                Decision::Install("0.7.8".into()),
                "{channel:?}"
            );
        }
        for channel in [
            Channel::Npm,
            Channel::Cargo,
            Channel::UvTool,
            Channel::Source,
        ] {
            assert_eq!(
                decide(&channel, true, &newer),
                Decision::Report(UpdateState::Available {
                    version: "0.7.8".into(),
                    command: channel.manual_command().into(),
                }),
                "{channel:?}"
            );
        }
        // notify: everyone is only told.
        assert!(matches!(
            decide(&Channel::Managed, false, &newer),
            Decision::Report(UpdateState::Available { .. })
        ));
        // Nothing newer, or an unreadable answer: nothing.
        assert_eq!(
            decide(&Channel::Managed, true, &known(Some("0.7.7"))),
            Decision::Nothing
        );
        assert_eq!(
            decide(&Channel::Managed, true, &known(None)),
            Decision::Nothing
        );
    }

    #[test]
    fn an_installed_update_waits_for_the_restart() {
        let k = Known {
            installed: Some("0.7.8"),
            ..known(Some("0.7.8"))
        };
        assert_eq!(
            decide(&Channel::Managed, true, &k),
            Decision::Report(UpdateState::Ready {
                version: "0.7.8".into()
            })
        );
        // Still ready when the source cannot be reached right now.
        let offline = Known {
            installed: Some("0.7.8"),
            ..known(None)
        };
        assert!(matches!(
            decide(&Channel::Managed, true, &offline),
            Decision::Report(UpdateState::Ready { .. })
        ));
    }

    #[test]
    fn a_failed_install_is_not_retried_in_a_loop() {
        let failure = Failure {
            version: "0.7.8".into(),
            at: 1_000_000 - 60,
            reason: "checksum mismatch".into(),
        };
        let k = Known {
            failed: Some(&failure),
            ..known(Some("0.7.8"))
        };
        assert_eq!(
            decide(&Channel::Managed, true, &k),
            Decision::Report(UpdateState::Failed {
                version: "0.7.8".into(),
                command: "cohors update".into(),
                reason: "checksum mismatch".into(),
            })
        );
        // A day later it is tried again; a newer release is tried at once.
        let later = Known {
            now: failure.at + RETRY_FAILED_AFTER,
            ..k
        };
        assert_eq!(
            decide(&Channel::Managed, true, &later),
            Decision::Install("0.7.8".into())
        );
        let newer = Known {
            latest: Some("0.7.9"),
            ..k
        };
        assert_eq!(
            decide(&Channel::Managed, true, &newer),
            Decision::Install("0.7.9".into())
        );
    }

    #[test]
    fn a_restart_that_came_back_old_is_reported_not_repeated() {
        let k = Known {
            restarted_into: Some("0.7.8"),
            ..known(Some("0.7.8"))
        };
        let Decision::RestartMissed(failure) = decide(&Channel::Homebrew, true, &k) else {
            panic!("expected a missed restart");
        };
        assert_eq!(failure.version, "0.7.8");
        assert!(failure.reason.contains("still 0.7.7"), "{}", failure.reason);
    }

    #[test]
    fn one_off_commands_leave_installing_to_cohors_update() {
        let said = report_only(Decision::Install("0.7.8".into()), &Channel::Homebrew);
        assert_eq!(
            said,
            Some(UpdateState::Available {
                version: "0.7.8".into(),
                command: "cohors update".into()
            })
        );
        assert_eq!(report_only(Decision::Nothing, &Channel::Homebrew), None);
    }

    #[test]
    fn the_switches_say_why_updates_are_off() {
        let on = Switches {
            setting: Setting::Auto,
            disable_updates: false,
            no_update: false,
            ci: false,
        };
        assert_eq!(auto_update(&on, &Channel::Managed).0, Auto::Installs);
        assert_eq!(auto_update(&on, &Channel::Npm).0, Auto::Tells);
        assert!(auto_update(&on, &Channel::Npm).1.contains("npm i -g"));
        let ci = Switches { ci: true, ..on };
        assert_eq!(
            auto_update(&ci, &Channel::Managed),
            (Auto::Off, "running in CI".into())
        );
        let both = Switches {
            disable_updates: true,
            no_update: true,
            ..on
        };
        assert!(
            auto_update(&both, &Channel::Managed)
                .1
                .contains("COHORS_DISABLE_UPDATES")
        );
        let notify = Switches {
            setting: Setting::Notify,
            ..on
        };
        assert_eq!(auto_update(&notify, &Channel::Managed).0, Auto::Tells);
    }

    #[test]
    fn release_archives_are_named_like_cargo_dist_names_them() {
        assert_eq!(
            release_target("aarch64", "macos", false),
            Some("aarch64-apple-darwin")
        );
        assert_eq!(
            release_target("x86_64", "linux", false),
            Some("x86_64-unknown-linux-gnu")
        );
        // No musl or Windows build is published.
        assert_eq!(release_target("x86_64", "linux", true), None);
        assert_eq!(release_target("x86_64", "windows", false), None);
        assert_eq!(
            archive_name("aarch64-apple-darwin"),
            "cohors-aarch64-apple-darwin.tar.xz"
        );
    }

    #[test]
    fn checksum_files_are_read_the_way_cargo_dist_writes_them() {
        let hex = "db6f4b2095a17a1644088e397dec2560be8fa23ecba5c8bbbd4b0d071736e848";
        let file = "cohors-aarch64-apple-darwin.tar.xz";
        assert_eq!(
            checksum_in(&format!("{hex} *{file}\n"), file).as_deref(),
            Some(hex)
        );
        assert_eq!(
            checksum_in(&format!("{}  {file}\n", hex.to_uppercase()), file).as_deref(),
            Some(hex)
        );
        assert_eq!(checksum_in(hex, file).as_deref(), Some(hex));
        // Another file's sum, or a short one, is not this file's.
        assert_eq!(checksum_in(&format!("{hex} *other.tar.xz"), file), None);
        assert_eq!(checksum_in("abc123 *x", file), None);
    }

    #[test]
    fn the_prune_keeps_the_running_and_the_two_newest() {
        let installed: Vec<String> = ["0.7.4", "0.7.5", "0.7.6", "0.7.7", "0.7.8", "notes"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut gone = to_prune(&installed, &["0.7.8"], KEEP_NEWEST);
        gone.sort();
        assert_eq!(gone, ["0.7.4", "0.7.5", "0.7.6"]);
        // An old version that is still running is never deleted.
        let mut gone = to_prune(&installed, &["0.7.4"], KEEP_NEWEST);
        gone.sort();
        assert_eq!(gone, ["0.7.5", "0.7.6"]);
        assert!(to_prune(&installed[..2], &[], KEEP_NEWEST).is_empty());
    }

    #[test]
    fn the_launcher_has_to_come_first_on_the_path() {
        let path: Vec<String> = ["/opt/homebrew/bin", "/h/.local/bin", "/usr/bin"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            path_standing(
                &path,
                &["/opt/homebrew/bin".into(), "/h/.local/bin".into()],
                "/h/.local/bin/"
            ),
            PathStanding::Behind("/opt/homebrew/bin".into())
        );
        assert_eq!(
            path_standing(&path, &["/h/.local/bin".into()], "/h/.local/bin"),
            PathStanding::First
        );
        assert_eq!(
            path_standing(&path[..1], &[], "/h/.local/bin"),
            PathStanding::Missing
        );
    }

    #[test]
    fn the_notice_reads_the_same_everywhere() {
        let installing = UpdateState::Installing {
            version: "0.7.8".into(),
        };
        assert_eq!(
            installing.notice_in_dashboard(),
            "Update available: 0.7.8 — updating…"
        );
        assert_eq!(installing.notice(), installing.notice_in_dashboard());
        let ready = UpdateState::Ready {
            version: "0.7.8".into(),
        };
        assert_eq!(
            ready.notice_in_dashboard(),
            "Updated to 0.7.8 — restart to use it (R)"
        );
        assert_eq!(ready.notice(), "Updated to 0.7.8 — next run is 0.7.8");
        let told = UpdateState::Available {
            version: "0.7.8".into(),
            command: "npm i -g @cohors/cli@latest".into(),
        };
        assert_eq!(
            told.notice(),
            "Update available: 0.7.8 — run npm i -g @cohors/cli@latest"
        );
        assert_eq!(told.version(), "0.7.8");
        assert_eq!(told.detail(), None);
        let failed = UpdateState::Failed {
            version: "0.7.8".into(),
            command: "cohors update".into(),
            reason: "checksum mismatch".into(),
        };
        assert_eq!(failed.notice(), "Auto-update failed · run cohors doctor");
        assert_eq!(
            failed.detail().as_deref(),
            Some(
                "Installing 0.7.8 failed: checksum mismatch. To update by hand, run: cohors update"
            )
        );
    }

    fn facts<'a>(channel: &'a Channel, managed: Option<&'a str>) -> HandoffFacts<'a> {
        HandoffFacts {
            channel,
            dev_build: false,
            switches: Switches {
                setting: Setting::Auto,
                disable_updates: false,
                no_update: false,
                ci: false,
            },
            handed_off: false,
            current: "0.7.7",
            managed_active: managed,
        }
    }

    #[test]
    fn a_newer_managed_version_is_handed_to_and_an_older_one_is_not() {
        let brew = Channel::Homebrew;
        assert_eq!(
            hand_off_to(&facts(&brew, Some("0.7.8"))).as_deref(),
            Some("0.7.8")
        );
        assert_eq!(hand_off_to(&facts(&brew, Some("0.7.7"))), None);
        assert_eq!(hand_off_to(&facts(&brew, Some("0.7.6"))), None);
        assert_eq!(hand_off_to(&facts(&brew, None)), None);
        // Every released way of installing hands off.
        for channel in [
            Channel::Pip { python: None },
            Channel::Uvx,
            Channel::UvTool,
            Channel::Npm,
            Channel::Npx,
            Channel::Cargo,
            Channel::Installer,
        ] {
            assert!(
                hand_off_to(&facts(&channel, Some("0.7.8"))).is_some(),
                "{channel:?}"
            );
        }
    }

    #[test]
    fn the_loop_guard_dev_builds_ci_and_switches_stop_the_hand_off() {
        let brew = Channel::Homebrew;
        let newer = facts(&brew, Some("0.7.8"));
        let blocked = |f: HandoffFacts| (hand_off_to(&f), handoff_blocked(&f), sets_up(&f, false));
        let handed = HandoffFacts {
            handed_off: true,
            ..newer
        };
        assert_eq!(blocked(handed).0, None);
        assert!(blocked(handed).1.unwrap().contains("COHORS_HANDOFF"));
        let dev = HandoffFacts {
            dev_build: true,
            ..newer
        };
        assert_eq!(
            blocked(dev),
            (None, Some("a dev build never hands off"), false)
        );
        let ci = HandoffFacts {
            switches: Switches {
                ci: true,
                ..newer.switches
            },
            ..newer
        };
        assert_eq!(blocked(ci), (None, Some("running in CI"), false));
        for switches in [
            Switches {
                setting: Setting::Off,
                ..newer.switches
            },
            Switches {
                no_update: true,
                ..newer.switches
            },
            Switches {
                disable_updates: true,
                ..newer.switches
            },
        ] {
            let f = HandoffFacts { switches, ..newer };
            assert_eq!(hand_off_to(&f), None, "{switches:?}");
            assert!(!sets_up(&f, false), "{switches:?}");
        }
        // Source builds and the managed copy itself never hand off.
        for channel in [Channel::Source, Channel::Managed] {
            assert_eq!(hand_off_to(&facts(&channel, Some("0.7.8"))), None);
            assert!(!sets_up(&facts(&channel, None), false));
        }
    }

    #[test]
    fn the_first_run_sets_up_once_and_only_in_auto() {
        let brew = Channel::Homebrew;
        assert!(sets_up(&facts(&brew, None), false));
        assert!(!sets_up(&facts(&brew, None), true));
        let notify = HandoffFacts {
            switches: Switches {
                setting: Setting::Notify,
                ..facts(&brew, None).switches
            },
            ..facts(&brew, None)
        };
        // notify installs nothing, but a copy already set up is still used.
        assert!(!sets_up(&notify, false));
        let notify_newer = HandoffFacts {
            managed_active: Some("0.7.8"),
            ..notify
        };
        assert_eq!(hand_off_to(&notify_newer).as_deref(), Some("0.7.8"));
        assert_eq!(
            set_up_notice("~/.local/share/cohors"),
            "cohors keeps itself up to date from ~/.local/share/cohors now (cohors doctor for details)"
        );
    }

    #[test]
    fn a_copy_that_hands_off_updates_the_managed_copy_not_its_package_manager() {
        assert_eq!(updates_through(&Channel::Homebrew, true), Channel::Managed);
        assert_eq!(
            updates_through(&Channel::Homebrew, false),
            Channel::Homebrew
        );
        assert_eq!(updates_through(&Channel::Source, true), Channel::Source);
        assert_eq!(
            Channel::Npm.uninstall_command(),
            Some("npm rm -g @cohors/cli")
        );
        assert_eq!(
            Channel::Homebrew.uninstall_command(),
            Some("brew uninstall cohors")
        );
        assert_eq!(Channel::Npx.uninstall_command(), None);
    }
}
