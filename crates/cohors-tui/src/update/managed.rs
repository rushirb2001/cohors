//! cohors' own install, which updates itself the way Claude Code's native
//! install does.
//!
//! Each release lives in its own folder, `<data>/cohors/versions/<version>/
//! cohors` (`<data>` is `$XDG_DATA_HOME`, by default `~/.local/share`), and the
//! launcher `~/.local/bin/cohors` is a symlink to the active one. An update
//! downloads this platform's release archive into a staging folder in the
//! cache, checks it against the release's published SHA-256 (three tries),
//! unpacks it, runs `<new> --version`, moves it into `versions/`, then
//! repoints the launcher atomically (a new link, renamed over the old). It
//! keeps the running version and the two newest; nothing else is ever deleted
//! from `versions/`. A lock file in `<state>/cohors/locks/` keeps two cohors
//! processes from updating at once, and the last attempt is recorded for
//! `cohors doctor`.
//!
//! The test seam: `COHORS_UPDATE_RELEASES` points the downloads at a local
//! folder (or another URL) laid out like the release host,
//! `<base>/v<version>/<asset>`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use cohors_core::update::{
    KEEP_NEWEST, Version, archive_name, checksum_in, release_target, to_prune,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::lookup::{local_path, user_agent};

/// The binary's file name.
pub(crate) const EXE: &str = if cfg!(windows) {
    "cohors.exe"
} else {
    "cohors"
};
/// How many times a download is tried before the update gives up.
const TRIES: u32 = 3;
/// A release archive is a few MB; anything this big is not one.
const MAX_ARCHIVE: u64 = 256 * 1024 * 1024;
/// An update that has not finished in this long is treated as abandoned.
const LOCK_STALE: Duration = Duration::from_secs(15 * 60);

/// Where everything update-related lives. From the environment in normal use;
/// a temporary folder in tests.
#[derive(Debug, Clone)]
pub(crate) struct Layout {
    pub home: PathBuf,
    /// `$XDG_DATA_HOME/cohors`: the managed versions.
    pub data: PathBuf,
    /// `$XDG_BIN_HOME`, or `~/.local/bin`: the launcher.
    pub bin: PathBuf,
    /// `$XDG_STATE_HOME/cohors`: the lock and the last attempt.
    pub state: PathBuf,
    /// cohors' cache folder: the memo and the staging folder.
    pub cache: PathBuf,
    /// `$XDG_CONFIG_HOME`, or `~/.config`: where the shell installer leaves
    /// its receipt.
    pub config_home: PathBuf,
    /// `$CARGO_HOME`, or `~/.cargo`: cargo's record of installs.
    pub cargo_home: PathBuf,
}

impl Layout {
    pub fn from_env() -> Option<Layout> {
        let home = cohors_config::paths::home_dir().ok()?.into_std_path_buf();
        let cache = cohors_config::paths::cache_dir().ok()?.into_std_path_buf();
        // An XDG variable only counts when it is an absolute path, as the
        // spec says.
        let var = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        Some(Layout {
            data: var("XDG_DATA_HOME")
                .unwrap_or_else(|| home.join(".local").join("share"))
                .join("cohors"),
            bin: var("XDG_BIN_HOME").unwrap_or_else(|| home.join(".local").join("bin")),
            state: var("XDG_STATE_HOME")
                .unwrap_or_else(|| home.join(".local").join("state"))
                .join("cohors"),
            config_home: var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")),
            cargo_home: var("CARGO_HOME").unwrap_or_else(|| home.join(".cargo")),
            cache,
            home,
        })
    }

    /// Everything under one folder, for tests.
    #[cfg(test)]
    pub fn under(root: &Path) -> Layout {
        Layout {
            home: root.to_path_buf(),
            data: root.join(".local/share/cohors"),
            bin: root.join(".local/bin"),
            state: root.join(".local/state/cohors"),
            cache: root.join(".cache/cohors"),
            config_home: root.join(".config"),
            cargo_home: root.join(".cargo"),
        }
    }

    pub fn versions(&self) -> PathBuf {
        self.data.join("versions")
    }

    pub fn launcher(&self) -> PathBuf {
        self.bin.join(EXE)
    }

