//! Setup tips, the disk half (ADR-065): which parts of cohors this machine is
//! not using yet, and what became of each tip.
//!
//! `cohors_core::suggest` words the tips and decides which are still worth
//! showing; this module finds the facts it decides from:
//!
//! - **The web dashboard:** a marker file in cohors' cache folder, written the
//!   first time `cohors web` serves its page ([`TipSource::page_opened`]).
//! - **Editors:** VS Code, Cursor and Windsurf, from their program on PATH,
//!   their macOS app, or their extensions folder (`~/.vscode/extensions`). The
//!   extension counts as installed when a `rushirb2001.cohors-*` folder is in
//!   that folder and the editor has not marked it removed (`.obsolete`). No
//!   editor is started (`code --list-extensions` takes a second or more).
//! - **Claude Code:** `claude` on PATH (or where its installers put it), and
//!   whether cohors is registered: its plugin in `~/.claude/plugins`, or an MCP
//!   server mentioning cohors in `~/.claude.json` (user or project scope).
//!   `$CLAUDE_CONFIG_DIR` moves both, as it does for Claude Code.
//! - **What became of each tip:** `tips.json` in the cache folder.
//!
//! Every read is bounded and fails quiet: a file that is missing, too large or
//! not what was expected leaves a tip out (or, where it would only be a guess,
//! leaves the tip out too). Nothing is sent anywhere. `COHORS_NO_TIPS=1` turns
//! every tip off.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cohors_core::suggest::{
    ClaudeFacts, EditorFacts, SetupFacts, Suggestion, TIP_EXTENSION, TIP_IDS, TIP_WEB, TipLedger,
    setup_tips, tip_label, tips_to_show,
};
use serde::Deserialize;

/// The ledger: what became of each tip.
const LEDGER_FILE: &str = "tips.json";
/// Written the first time `cohors web` serves its page.
const WEB_MARKER: &str = "web-opened";
/// The largest `~/.claude.json` read. It grows with every project Claude
/// Code has opened; past this, whether cohors is registered is not looked
/// for and the tip is left out.
const CLAUDE_JSON_MAX: u64 = 32 * 1024 * 1024;
/// The largest of the other small files read (the ledger, the plugin list,
/// an editor's `.obsolete`).
const SMALL_FILE_MAX: u64 = 1024 * 1024;
/// At most this many entries of a folder are looked at.
const MAX_ENTRIES: usize = 5_000;

/// The editors that take the cohors extension: their name, their program,
/// their macOS app, and the folder in the home folder holding `extensions/`.
const EDITORS: [(&str, &str, &str, &str); 3] = [
    ("VS Code", "code", "Visual Studio Code.app", ".vscode"),
    ("Cursor", "cursor", "Cursor.app", ".cursor"),
    ("Windsurf", "windsurf", "Windsurf.app", ".windsurf"),
];

/// Where the tips look. [`TipEnv::from_env`] is this machine's; tests build
/// one over a temporary folder.
#[derive(Debug, Clone)]
pub struct TipEnv {
    /// The home folder.
    pub home: PathBuf,
    /// The program search path (`PATH`).
    pub path: OsString,
    /// cohors' cache folder (the ledger and the web marker). `None` keeps
    /// nothing: every tip is shown as new and none can be dismissed.
    pub cache_dir: Option<PathBuf>,
    /// Claude Code's folder (`~/.claude`, or `$CLAUDE_CONFIG_DIR`).
    pub claude_dir: PathBuf,
    /// Claude Code's settings file (`~/.claude.json`).
    pub claude_json: PathBuf,
    /// Where macOS apps are installed (`/Applications`, `~/Applications`).
    pub app_dirs: Vec<PathBuf>,
    /// This cohors, as it should be registered with Claude Code.
    pub cohors: Option<PathBuf>,
}

