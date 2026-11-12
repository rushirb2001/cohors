//! Keeping cohors itself up to date: looking up the newest release, installing
//! it the way this copy was installed, and restarting into it.
//!
//! The pure decisions (comparing versions, recognising the install channel,
//! the notice's words) live in `cohors_core::update`; this is the I/O around
//! them. Everything is best-effort: a lookup or install that fails is reported
//! as a notice, never as an error that stops cohors.
//!
//! The only connection made is one request to GitHub's public releases API, at
//! most once a day, sending nothing but cohors' version in the User-Agent.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use camino::Utf8PathBuf;
use cohors_config::UpdatesMode;
use cohors_core::update::{Channel, UpdateState, detect_channel, is_newer};
use serde::{Deserialize, Serialize};

/// This build's version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "rushirb2001/cohors";
/// Look again after a day.
const CHECK_EVERY: i64 = 24 * 60 * 60;
/// After a failed lookup (offline, rate-limited), try again in an hour.
const RETRY_AFTER: i64 = 60 * 60;
/// An install that has not finished in this long is treated as abandoned.
const LOCK_STALE: Duration = Duration::from_secs(15 * 60);

/// What cohors remembers between runs, in `<cache>/update.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Memo {
    /// When GitHub was last asked (unix seconds).
    checked_at: i64,
    /// The newest release GitHub reported, e.g. `0.7.1`.
    latest: Option<String>,
    /// A version this machine already installed and is waiting for a restart.
    installed: Option<String>,
    /// The last install that failed: which version, when, and why. It is not
    /// tried again for a day, so a broken install never runs a package manager
    /// every time the dashboard opens.
    #[serde(default)]
    failed: Option<Failure>,
    /// The version a restart was meant to start. If the cohors that comes up
    /// is still older, the install went somewhere this copy does not run from.
    #[serde(default)]
    restarted_into: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Failure {
    version: String,
    at: i64,
    reason: String,
}

fn memo_path() -> Option<Utf8PathBuf> {
    cohors_config::paths::cache_dir()
        .ok()
        .map(|dir| dir.join("update.json"))
}

fn load_memo() -> Memo {
    memo_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_memo(memo: &Memo) {
    let Some(path) = memo_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec_pretty(memo) {
        let _ = std::fs::write(path, json);
    }
}

/// The mode in force: the config's, unless the environment says to stay quiet.
/// `COHORS_NO_UPDATE` (any value but `0`) and CI both mean off.
pub fn effective_mode(configured: UpdatesMode) -> UpdatesMode {
    mode_given(configured, |name| std::env::var(name).ok())
}

/// [`effective_mode`] with the environment passed in, so it can be tested.
fn mode_given(configured: UpdatesMode, env: impl Fn(&str) -> Option<String>) -> UpdatesMode {
    let set = |name: &str| env(name).is_some_and(|v| !v.is_empty() && v != "0");
    if set("COHORS_NO_UPDATE") || set("CI") {
        UpdatesMode::Off
    } else {
        configured
    }
}

/// How this copy of cohors was installed.
pub fn channel() -> Channel {
    let exe = std::env::current_exe()
        .ok()
        .map(|path| path.canonicalize().unwrap_or(path))
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let launcher = std::env::var("COHORS_INSTALLER").ok();
    let python = std::env::var("COHORS_PYTHON").ok();
    detect_channel(
        &exe,
        launcher.as_deref(),
        python.as_deref(),
        installer_owns(&exe),
        cargo_installed(&exe),
    )
}

/// The shell installer's receipts, newest name first. The installer was named
/// after the crate, which was `cohors-tui` up to 0.7.2 and is `cohors` since.
fn receipt_paths() -> Vec<PathBuf> {
    let Some(base) = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))
    else {
        return Vec::new();
    };
    vec![
        base.join("cohors").join("cohors-receipt.json"),
        base.join("cohors-tui").join("cohors-tui-receipt.json"),
    ]
}

#[derive(Deserialize)]
struct Receipt {
    install_prefix: String,
    #[serde(default)]
    install_layout: String,
    #[serde(default)]
    version: String,
}