    pub fn memo(&self) -> PathBuf {
        self.cache.join("update.json")
    }

    pub(crate) fn lock_file(&self) -> PathBuf {
        self.state.join("locks").join("update.lock")
    }

    pub(crate) fn last_attempt_file(&self) -> PathBuf {
        self.state.join("last-update.json")
    }

    /// What the hand-off remembers: the first-run setup, the last binary that
    /// passed its check, the last time it fell back.
    pub(crate) fn handoff_file(&self) -> PathBuf {
        self.state.join("handoff.json")
    }

    pub(crate) fn staging(&self) -> PathBuf {
        self.cache.join("update-staging")
    }

    /// A path for a person: `~/…` under the home folder, whether the path
    /// came with symlinks resolved (macOS's `/tmp` is `/private/tmp`) or not.
    pub fn show(&self, path: &Path) -> String {
        if self.home.as_os_str().is_empty() {
            return path.display().to_string();
        }
        let real_home = std::fs::canonicalize(&self.home).unwrap_or(self.home.clone());
        match path
            .strip_prefix(&self.home)
            .or_else(|_| path.strip_prefix(&real_home))
        {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        }
    }
}

/// Where release files come from.
#[derive(Debug, Clone)]
pub(crate) enum Origin {
    /// `<base>/v<version>/<asset>` on the web; GitHub's release downloads.
    Web(String),
    /// The same layout in a local folder (the test seam).
    Dir(PathBuf),
}

impl Origin {
    pub fn from_env() -> Origin {
        match std::env::var("COHORS_UPDATE_RELEASES")
            .ok()
            .filter(|v| !v.trim().is_empty())
        {
            Some(location) => match local_path(&location) {
                Some(dir) => Origin::Dir(dir),
                None => Origin::Web(location.trim_end_matches('/').to_string()),
            },
            None => Origin::Web(format!(
                "https://github.com/{}/releases/download",
                super::REPO
            )),
        }
    }

    /// One release file, at most `limit` bytes.
    fn get(&self, version: &str, asset: &str, limit: u64) -> Result<Vec<u8>> {
        match self {
            Origin::Dir(dir) => {
                let path = dir.join(format!("v{version}")).join(asset);
                std::fs::read(&path).with_context(|| format!("reading {}", path.display()))
            }
            Origin::Web(base) => {
                let url = format!("{base}/v{version}/{asset}");
                let response = ureq::AgentBuilder::new()
                    .timeout_connect(Duration::from_secs(10))
                    .timeout(Duration::from_secs(120))
                    .build()
                    .get(&url)
                    .set("User-Agent", &user_agent())
                    .call()
                    .with_context(|| format!("downloading {asset}"))?;
                let mut bytes = Vec::new();
                response
                    .into_reader()
                    .take(limit + 1)
                    .read_to_end(&mut bytes)
                    .with_context(|| format!("downloading {asset}"))?;
                if bytes.len() as u64 > limit {
                    bail!("{asset} is larger than any cohors release");
                }
                Ok(bytes)
            }
        }
    }

    fn is_web(&self) -> bool {
        matches!(self, Origin::Web(_))
    }
}

/// Another cohors holds the update lock: this one skips, and that one's
/// result shows up on the next look.
#[derive(Debug)]
pub(crate) struct Busy;

impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("another cohors is updating right now")
    }
}

impl std::error::Error for Busy {}

/// The update lock, removed when dropped.
pub(crate) struct Lock(PathBuf);