impl TipEnv {
    /// This machine's. `None` when tips are turned off (`COHORS_NO_TIPS`
    /// set to anything but `0` or empty) or there is no home folder.
    pub fn from_env() -> Option<Self> {
        if std::env::var_os("COHORS_NO_TIPS").is_some_and(|v| !v.is_empty() && v != "0") {
            return None;
        }
        let home = cohors_config::paths::home_dir().ok()?.into_std_path_buf();
        let path = std::env::var_os("PATH").unwrap_or_default();
        let (claude_dir, claude_json) = match std::env::var_os("CLAUDE_CONFIG_DIR") {
            Some(dir) if !dir.is_empty() => {
                let dir = PathBuf::from(dir);
                (dir.clone(), dir.join(".claude.json"))
            }
            _ => (home.join(".claude"), home.join(".claude.json")),
        };
        let app_dirs = if cfg!(target_os = "macos") {
            vec![PathBuf::from("/Applications"), home.join("Applications")]
        } else {
            Vec::new()
        };
        let cohors = registrable_cohors(&path);
        Some(Self {
            cache_dir: cohors_config::paths::cache_dir()
                .ok()
                .map(|d| d.into_std_path_buf()),
            home,
            path,
            claude_dir,
            claude_json,
            app_dirs,
            cohors,
        })
    }

    fn cache_file(&self, name: &str) -> Option<PathBuf> {
        self.cache_dir.as_ref().map(|dir| dir.join(name))
    }
}

/// The cohors that is running, as an absolute path to register: the one on
/// PATH when it is this same program (a package manager's link, which still
/// works after an upgrade), else the program itself.
fn registrable_cohors(path_var: &OsString) -> Option<PathBuf> {
    let running = std::env::current_exe().ok()?;
    let real = fs::canonicalize(&running).unwrap_or_else(|_| running.clone());
    let on_path = cohors_launchpad::find_on_path("cohors", path_var)
        .filter(|p| p.is_absolute() && fs::canonicalize(p).ok().as_ref() == Some(&real));
    Some(on_path.unwrap_or(running))
}

// ── Detection ────────────────────────────────────────────────────────────────

/// What the tips need to know about this machine.
pub fn detect(env: &TipEnv) -> SetupFacts {
    SetupFacts {
        web_opened: env.cache_file(WEB_MARKER).is_some_and(|m| m.exists()),
        editors: EDITORS
            .iter()
            .filter_map(|&(name, program, app, folder)| editor(env, name, program, app, folder))
            .collect(),
        claude: claude(env),
        cohors_path: env.cohors.as_ref().map(|p| p.display().to_string()),
    }
}

/// One editor, when it is installed.
fn editor(env: &TipEnv, name: &str, program: &str, app: &str, folder: &str) -> Option<EditorFacts> {
    let on_path = cohors_launchpad::find_on_path(program, &env.path).is_some();
    let bundle = env
        .app_dirs
        .iter()
        .map(|dir| dir.join(app))
        .find(|p| p.is_dir());
    let extensions = env.home.join(folder).join("extensions");
    let has_folder = extensions.is_dir();
    if !on_path && bundle.is_none() && !has_folder {
        return None;
    }
    // Without the program on PATH, the one inside the app still installs
    // extensions; with neither, the program's name is the best there is.
    let cli = if on_path {
        program.to_string()
    } else {
        bundle
            .map(|b| b.join("Contents/Resources/app/bin").join(program))
            .filter(|p| p.is_file())
            .map_or_else(|| program.to_string(), |p| p.display().to_string())
    };
    Some(EditorFacts {
        name: name.to_string(),
        cli,
        has_extension: has_folder && has_cohors_extension(&extensions),
    })
}

/// Whether `extensions` (an editor's extensions folder) holds the cohors
/// extension: a `rushirb2001.cohors-<version>` folder the editor has not
/// marked removed (it lists those in `.obsolete` until it restarts).
fn has_cohors_extension(extensions: &Path) -> bool {
    let Ok(entries) = fs::read_dir(extensions) else {
        return false;
    };
    let obsolete = match read_bounded(&extensions.join(".obsolete"), SMALL_FILE_MAX) {
        Bounded::Text(text) => text,
        _ => String::new(),
    };
    entries.take(MAX_ENTRIES).flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.to_ascii_lowercase().starts_with("rushirb2001.cohors-")
            && entry.path().is_dir()
            && !obsolete.contains(&format!("\"{name}\""))
    })
}

/// Claude Code, when it is installed.
fn claude(env: &TipEnv) -> Option<ClaudeFacts> {
    let cli = if cohors_launchpad::find_on_path("claude", &env.path).is_some() {
        "claude".to_string()
    } else {
        // Where its installers put it when PATH has not caught up.
        let places = [env.claude_dir.join("local"), env.home.join(".local/bin")];
        let places = std::env::join_paths(places).ok()?;
        cohors_launchpad::find_on_path("claude", &places)?
            .display()
            .to_string()
    };
    Some(ClaudeFacts {
        cli,
        has_cohors: cohors_in_claude(env),
    })
}