fn read_receipt() -> Option<Receipt> {
    receipt_paths().into_iter().find_map(|path| {
        let bytes = std::fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    })
}

/// Did the shell installer put *this* binary here? A receipt alone is not
/// enough: a source build of cohors can sit in the same `~/.cargo/bin`. The
/// binary has to be under the receipt's prefix, and be the version it records.
fn installer_owns(exe: &str) -> bool {
    read_receipt().is_some_and(|receipt| {
        !receipt.install_prefix.is_empty()
            && exe.starts_with(&receipt.install_prefix)
            && receipt.version == CURRENT
    })
}

/// Did `cargo install cohors` put *this* binary here, from crates.io? Cargo
/// records every install in `$CARGO_HOME/.crates2.json`, keyed like
/// `cohors 0.7.3 (registry+https://github.com/rust-lang/crates.io-index)`.
fn cargo_installed(exe: &str) -> bool {
    let Some(home) = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cargo")))
    else {
        return false;
    };
    let home = home.canonicalize().unwrap_or(home);
    if !Path::new(exe).starts_with(home.join("bin")) {
        return false;
    }
    std::fs::read(home.join(".crates2.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|record| crates_io_install_of(&record, CURRENT))
}

/// Does cargo's install record list `cohors` at `version` from crates.io?
fn crates_io_install_of(record: &serde_json::Value, version: &str) -> bool {
    let wanted = format!("cohors {version} (registry+");
    record["installs"]
        .as_object()
        .is_some_and(|installs| installs.keys().any(|key| key.starts_with(&wanted)))
}

/// Ask GitHub for the newest release's version (without its `v`).
fn latest_release() -> Result<String> {
    // Overridable so the whole flow can be tried against a local server.
    let url = std::env::var("COHORS_UPDATE_URL")
        .unwrap_or_else(|_| format!("https://api.github.com/repos/{REPO}/releases/latest"));
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build();
    let body: serde_json::Value = agent
        .get(&url)
        .set("User-Agent", &format!("cohors/{CURRENT}"))
        .set("Accept", "application/vnd.github+json")
        .call()
        .context("asking GitHub for the latest release")?
        .into_json()
        .context("reading GitHub's answer")?;
    let tag = body["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("GitHub's answer has no tag_name"))?;
    Ok(tag.trim_start_matches('v').to_string())
}

/// A newer release than this one, if there is one. Asks GitHub only when the
/// last answer is over a day old (or `force`); otherwise uses that answer.
pub fn newer_release(force: bool, now: i64) -> Option<String> {
    let mut memo = load_memo();
    if force || now - memo.checked_at >= CHECK_EVERY {
        match latest_release() {
            Ok(version) => {
                memo.latest = Some(version);
                memo.checked_at = now;
            }
            Err(err) => {
                tracing::debug!("update check failed: {err:#}");
                // Keep the last answer, and try again in an hour.
                memo.checked_at = now - CHECK_EVERY + RETRY_AFTER;
            }
        }
        save_memo(&memo);
    }
    memo.latest.filter(|latest| is_newer(latest, CURRENT))
}

/// What the last lookup said, without asking GitHub: for the commands and the
/// MCP server, which should never wait on the network for this.
pub fn remembered(mode: UpdatesMode) -> Option<UpdateState> {
    if mode == UpdatesMode::Off {
        return None;
    }
    let memo = load_memo();
    if let Some(installed) = memo.installed.filter(|v| is_newer(v, CURRENT)) {
        return Some(UpdateState::Ready { version: installed });
    }
    let latest = memo.latest.filter(|v| is_newer(v, CURRENT))?;
    Some(UpdateState::Available {
        version: latest,
        command: channel().manual_command().to_string(),
    })
}

/// The whole background job the dashboard starts: look up the newest release
/// and, in `auto` mode, install it. Every state change goes to `report`.
pub fn run_in_background(mode: UpdatesMode, now: i64, report: impl Fn(UpdateState)) {
    if mode == UpdatesMode::Off {
        return;
    }
    let Some(version) = newer_release(false, now) else {
        return;
    };
    let channel = channel();
    let command = channel.manual_command().to_string();
    // A restart was meant to start `version`, yet this is still the old one:
    // the install landed somewhere this copy is not run from. Installing again
    // would only repeat that, so say so once and stop for the day.
    let mut memo = load_memo();
    if memo.restarted_into.take().as_deref() == Some(version.as_str()) {
        let reason =
            format!("{version} was installed, but the cohors that started is still {CURRENT}");
        memo.failed = Some(Failure {
            version: version.clone(),
            at: now,
            reason: reason.clone(),
        });
        save_memo(&memo);
        report(UpdateState::Failed {
            version,
            command,
            reason,
        });
        return;
    }
    if mode == UpdatesMode::Notify || !channel.installs_itself() {
        report(UpdateState::Available { version, command });
        return;
    }
    // Already installed by an earlier run: only the restart is left.
    if load_memo().installed.as_deref() == Some(version.as_str()) {
        report(UpdateState::Ready { version });
        return;
    }
    // Failed recently: say so again, but leave the package manager alone
    // until a day has passed.
    if let Some(failure) = load_memo()
        .failed
        .filter(|f| f.version == version && now - f.at < CHECK_EVERY)
    {
        report(UpdateState::Failed {
            version,
            command,
            reason: failure.reason,
        });
        return;
    }
    report(UpdateState::Installing {
        version: version.clone(),
    });
    match install(&channel, &version) {
        Ok(()) => report(UpdateState::Ready { version }),
        Err(err) => {
            let reason = format!("{err:#}");
            let mut memo = load_memo();
            memo.failed = Some(Failure {
                version: version.clone(),
                at: now,
                reason: reason.clone(),
            });
            save_memo(&memo);
            report(UpdateState::Failed {
                version,
                command,
                reason,
            });
        }
    }
}

/// Install `version` the way this copy was installed, then remember that it
/// is waiting for a restart. Holds a lock so two open dashboards never run two
/// installs at once.
pub fn install(channel: &Channel, version: &str) -> Result<()> {
    let _lock = Lock::take().ok_or_else(|| anyhow!("another cohors is already updating"))?;
    match channel {
        Channel::Homebrew => run_quietly(
            Command::new("brew")
                .args(["upgrade", "rushirb2001/tap/cohors"])
                .env("HOMEBREW_NO_ENV_HINTS", "1"),
        )?,
        Channel::Pip { python } => {
            let python = python
                .clone()
                .or_else(python_beside_package)
                .ok_or_else(|| anyhow!("could not find the Python that installed cohors"))?;
            run_quietly(Command::new(python).args([
                "-m",
                "pip",
                "install",
                "--upgrade",
                "--disable-pip-version-check",
                "--quiet",
                &format!("cohors=={version}"),
            ]))?
        }
        Channel::Installer => run_installer(version)?,
        Channel::Uvx | Channel::Npm | Channel::Npx | Channel::Cargo | Channel::Source => {
            return Err(anyhow!("this copy of cohors does not update itself"));
        }
    }
    let mut memo = load_memo();
    memo.installed = Some(version.to_string());
    memo.failed = None;
    save_memo(&memo);
    Ok(())
}

/// For a pip install, the interpreter next to the package: the binary sits at
/// `<prefix>/lib/pythonX.Y/site-packages/cohors/cohors`, so it is
/// `<prefix>/bin/python3`.
fn python_beside_package() -> Option<String> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let prefix = exe.ancestors().nth(5)?;
    ["python3", "python"]
        .iter()
        .map(|name| prefix.join("bin").join(name))
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

/// Run the release's own shell installer for `version`, quietly, into the
/// place the first install chose. It checks the download's checksum itself.
fn run_installer(version: &str) -> Result<()> {
    let url = format!("https://github.com/{REPO}/releases/download/v{version}/cohors-installer.sh");
    let script = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build()
        .get(&url)
        .set("User-Agent", &format!("cohors/{CURRENT}"))
        .call()
        .context("downloading the installer")?
        .into_string()
        .context("reading the installer")?;
    // The script goes to `sh` on its standard input, exactly as `curl … | sh`
    // runs it. Nothing is written to disk, so there is no file another user
    // of the machine could plant or swap before it runs.
    let mut command = Command::new("sh");
    command
        .args(["-s", "--", "--quiet"])
        // The first install already put cohors on the PATH.
        .env("COHORS_NO_MODIFY_PATH", "1");
    // A first install into a chosen folder ("flat" layout) goes back there;
    // the default layout finds its own way to the same place.
    if let Some(receipt) = read_receipt()
        && receipt.install_layout == "flat"
    {
        command.env("COHORS_INSTALL_DIR", &receipt.install_prefix);
    }
    run_with_input(&mut command, Some(script.as_bytes()))
}

/// Run a command with no input and its output kept, failing with the last few
/// lines of what it printed.
fn run_quietly(command: &mut Command) -> Result<()> {
    run_with_input(command, None)
}

/// [`run_quietly`], optionally feeding `input` to the command's standard input.
fn run_with_input(command: &mut Command, input: Option<&[u8]>) -> Result<()> {
    use std::io::Write;
    let program = command.get_program().to_owned();
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {program:?}"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin
            .write_all(input)
            .with_context(|| format!("handing the script to {program:?}"))?;
        // Dropping stdin closes it, so the shell sees the end of the script.
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("waiting for {program:?}"))?;
    if output.status.success() {
        return Ok(());
    }
    let printed = String::from_utf8_lossy(&output.stderr);
    let tail: Vec<&str> = printed.lines().rev().take(3).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    Err(anyhow!("{}", tail.join(" ").trim()))
}

/// A lock file in the cache folder, removed when dropped.
struct Lock(PathBuf);

impl Lock {
    fn take() -> Option<Lock> {
        let path = cohors_config::paths::cache_dir()
            .ok()?
            .join("update.lock")
            .into_std_path_buf();
        let _ = std::fs::create_dir_all(path.parent()?);
        // A lock left behind by a crash does not block updates forever.
        if let Ok(meta) = std::fs::metadata(&path)
            && meta
                .modified()
                .ok()
                .and_then(|at| at.elapsed().ok())
                .is_some_and(|age| age > LOCK_STALE)
        {
            let _ = std::fs::remove_file(&path);
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .ok()
            .map(|_| Lock(path))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `cohors update`: look now, and install a newer release unless `check_only`.
/// Run by hand, so it ignores the `updates` setting.
pub fn run_command(check_only: bool) -> Result<()> {
    let channel = channel();
    println!("cohors {CURRENT}, installed with {}.", channel.label());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let Some(version) = newer_release(true, now) else {
        if load_memo().checked_at == now {
            println!("This is the newest release.");
        } else {
            println!("Could not reach GitHub to look for a newer release. Try again later.");
        }
        return Ok(());
    };
    let command = channel.manual_command();
    // Why the dashboard's last background attempt failed, in full: the header
    // has room only for the short notice.
    if let Some(failure) = load_memo().failed.filter(|f| f.version == version) {
        println!(
            "The last try to install {version} failed: {}",
            failure.reason
        );
    }
    if check_only || !channel.installs_itself() {
        println!("New version available → {version} → {command}");
        return Ok(());
    }
    println!("New version available → {version} → installing…");
    match install(&channel, &version) {
        Ok(()) => {
            // Nothing is left to restart: the next `cohors` is the new one.
            let mut memo = load_memo();
            memo.installed = None;
            save_memo(&memo);
            println!(
                "Updated to {version}. The next time you start cohors, it is the new version."
            );
            Ok(())
        }
        Err(err) => Err(err.context(format!(
            "installing {version}; to do it by hand, run: {command}"
        ))),
    }
}

/// After a one-off command, one line on stderr when a newer cohors is known.
/// It never asks GitHub (that would slow every command down); it repeats what
/// the dashboard's daily look found. Quiet when stderr is not a terminal.
pub fn print_notice(configured: UpdatesMode) {
    use std::io::IsTerminal;
    if !std::io::stderr().is_terminal() {
        return;
    }
    if let Some(state) = remembered(effective_mode(configured)) {
        eprintln!("\n  {}", state.notice());
    }
}

/// Start cohors again, the same way it was started, so the updated binary
/// runs. Call it with the terminal restored. Only returns if that failed.
pub fn restart() -> anyhow::Error {
    let mut args = std::env::args_os();
    let first = args.next().map(PathBuf::from);
    // Started by a path (`/opt/homebrew/bin/cohors`, the pip package's binary):
    // use it if it still exists. Started as plain `cohors`: let the PATH find
    // the new one. After a Homebrew upgrade the old versioned folder is gone,
    // so a path that vanished also falls back to the PATH.
    let program = match first {
        Some(path) if path.components().count() > 1 && path.exists() => path,
        _ => PathBuf::from("cohors"),
    };
    let mut command = Command::new(&program);
    command.args(args);
    // The update has been installed, so the next run should not announce it,
    // but it can check that it really is the new version.
    let mut memo = load_memo();
    memo.restarted_into = memo.installed.take();
    save_memo(&memo);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        anyhow!(command.exec()).context(format!("restarting {}", program.display()))
    }
    #[cfg(not(unix))]
    {
        match command.status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(0)),
            Err(err) => anyhow!(err).context(format!("restarting {}", program.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crates_io_install_is_recognised_by_name_and_version() {
        let record = serde_json::json!({
            "installs": {
                "cohors 0.7.3 (registry+https://github.com/rust-lang/crates.io-index)": {"bins": ["cohors"]},
                "ripgrep 14.1.1 (registry+https://github.com/rust-lang/crates.io-index)": {"bins": ["rg"]}
            }
        });
        assert!(crates_io_install_of(&record, "0.7.3"));
        // Another version, a git install or another crate is not this binary.
        assert!(!crates_io_install_of(&record, "0.7.2"));
        let from_git = serde_json::json!({
            "installs": {"cohors 0.7.3 (git+https://github.com/rushirb2001/cohors#abc)": {}}
        });
        assert!(!crates_io_install_of(&from_git, "0.7.3"));
        assert!(!crates_io_install_of(&serde_json::json!({}), "0.7.3"));
    }

    #[test]
    fn quiet_environments_switch_updates_off() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        assert_eq!(mode_given(UpdatesMode::Auto, env(&[])), UpdatesMode::Auto);
        assert_eq!(
            mode_given(UpdatesMode::Auto, env(&[("COHORS_NO_UPDATE", "1")])),
            UpdatesMode::Off
        );
        assert_eq!(
            mode_given(UpdatesMode::Auto, env(&[("CI", "true")])),
            UpdatesMode::Off
        );
        // `0` and empty mean "not set".
        assert_eq!(
            mode_given(
                UpdatesMode::Notify,
                env(&[("COHORS_NO_UPDATE", "0"), ("CI", "")])
            ),
            UpdatesMode::Notify
        );
    }

    #[test]
    fn a_script_runs_from_standard_input_with_its_arguments() {
        // The installer's path: the script arrives on stdin, never on disk,
        // and still sees the arguments after `--`.
        let script = b"[ \"$1\" = --quiet ] || exit 7\nexit 0\n";
        run_with_input(
            Command::new("sh").args(["-s", "--", "--quiet"]),
            Some(script),
        )
        .unwrap();
        let err = run_with_input(
            Command::new("sh").args(["-s", "--", "--loud"]),
            Some(b"[ \"$1\" = --quiet ] || { echo wrong flag >&2; exit 7; }\n"),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "wrong flag");
    }

    #[test]
    fn a_failed_command_reports_its_last_words() {
        let err =
            run_quietly(Command::new("sh").args(["-c", "echo one >&2; echo two >&2; exit 3"]))
                .unwrap_err();
        assert_eq!(err.to_string(), "one two");
    }
}