impl Lock {
    /// Take the lock, or `None` when another cohors holds it. A lock left
    /// behind by a crash stops counting after [`LOCK_STALE`].
    pub fn take(layout: &Layout) -> Option<Lock> {
        let path = layout.lock_file();
        let _ = std::fs::create_dir_all(path.parent()?);
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

/// The last update attempt, for `cohors doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Attempt {
    pub at: i64,
    pub from: String,
    pub to: String,
    /// The channel's label: how the install was attempted.
    pub channel: String,
    pub ok: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

pub(crate) fn record(layout: &Layout, attempt: &Attempt) {
    if let Ok(json) = serde_json::to_vec_pretty(attempt) {
        let _ = write_atomically(&layout.last_attempt_file(), &json);
    }
}

pub(crate) fn last_attempt(layout: &Layout) -> Option<Attempt> {
    let bytes = std::fs::read(layout.last_attempt_file()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Write a file whole or not at all: a reader never sees half of it.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// The release target this build downloads.
pub(crate) fn this_target() -> Option<&'static str> {
    release_target(
        std::env::consts::ARCH,
        std::env::consts::OS,
        cfg!(target_env = "musl"),
    )
}

/// What an update put where.
#[derive(Debug)]
pub(crate) struct Installed {
    pub binary: PathBuf,
    pub launcher: PathBuf,
    /// Why the launcher was left alone (something cohors did not make sits
    /// there), or `None` when it now points at the new version. Either way the
    /// new version runs: every other cohors hands control to it.
    pub launcher_left: Option<String>,
    pub pruned: Vec<String>,
}

/// Download, verify, unpack, test and switch to `version`. `running` is the
/// running binary (symlinks resolved), whose version is never pruned.
/// Fails with [`Busy`] when another cohors is updating.
pub(crate) fn update_to(
    layout: &Layout,
    origin: &Origin,
    version: &str,
    running: Option<&Path>,
) -> Result<Installed> {
    let _lock = Lock::take(layout).ok_or(Busy)?;
    let target = this_target().ok_or_else(|| {
        anyhow!(
            "no cohors release is built for this platform ({} {})",
            std::env::consts::ARCH,
            std::env::consts::OS
        )
    })?;
    let archive = archive_name(target);
    let stage = layout.staging().join(format!(
        "{version}-{}-{}",
        std::process::id(),
        super::now_secs()
    ));
    std::fs::create_dir_all(&stage).context("making the staging folder")?;
    let result = (|| {
        let bytes = download_verified(origin, version, &archive)?;
        let archive_path = stage.join(&archive);
        std::fs::write(&archive_path, &bytes).context("saving the download")?;
        let unpacked = stage.join("unpacked");
        std::fs::create_dir_all(&unpacked)?;
        super::run_quietly(
            Command::new("tar")
                .arg("-xf")
                .arg(&archive_path)
                .arg("-C")
                .arg(&unpacked),
        )
        .context("unpacking the release")?;
        let binary = find_binary(&unpacked)
            .ok_or_else(|| anyhow!("the release archive has no {EXE} in it"))?;
        smoke_test(&binary, version)?;
        let placed = place(layout, version, &binary)?;
        let launcher_left = link_unless_foreign(layout, &placed)?;
        let pruned = prune(layout, running, version);
        Ok(Installed {
            binary: placed,
            launcher: layout.launcher(),
            launcher_left,
            pruned,
        })
    })();
    let _ = std::fs::remove_dir_all(&stage);
    result
}

/// The archive, checked against the release's SHA-256, in up to [`TRIES`]
/// tries (a dropped connection or a corrupt download gets another go).
fn download_verified(origin: &Origin, version: &str, archive: &str) -> Result<Vec<u8>> {
    let mut last = None;
    for attempt in 1..=TRIES {
        match download_once(origin, version, archive) {
            Ok(bytes) => return Ok(bytes),
            Err(err) => {
                tracing::debug!("download try {attempt} of {archive} failed: {err:#}");
                last = Some(err);
                if origin.is_web() && attempt < TRIES {
                    std::thread::sleep(Duration::from_secs(u64::from(attempt)));
                }
            }
        }
    }
    Err(last
        .unwrap_or_else(|| anyhow!("nothing was tried"))
        .context(format!("{TRIES} tries")))
}

fn download_once(origin: &Origin, version: &str, archive: &str) -> Result<Vec<u8>> {
    let sums = origin.get(version, &format!("{archive}.sha256"), 4096)?;
    let expected = checksum_in(&String::from_utf8_lossy(&sums), archive)
        .ok_or_else(|| anyhow!("the release's checksum for {archive} cannot be read"))?;
    let bytes = origin.get(version, archive, MAX_ARCHIVE)?;
    let actual = sha256_hex(&bytes);
    if actual != expected {
        bail!(
            "{archive} does not match the release's checksum (expected {expected}, got {actual})"
        );
    }
    Ok(bytes)
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The binary in an unpacked archive: at the top, or one folder down
/// (`cohors-<target>/cohors`, as cargo-dist packs it). Links do not count.
fn find_binary(unpacked: &Path) -> Option<PathBuf> {
    let is_binary = |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file());
    let top = unpacked.join(EXE);
    if is_binary(&top) {
        return Some(top);
    }
    std::fs::read_dir(unpacked)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(EXE))
        .find(|path| is_binary(path))
}

/// Run `<binary> --version` and expect `cohors <version>`: a download that
/// does not start, or is another version, is never switched to (nor handed
/// control).
pub(crate) fn smoke_test(binary: &Path, version: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o755))?;
    }
    let mut child = Command::new(binary)
        .arg("--version")
        // Whatever the new binary is, it must not start an update of its own.
        .env("COHORS_DISABLE_UPDATES", "1")
        .env("COHORS_NO_UPDATE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("starting the new cohors")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("the new cohors did not answer `--version`");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let output = child.wait_with_output()?;
    let said = String::from_utf8_lossy(&output.stdout);
    let wanted = format!("cohors {version}");
    if output.status.success() && said.lines().any(|line| line.trim() == wanted) {
        Ok(())
    } else {
        bail!(
            "the new cohors says {:?}, not {wanted:?}",
            said.lines().next().unwrap_or("").trim()
        )
    }
}