/// Whether Claude Code already has cohors: its plugin, or an MCP server that
/// mentions cohors (by name, program or argument, in any scope). `None` when
/// its settings could not be read reliably.
fn cohors_in_claude(env: &TipEnv) -> Option<bool> {
    if plugin_installed(&env.claude_dir) {
        return Some(true);
    }
    let text = match read_bounded(&env.claude_json, CLAUDE_JSON_MAX) {
        // Never run, or run without settings: nothing is registered.
        Bounded::Missing => return Some(false),
        Bounded::Unusable => return None,
        Bounded::Text(text) => text,
    };
    // Most files never mention cohors at all: no need to parse those.
    if !text.to_ascii_lowercase().contains("cohors") {
        return Some(false);
    }
    let settings: ClaudeSettings = serde_json::from_str(&text).ok()?;
    let mut servers = settings.mcp_servers.iter().chain(
        settings
            .projects
            .values()
            .flat_map(|p| p.mcp_servers.iter()),
    );
    Some(servers.any(|(name, server)| mentions_cohors(name, server)))
}

/// The parts of `~/.claude.json` that say which MCP servers are registered.
/// Everything else in it is skipped unread.
#[derive(Deserialize, Default)]
#[serde(default)]
struct ClaudeSettings {
    #[serde(rename = "mcpServers")]
    mcp_servers: BTreeMap<String, serde_json::Value>,
    projects: BTreeMap<String, ClaudeProject>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClaudeProject {
    #[serde(rename = "mcpServers")]
    mcp_servers: BTreeMap<String, serde_json::Value>,
}

/// Whether an MCP server entry is cohors: by its name, its program, or an
/// argument (`docker run … ghcr.io/rushirb2001/cohors`).
fn mentions_cohors(name: &str, server: &serde_json::Value) -> bool {
    let has = |s: &str| s.to_ascii_lowercase().contains("cohors");
    has(name)
        || server
            .get("command")
            .and_then(|c| c.as_str())
            .is_some_and(has)
        || server
            .get("args")
            .and_then(|a| a.as_array())
            .is_some_and(|args| args.iter().filter_map(|a| a.as_str()).any(has))
}

/// Whether the cohors plugin is installed in Claude Code: listed in
/// `plugins/installed_plugins.json` (`cohors@<marketplace>`), or present in
/// its plugin cache (`plugins/cache/<marketplace>/cohors`).
fn plugin_installed(claude_dir: &Path) -> bool {
    let plugins = claude_dir.join("plugins");
    if let Bounded::Text(text) =
        read_bounded(&plugins.join("installed_plugins.json"), SMALL_FILE_MAX)
        && let Ok(list) = serde_json::from_str::<serde_json::Value>(&text)
        && list
            .get("plugins")
            .and_then(|p| p.as_object())
            .is_some_and(|p| p.keys().any(|k| k.split('@').next() == Some("cohors")))
    {
        return true;
    }
    fs::read_dir(plugins.join("cache")).is_ok_and(|marketplaces| {
        marketplaces
            .take(MAX_ENTRIES)
            .flatten()
            .any(|m| m.path().join("cohors").is_dir())
    })
}

/// A file read with a size limit.
enum Bounded {
    Missing,
    /// Unreadable, larger than the limit, or not UTF-8.
    Unusable,
    Text(String),
}

fn read_bounded(path: &Path, max: u64) -> Bounded {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Bounded::Missing,
        Err(_) => return Bounded::Unusable,
    };
    let mut bytes = Vec::new();
    match file.take(max + 1).read_to_end(&mut bytes) {
        Ok(_) if bytes.len() as u64 <= max => {
            String::from_utf8(bytes).map_or(Bounded::Unusable, Bounded::Text)
        }
        _ => Bounded::Unusable,
    }
}

// ── The ledger ───────────────────────────────────────────────────────────────

/// What became of each tip. Empty when there is no ledger yet, or it cannot
/// be read (a tip then shows as new, which is the harmless way to be wrong).
pub fn read_ledger(env: &TipEnv) -> TipLedger {
    match env
        .cache_file(LEDGER_FILE)
        .map(|f| read_bounded(&f, SMALL_FILE_MAX))
    {
        Some(Bounded::Text(text)) => serde_json::from_str(&text).unwrap_or_default(),
        _ => TipLedger::default(),
    }
}

