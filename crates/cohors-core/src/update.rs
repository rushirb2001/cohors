//! Keeping cohors itself up to date: the pure half.
//!
//! Comparing versions, recognising how this copy of cohors was installed, and
//! the words every surface shows. No I/O lives here (the release lookup and the
//! install run in the binary), so all of it is testable and WASM-safe.

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

/// How this copy of cohors got onto the machine, which decides how it updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Channel {
    /// A Homebrew formula (the binary lives under `Cellar/cohors/`).
    Homebrew,
    /// `pip install cohors`. `python` is the interpreter whose pip owns it,
    /// when known.
    Pip { python: Option<String> },
    /// Run through `uvx`, from uv's cache. Nothing to install: `uvx
    /// cohors@latest` fetches the newest release when it next starts.
    Uvx,
    /// `npm install -g cohors`. The npm package keeps the binary inside
    /// itself (`node_modules/cohors/node_modules/.bin_real/`). Only told, never
    /// updated in place: a global npm prefix often needs elevated rights.
    Npm,
    /// Run through `npx`, from npm's cache. Like uvx, `npx cohors@latest`
    /// fetches the newest release when it next starts.
    Npx,
    /// The release's shell installer, which leaves an install receipt.
    Installer,
    /// Built from source (`cargo install`, a checkout). Never updated
    /// automatically: whoever built it chose what to build.
    Source,
}

/// Recognise the channel from the running binary's path and what the launcher
/// said. `launcher` is `COHORS_INSTALLER` (the pip wrapper sets `pip`, or
/// `uvx` when it runs from uv's cache); `receipt` is whether the shell
/// installer's receipt exists.
pub fn detect_channel(
    exe: &str,
    launcher: Option<&str>,
    python: Option<&str>,
    receipt: bool,
) -> Channel {
    let exe = exe.replace('\\', "/");
    let python = python.map(str::to_string);
    if launcher == Some("uvx") || exe.contains("/uv/archive-") || exe.contains("/uv/tools/") {
        return Channel::Uvx;
    }
    if launcher == Some("pip") || exe.contains("/site-packages/cohors/") {
        return Channel::Pip { python };
    }
    // npx runs from npm's cache (`_npx/<hash>/node_modules/cohors/`), so test
    // for it before the plain npm install it otherwise looks like.
    if exe.contains("/_npx/") {
        return Channel::Npx;
    }
    if exe.contains("/node_modules/cohors/") {
        return Channel::Npm;
    }
    if exe.contains("/Cellar/cohors/") {
        return Channel::Homebrew;
    }
    if receipt {
        return Channel::Installer;
    }
    Channel::Source
}

impl Channel {
    /// Can cohors install an update for this channel by itself?
    pub fn installs_itself(&self) -> bool {
        matches!(
            self,
            Channel::Homebrew | Channel::Pip { .. } | Channel::Installer
        )
    }

    /// How this channel is named to a person: "installed with {label}".
    pub fn label(&self) -> &'static str {
        match self {
            Channel::Homebrew => "Homebrew",
            Channel::Pip { .. } => "pip",
            Channel::Uvx => "uvx",
            Channel::Npm => "npm",
            Channel::Npx => "npx",
            Channel::Installer => "the shell installer",
            Channel::Source => "a build from source",
        }
    }

    /// The command a person would run to update, for the notice when cohors
    /// will not (or could not) do it itself.
    pub fn manual_command(&self) -> &'static str {
        match self {
            Channel::Homebrew => "brew upgrade cohors",
            Channel::Pip { .. } => "pip install -U cohors",
            Channel::Uvx => "uvx cohors@latest",
            Channel::Npm => "npm install -g cohors@latest",
            Channel::Npx => "npx cohors@latest",
            Channel::Installer => {
                "curl --proto '=https' --tlsv1.2 -LsSf https://github.com/rushirb2001/cohors/releases/latest/download/cohors-tui-installer.sh | sh"
            }
            Channel::Source => {
                "cargo install --git https://github.com/rushirb2001/cohors cohors-tui"
            }
        }
    }
}

/// Where an update stands, as every surface shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum UpdateState {
    /// A newer release exists. `command` is how to get it when cohors will not
    /// install it itself (updates switched to notify-only, a source build, uvx,
    /// npm or npx).
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

    /// The one-line notice, the same words on every surface.
    pub fn notice(&self) -> String {
        match self {
            UpdateState::Available { version, command } => {
                format!("New version available → {version} → {command}")
            }
            UpdateState::Installing { version } => {
                format!("New version available → {version} → installing…")
            }
            UpdateState::Ready { version } => {
                format!("New version available → {version} → Restart to update")
            }
            UpdateState::Failed {
                version, command, ..
            } => {
                format!("New version available → {version} → could not install, run {command}")
            }
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

    #[test]
    fn the_channel_follows_where_the_binary_lives() {
        let detect = |exe: &str| detect_channel(exe, None, None, false);
        assert_eq!(
            detect("/opt/homebrew/Cellar/cohors/0.7.0/bin/cohors"),
            Channel::Homebrew
        );
        assert_eq!(
            detect("/home/linuxbrew/.linuxbrew/Cellar/cohors/0.7.0/bin/cohors"),
            Channel::Homebrew
        );
        assert_eq!(
            detect("/Users/a/.venv/lib/python3.12/site-packages/cohors/cohors"),
            Channel::Pip { python: None }
        );
        assert_eq!(
            detect("/Users/a/.cache/uv/archive-v0/abc/lib/python3.12/site-packages/cohors/cohors"),
            Channel::Uvx
        );
        assert_eq!(
            detect("/opt/homebrew/lib/node_modules/cohors/node_modules/.bin_real/cohors"),
            Channel::Npm
        );
        assert_eq!(
            detect("/Users/a/.npm/_npx/3f2a/node_modules/cohors/node_modules/.bin_real/cohors"),
            Channel::Npx
        );
        assert_eq!(detect("/Users/a/.cargo/bin/cohors"), Channel::Source);
        assert_eq!(
            detect_channel("/Users/a/.cargo/bin/cohors", None, None, true),
            Channel::Installer
        );
    }

    #[test]
    fn what_the_launcher_says_wins() {
        assert_eq!(
            detect_channel("/x/cohors", Some("pip"), Some("/v/bin/python3"), true),
            Channel::Pip {
                python: Some("/v/bin/python3".into())
            }
        );
        assert_eq!(
            detect_channel("/x/cohors", Some("uvx"), None, false),
            Channel::Uvx
        );
    }

    #[test]
    fn only_package_managers_and_the_installer_update_themselves() {
        assert!(Channel::Homebrew.installs_itself());
        assert!(Channel::Installer.installs_itself());
        assert!(Channel::Pip { python: None }.installs_itself());
        assert!(!Channel::Uvx.installs_itself());
        assert!(!Channel::Npm.installs_itself());
        assert!(!Channel::Npx.installs_itself());
        assert!(!Channel::Source.installs_itself());
    }

    #[test]
    fn the_notice_reads_the_same_everywhere() {
        let ready = UpdateState::Ready {
            version: "0.7.1".into(),
        };
        assert_eq!(
            ready.notice(),
            "New version available → 0.7.1 → Restart to update"
        );
        let available = UpdateState::Available {
            version: "0.7.1".into(),
            command: "brew upgrade cohors".into(),
        };
        assert_eq!(
            available.notice(),
            "New version available → 0.7.1 → brew upgrade cohors"
        );
        assert_eq!(available.version(), "0.7.1");
    }
}