/// Move the tested binary into `versions/<version>/`, replacing that folder
/// whole if it is already there (a reinstall). Returns the binary's new path.
pub(crate) fn place(layout: &Layout, version: &str, binary: &Path) -> Result<PathBuf> {
    let versions = layout.versions();
    std::fs::create_dir_all(&versions).context("making the versions folder")?;
    let pid = std::process::id();
    let incoming = versions.join(format!(".incoming-{version}-{pid}"));
    let _ = std::fs::remove_dir_all(&incoming);
    std::fs::create_dir_all(&incoming)?;
    std::fs::copy(binary, incoming.join(EXE)).context("copying the new cohors into place")?;
    let folder = versions.join(version);
    if folder.exists() {
        let old = versions.join(format!(".old-{version}-{pid}"));
        std::fs::rename(&folder, &old)?;
        if let Err(err) = std::fs::rename(&incoming, &folder) {
            let _ = std::fs::rename(&old, &folder);
            return Err(err).context("moving the new cohors into place");
        }
        let _ = std::fs::remove_dir_all(&old);
    } else {
        std::fs::rename(&incoming, &folder).context("moving the new cohors into place")?;
    }
    Ok(folder.join(EXE))
}

/// Point the launcher at `binary`, unless something cohors did not make sits
/// at its place: then leave that alone and say why (`Some`).
pub(crate) fn link_unless_foreign(layout: &Layout, binary: &Path) -> Result<Option<String>> {
    if let Some(why) = foreign_launcher(layout) {
        return Ok(Some(why));
    }
    point_launcher(layout, binary)?;
    Ok(None)
}

/// Why the launcher's place holds something cohors did not put there, if it
/// does: a pipx or uv tool link, a file, a link elsewhere. cohors never
/// replaces those. (On Windows the launcher is a copy, which cannot be told
/// apart, so this never objects there.)
#[cfg(not(unix))]
pub(crate) fn foreign_launcher(_layout: &Layout) -> Option<String> {
    None
}

/// Why the launcher's place holds something cohors did not put there, if it
/// does: a pipx or uv tool link, a file, a link elsewhere. cohors never
/// replaces those.
#[cfg(unix)]
pub(crate) fn foreign_launcher(layout: &Layout) -> Option<String> {
    let launcher = layout.launcher();
    let meta = std::fs::symlink_metadata(&launcher).ok()?;
    let shown = layout.show(&launcher);
    if !meta.file_type().is_symlink() {
        return Some(format!("{shown} is a file cohors did not put there"));
    }
    let target = std::fs::read_link(&launcher).ok()?;
    let target = layout.bin.join(target);
    if target.starts_with(layout.versions()) {
        return None;
    }
    let resolved = std::fs::canonicalize(&target).unwrap_or(target);
    Some(format!(
        "{shown} links to {}, which cohors did not make",
        layout.show(&resolved)
    ))
}