/// Write the ledger whole: to a temporary file beside it, then renamed into
/// place, so a crash never leaves half of it.
fn write_ledger(env: &TipEnv, ledger: &TipLedger) -> std::io::Result<()> {
    let Some(file) = env.cache_file(LEDGER_FILE) else {
        return Err(std::io::Error::other("there is no cache folder"));
    };
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let body = serde_json::to_vec_pretty(ledger).map_err(std::io::Error::other)?;
    let temp = file.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&temp, body)?;
    fs::rename(&temp, &file).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })
}

/// The setup tips to show now, at most `max`: the ones that apply here, less
/// those dismissed or shown on enough days already.
pub fn current(env: &TipEnv, now: i64, max: usize) -> Vec<Suggestion> {
    tips_to_show(setup_tips(&detect(env)), &read_ledger(env), now, max)
}

/// The tips for a place someone is learning cohors in (the dashboard, the
/// start screen): the setup tips first, then the first-run tips
/// (`cohors_core::suggest::LEARN_TIPS`), at most `max`. `cohors next` and the
/// web page keep to the setup tips: the first-run tips are mostly about the
/// dashboard's keys.
pub fn current_with_first_run(env: &TipEnv, now: i64, max: usize) -> Vec<Suggestion> {
    let mut tips = setup_tips(&detect(env));
    tips.extend(cohors_core::suggest::learn_tips());
    tips_to_show(tips, &read_ledger(env), now, max)
}

/// Count `shown` as put in front of a person today. Quiet on failure: a tip
/// that is not counted is only shown a day longer.
pub fn note_shown(env: &TipEnv, shown: &[Suggestion], now: i64) {
    let mut ledger = read_ledger(env);
    let mut changed = false;
    for id in shown.iter().filter_map(|t| t.tip.as_deref()) {
        changed |= ledger.note_shown(id, now);
    }
    if changed && let Err(e) = write_ledger(env, &ledger) {
        tracing::debug!("could not record the tips shown: {e}");
    }
}

/// Why a tip could not be dismissed.
#[derive(Debug, thiserror::Error)]
pub enum DismissError {
    #[error("there is no tip called {0:?}; the tips are {ids}", ids = TIP_IDS.join(", "))]
    Unknown(String),
    #[error("could not save that: {0}")]
    Io(#[from] std::io::Error),
}

/// Hide tip `id` for good. Returns what the tip is about, to say so.
pub fn dismiss(env: &TipEnv, id: &str) -> Result<&'static str, DismissError> {
    let label = tip_label(id).ok_or_else(|| DismissError::Unknown(id.to_string()))?;
    let mut ledger = read_ledger(env);
    if ledger.dismiss(id) {
        write_ledger(env, &ledger)?;
    }
    Ok(label)
}

/// Note that `cohors web` served its page: the web tip is done.
pub fn record_web_opened(env: &TipEnv) {
    let Some(marker) = env.cache_file(WEB_MARKER) else {
        return;
    };
    if marker.exists() {
        return;
    }
    let written = marker
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(&marker, b""));
    if let Err(e) = written {
        tracing::debug!("could not note that the web page was opened: {e}");
    }
}

// ── For the web server ───────────────────────────────────────────────────────

/// How long the web server trusts what it found before looking again: long
/// enough that a page refreshing on every change costs nothing, short enough
/// that an extension installed meanwhile drops its tip within a minute.
const WEB_REDETECT: Duration = Duration::from_secs(60);

/// The tips `cohors web` shows, found at most once a minute however often the
/// page asks.
pub struct TipSource {
    env: TipEnv,
    found: Mutex<Option<(Instant, SetupFacts)>>,
    opened: AtomicBool,
}

impl TipSource {
    pub fn new(env: TipEnv) -> Self {
        Self {
            env,
            found: Mutex::new(None),
            opened: AtomicBool::new(false),
        }
    }

    /// The page was served: note it once (the web tip is done).
    pub fn page_opened(&self) {
        if !self.opened.swap(true, Ordering::Relaxed) {
            record_web_opened(&self.env);
        }
    }

    /// The tip for the web page, if any: never the web tip (that is where
    /// the person is), and the extension tip worded for a page, "Get the VS
    /// Code extension". Counted as shown.
    pub fn for_web(&self, now: i64) -> Vec<Suggestion> {
        let Ok(mut found) = self.found.lock() else {
            return Vec::new();
        };
        let facts = match &*found {
            Some((at, facts)) if at.elapsed() < WEB_REDETECT => facts.clone(),
            _ => {
                let facts = detect(&self.env);
                *found = Some((Instant::now(), facts.clone()));
                facts
            }
        };
        let mut tips = setup_tips(&facts);
        tips.retain(|t| t.tip.as_deref() != Some(TIP_WEB));
        for tip in &mut tips {
            if tip.tip.as_deref() == Some(TIP_EXTENSION)
                && let Some(editor) = facts.editors.first()
            {
                tip.title = format!("Get the {} extension", editor.name);
            }
        }
        let shown = tips_to_show(tips, &read_ledger(&self.env), now, 1);
        // Still under the lock: two requests at once do not both write.
        note_shown(&self.env, &shown, now);
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cohors_core::suggest::TIP_CLAUDE;

    const NOW: i64 = 1_800_000_000;

    /// A machine in a temporary folder: a home, an empty PATH folder, a
    /// cache folder, no apps.
    fn machine() -> (tempfile::TempDir, TipEnv) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let home = root.join("home");
        let bin = root.join("bin");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let env = TipEnv {
            path: std::env::join_paths([&bin]).unwrap(),
            cache_dir: Some(home.join(".cache/cohors")),
            claude_dir: home.join(".claude"),
            claude_json: home.join(".claude.json"),
            app_dirs: vec![root.join("Applications")],
            cohors: Some(PathBuf::from("/usr/local/bin/cohors")),
            home,
        };
        (dir, env)
    }