/// Point the launcher at `binary`: a new link, renamed over the old one, so
/// `cohors` is always one version or the other, never missing.
#[cfg(unix)]
pub(crate) fn point_launcher(layout: &Layout, binary: &Path) -> Result<()> {
    if let Some(problem) = foreign_launcher(layout) {
        bail!("{problem}; remove it, then run `cohors install` again");
    }
    std::fs::create_dir_all(&layout.bin).context("making the launcher's folder")?;
    let fresh = layout.bin.join(format!(".cohors-{}", std::process::id()));
    let _ = std::fs::remove_file(&fresh);
    std::os::unix::fs::symlink(binary, &fresh).context("linking the launcher")?;
    std::fs::rename(&fresh, layout.launcher()).context("switching the launcher")
}

/// Windows has no links a user can make by default: copy the binary beside
/// the launcher, set the old one aside as `cohors.exe.old.<time>`, and rename
/// the copy into place.
#[cfg(not(unix))]
pub(crate) fn point_launcher(layout: &Layout, binary: &Path) -> Result<()> {
    std::fs::create_dir_all(&layout.bin).context("making the launcher's folder")?;
    let launcher = layout.launcher();
    let fresh = layout
        .bin
        .join(format!(".cohors-{}.exe", std::process::id()));
    std::fs::copy(binary, &fresh).context("copying the launcher")?;
    if launcher.exists() {
        let old = layout.bin.join(format!("{EXE}.old.{}", super::now_secs()));
        std::fs::rename(&launcher, old).context("setting the old launcher aside")?;
    }
    std::fs::rename(&fresh, launcher).context("switching the launcher")
}

/// The versions in `versions/`, newest first.
pub(crate) fn installed_versions(layout: &Layout) -> Vec<String> {
    let mut versions: Vec<(Version, String)> = std::fs::read_dir(layout.versions())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let version = Version::parse(&name)?;
            entry.path().join(EXE).is_file().then_some((version, name))
        })
        .collect();
    versions.sort_by_key(|(version, _)| std::cmp::Reverse(*version));
    versions.into_iter().map(|(_, name)| name).collect()
}

/// The version the launcher points at, when the launcher is ours.
pub(crate) fn linked_version(layout: &Layout) -> Option<String> {
    let target = std::fs::read_link(layout.launcher()).ok()?;
    let target = layout.bin.join(target);
    let rest = target.strip_prefix(layout.versions()).ok()?;
    let name = rest.components().next()?.as_os_str().to_string_lossy();
    Version::parse(&name).map(|_| name.into_owned())
}

/// The managed copy's active version: the one the launcher points at, or,
/// with no launcher of ours (something else sits there), the newest one.
pub(crate) fn active_version(layout: &Layout) -> Option<String> {
    linked_version(layout)
        .filter(|v| layout.versions().join(v).join(EXE).is_file())
        .or_else(|| installed_versions(layout).into_iter().next())
}

/// The active version's binary.
pub(crate) fn active_binary(layout: &Layout) -> Option<PathBuf> {
    let binary = layout.versions().join(active_version(layout)?).join(EXE);
    binary.is_file().then_some(binary)
}

/// The version folder `binary` runs from, if it is one of ours.
fn version_running(layout: &Layout, binary: &Path) -> Option<String> {
    let versions = std::fs::canonicalize(layout.versions()).unwrap_or(layout.versions());
    let binary = std::fs::canonicalize(binary).unwrap_or(binary.to_path_buf());
    let rest = binary.strip_prefix(&versions).ok()?;
    Some(
        rest.components()
            .next()?
            .as_os_str()
            .to_string_lossy()
            .into_owned(),
    )
}

/// Delete all but the running, the active and the two newest versions, and
/// any half-made folder a crashed update left (the lock is held, so no other
/// update is using them). Returns what was deleted.
fn prune(layout: &Layout, running: Option<&Path>, active: &str) -> Vec<String> {
    let versions = layout.versions();
    let Ok(entries) = std::fs::read_dir(&versions) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".incoming-") || name.starts_with(".old-") {
            let _ = std::fs::remove_dir_all(entry.path());
        } else if entry.path().is_dir() {
            names.push(name);
        }
    }
    let running = running.and_then(|binary| version_running(layout, binary));
    let mut keep = vec![active];
    if let Some(running) = running.as_deref() {
        keep.push(running);
    }
    let gone = to_prune(&names, &keep, KEEP_NEWEST);
    gone.into_iter()
        .filter(|name| std::fs::remove_dir_all(versions.join(name)).is_ok())
        .collect()
}