    /// Put an executable called `name` on the machine's PATH.
    fn install(env: &TipEnv, name: &str) {
        let bin = std::env::split_paths(&env.path).next().unwrap();
        let file = bin.join(name);
        fs::write(&file, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn ids(tips: &[Suggestion]) -> Vec<&str> {
        tips.iter().filter_map(|t| t.tip.as_deref()).collect()
    }

    #[test]
    fn a_bare_machine_only_has_the_web_tip() {
        let (_dir, env) = machine();
        let facts = detect(&env);
        assert!(!facts.web_opened && facts.editors.is_empty() && facts.claude.is_none());
        assert_eq!(ids(&current(&env, NOW, 3)), [TIP_WEB]);
    }

    #[test]
    fn an_editor_is_found_by_its_program_or_its_folder_and_the_extension_by_its_folder() {
        let (_dir, env) = machine();
        // Only the extensions folder: VS Code was installed here.
        let extensions = env.home.join(".vscode/extensions");
        fs::create_dir_all(&extensions).unwrap();
        install(&env, "cursor");
        let facts = detect(&env);
        let names: Vec<&str> = facts.editors.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["VS Code", "Cursor"]);
        assert_eq!(facts.editors[1].cli, "cursor");
        assert_eq!(ids(&current(&env, NOW, 3)), [TIP_WEB, TIP_EXTENSION]);

        // Installed in VS Code: no tip, for either editor.
        let installed = extensions.join("rushirb2001.cohors-0.7.7");
        fs::create_dir_all(&installed).unwrap();
        assert!(detect(&env).editors[0].has_extension);
        assert_eq!(ids(&current(&env, NOW, 3)), [TIP_WEB]);

        // Removed, but the folder lingers until VS Code restarts.
        fs::write(
            extensions.join(".obsolete"),
            r#"{"rushirb2001.cohors-0.7.7":true}"#,
        )
        .unwrap();
        assert!(!detect(&env).editors[0].has_extension);
    }

    #[test]
    fn a_mac_app_without_its_program_on_path_installs_through_the_app() {
        let (dir, env) = machine();
        let cli = dir
            .path()
            .join("Applications/Visual Studio Code.app/Contents/Resources/app/bin/code");
        fs::create_dir_all(cli.parent().unwrap()).unwrap();
        fs::write(&cli, "").unwrap();
        let facts = detect(&env);
        assert_eq!(facts.editors[0].cli, cli.display().to_string());
    }

    #[test]
    fn claude_code_is_asked_about_only_when_its_settings_say_for_sure() {
        let (_dir, env) = machine();
        install(&env, "claude");
        // Never run: nothing registered.
        assert_eq!(detect(&env).claude.unwrap().has_cohors, Some(false));
        assert_eq!(ids(&current(&env, NOW, 3)), [TIP_WEB, TIP_CLAUDE]);
        let tip = current(&env, NOW, 3).remove(1);
        assert_eq!(
            tip.command,
            "claude mcp add --scope user cohors -- /usr/local/bin/cohors mcp"
        );

        // A settings file that mentions cohors only as a folder name.
        fs::write(
            &env.claude_json,
            r#"{"projects":{"/code/cohors":{"allowedTools":[]}},"numStartups":3}"#,
        )
        .unwrap();
        assert_eq!(detect(&env).claude.unwrap().has_cohors, Some(false));

        // Registered in one project's scope, under another name.
        fs::write(
            &env.claude_json,
            r#"{"projects":{"/code/x":{"mcpServers":{"fleet":{"command":"/opt/bin/cohors","args":["mcp"]}}}}}"#,
        )
        .unwrap();
        assert_eq!(detect(&env).claude.unwrap().has_cohors, Some(true));

        // Registered through docker, in the user scope.
        fs::write(
            &env.claude_json,
            r#"{"mcpServers":{"repos":{"command":"docker","args":["run","ghcr.io/rushirb2001/cohors"]}}}"#,
        )
        .unwrap();
        assert_eq!(detect(&env).claude.unwrap().has_cohors, Some(true));

        // Unreadable: unknown, so no tip.
        fs::write(&env.claude_json, "{ cohors, not json").unwrap();
        assert_eq!(detect(&env).claude.unwrap().has_cohors, None);
        assert_eq!(ids(&current(&env, NOW, 3)), [TIP_WEB]);
    }

    #[test]
    fn the_plugin_counts_as_registered() {
        let (_dir, env) = machine();
        install(&env, "claude");
        let plugins = env.claude_dir.join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        fs::write(
            plugins.join("installed_plugins.json"),
            r#"{"version":2,"plugins":{"cohors@cohors":[{"scope":"user"}]}}"#,
        )
        .unwrap();
        assert_eq!(detect(&env).claude.unwrap().has_cohors, Some(true));

        fs::remove_file(plugins.join("installed_plugins.json")).unwrap();
        fs::create_dir_all(plugins.join("cache/some-marketplace/cohors/0.7.7")).unwrap();
        assert_eq!(detect(&env).claude.unwrap().has_cohors, Some(true));
    }

    #[test]
    fn opening_the_page_dismissing_and_showing_are_remembered() {
        let (_dir, env) = machine();
        install(&env, "claude");
        assert_eq!(ids(&current(&env, NOW, 3)), [TIP_WEB, TIP_CLAUDE]);

        let web = TipSource::new(env.clone());
        // The page never offers itself; it offers the next tip, once.
        assert_eq!(ids(&web.for_web(NOW)), [TIP_CLAUDE]);
        web.page_opened();
        assert!(detect(&env).web_opened);
        assert_eq!(read_ledger(&env).tips[TIP_CLAUDE].days, 1);

        assert_eq!(
            dismiss(&env, TIP_CLAUDE).unwrap(),
            "let Claude Code see your fleet"
        );
        assert!(current(&env, NOW, 3).is_empty());
        assert!(matches!(
            dismiss(&env, "nope"),
            Err(DismissError::Unknown(_))
        ));

        // A ledger that is not JSON is read as empty, not an error.
        fs::write(env.cache_dir.as_ref().unwrap().join(LEDGER_FILE), "garbage").unwrap();
        assert_eq!(read_ledger(&env), TipLedger::default());
    }

    #[test]
    fn the_web_page_words_the_extension_tip_for_a_page() {
        let (_dir, env) = machine();
        install(&env, "code");
        let tips = TipSource::new(env).for_web(NOW);
        assert_eq!(tips.len(), 1);
        assert_eq!(tips[0].title, "Get the VS Code extension");
        assert_eq!(
            tips[0].command,
            "code --install-extension rushirb2001.cohors"
        );
    }

    #[test]
    fn an_oversized_settings_file_is_not_read() {
        let (_dir, env) = machine();
        let path = env.home.join("big");
        fs::write(&path, vec![b'x'; 2048]).unwrap();
        assert!(matches!(read_bounded(&path, 1024), Bounded::Unusable));
        assert!(matches!(read_bounded(&path, 4096), Bounded::Text(_)));
        assert!(matches!(
            read_bounded(&env.home.join("absent"), 10),
            Bounded::Missing
        ));
    }
}