#[cfg(all(test, unix))]
pub(crate) mod tests {
    use super::*;

    /// A fake release of `version` under `releases/v<version>/`, whose binary
    /// is a script that answers `--version` with `says`.
    pub(crate) fn fake_release(releases: &Path, version: &str, says: &str) {
        let target = this_target().expect("a platform with releases");
        let folder = releases.join(format!("v{version}"));
        let pack = folder.join("pack").join(format!("cohors-{target}"));
        std::fs::create_dir_all(&pack).unwrap();
        let script = pack.join("cohors");
        std::fs::write(&script, format!("#!/bin/sh\necho 'cohors {says}'\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let archive = archive_name(target);
        let status = Command::new("tar")
            .arg("-cJf")
            .arg(folder.join(&archive))
            .arg("-C")
            .arg(folder.join("pack"))
            .arg(format!("cohors-{target}"))
            .status()
            .unwrap();
        assert!(status.success(), "tar could not pack the fake release");
        let bytes = std::fs::read(folder.join(&archive)).unwrap();
        std::fs::write(
            folder.join(format!("{archive}.sha256")),
            format!("{} *{archive}\n", sha256_hex(&bytes)),
        )
        .unwrap();
        std::fs::remove_dir_all(folder.join("pack")).unwrap();
    }

    /// A managed version already in place, as an earlier install left it.
    pub(crate) fn installed(layout: &Layout, version: &str) -> PathBuf {
        let folder = layout.versions().join(version);
        std::fs::create_dir_all(&folder).unwrap();
        let binary = folder.join(EXE);
        std::fs::write(&binary, format!("#!/bin/sh\necho 'cohors {version}'\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        binary
    }

    fn launch(layout: &Layout) -> String {
        let out = Command::new(layout.launcher())
            .arg("--version")
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn setup() -> (tempfile::TempDir, Layout, Origin) {
        let tmp = tempfile::tempdir().unwrap();
        let layout = Layout::under(&tmp.path().join("home"));
        let origin = Origin::Dir(tmp.path().join("releases"));
        (tmp, layout, origin)
    }

    #[test]
    fn a_verified_release_is_swapped_in_and_the_launcher_follows() {
        let (tmp, layout, origin) = setup();
        fake_release(&tmp.path().join("releases"), "0.7.8", "0.7.8");
        let running = installed(&layout, "0.7.7");
        point_launcher(&layout, &running).unwrap();
        assert_eq!(launch(&layout), "cohors 0.7.7");

        let done = update_to(&layout, &origin, "0.7.8", Some(&running)).unwrap();
        assert_eq!(done.binary, layout.versions().join("0.7.8").join(EXE));
        assert_eq!(launch(&layout), "cohors 0.7.8");
        assert_eq!(active_version(&layout).as_deref(), Some("0.7.8"));
        assert_eq!(installed_versions(&layout), ["0.7.8", "0.7.7"]);
        // The staging folder and the lock are gone.
        assert!(
            std::fs::read_dir(layout.staging())
                .map(|mut d| d.next().is_none())
                .unwrap_or(true)
        );
        assert!(!layout.lock_file().exists());
        // A reinstall of the same version replaces it in place.
        update_to(&layout, &origin, "0.7.8", Some(&running)).unwrap();
        assert_eq!(launch(&layout), "cohors 0.7.8");
    }

    #[test]
    fn the_prune_never_deletes_the_running_version() {
        let (tmp, layout, origin) = setup();
        fake_release(&tmp.path().join("releases"), "0.7.8", "0.7.8");
        let mut running = PathBuf::new();
        for version in ["0.7.4", "0.7.5", "0.7.6", "0.7.7"] {
            let binary = installed(&layout, version);
            if version == "0.7.4" {
                running = binary;
            }
        }
        // A crashed update's leftovers go too.
        std::fs::create_dir_all(layout.versions().join(".incoming-0.7.6-1")).unwrap();
        let done = update_to(&layout, &origin, "0.7.8", Some(&running)).unwrap();
        let mut pruned = done.pruned.clone();
        pruned.sort();
        assert_eq!(pruned, ["0.7.5", "0.7.6"]);
        assert_eq!(installed_versions(&layout), ["0.7.8", "0.7.7", "0.7.4"]);
        assert!(running.is_file(), "the running binary was deleted");
        assert!(!layout.versions().join(".incoming-0.7.6-1").exists());
    }

    #[test]
    fn a_bad_checksum_leaves_the_current_version_untouched() {
        let (tmp, layout, origin) = setup();
        let releases = tmp.path().join("releases");
        fake_release(&releases, "0.7.8", "0.7.8");
        let target = this_target().unwrap();
        let sums = releases
            .join("v0.7.8")
            .join(format!("{}.sha256", archive_name(target)));
        std::fs::write(
            &sums,
            format!("{} *{}\n", "0".repeat(64), archive_name(target)),
        )
        .unwrap();
        let running = installed(&layout, "0.7.7");
        point_launcher(&layout, &running).unwrap();

        let err = update_to(&layout, &origin, "0.7.8", Some(&running)).unwrap_err();
        assert!(format!("{err:#}").contains("checksum"), "{err:#}");
        assert!(format!("{err:#}").contains("3 tries"), "{err:#}");
        assert_eq!(launch(&layout), "cohors 0.7.7");
        assert_eq!(installed_versions(&layout), ["0.7.7"]);
    }

    #[test]
    fn a_binary_that_does_not_answer_with_its_version_is_never_switched_to() {
        let (tmp, layout, origin) = setup();
        // The archive says 0.7.8 on the outside and 0.6.0 on the inside.
        fake_release(&tmp.path().join("releases"), "0.7.8", "0.6.0");
        let running = installed(&layout, "0.7.7");
        point_launcher(&layout, &running).unwrap();
        let err = update_to(&layout, &origin, "0.7.8", Some(&running)).unwrap_err();
        assert!(format!("{err:#}").contains("0.6.0"), "{err:#}");
        assert_eq!(launch(&layout), "cohors 0.7.7");
        assert_eq!(installed_versions(&layout), ["0.7.7"]);
    }

    #[test]
    fn two_updaters_at_once_one_skips() {
        let (tmp, layout, origin) = setup();
        fake_release(&tmp.path().join("releases"), "0.7.8", "0.7.8");
        let held = Lock::take(&layout).expect("the first takes the lock");
        assert!(Lock::take(&layout).is_none());
        let err = update_to(&layout, &origin, "0.7.8", None).unwrap_err();
        assert!(err.downcast_ref::<Busy>().is_some(), "{err:#}");
        assert!(installed_versions(&layout).is_empty());
        drop(held);
        update_to(&layout, &origin, "0.7.8", None).unwrap();
        assert_eq!(installed_versions(&layout), ["0.7.8"]);
    }

    #[test]
    fn a_launcher_cohors_did_not_make_is_left_alone() {
        let (tmp, layout, origin) = setup();
        fake_release(&tmp.path().join("releases"), "0.7.8", "0.7.8");
        // A pipx or uv tool install puts its own `cohors` in ~/.local/bin.
        std::fs::create_dir_all(&layout.bin).unwrap();
        std::fs::write(layout.launcher(), "#!/usr/bin/env python3\n").unwrap();
        // The update still lands (hand-off reaches it); the file is untouched.
        let done = update_to(&layout, &origin, "0.7.8", None).unwrap();
        let why = done.launcher_left.expect("the launcher was left alone");
        assert!(why.contains("did not put there"), "{why}");
        assert_eq!(
            std::fs::read_to_string(layout.launcher()).unwrap(),
            "#!/usr/bin/env python3\n"
        );
        assert_eq!(linked_version(&layout), None);
        assert_eq!(active_version(&layout).as_deref(), Some("0.7.8"));
        assert_eq!(active_binary(&layout), Some(done.binary));
    }
}
