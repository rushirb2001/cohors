//! TUI-local, desktop side effects: reveal in the file manager, copy to the
//! clipboard, and the argv for an editor handoff.
//!
//! The git mutating primitives (fetch/pull/push/commit/stash) and the shell-out
//! command runners now live in the `cohors-actions` crate so every front-end —
//! TUI, MCP, web — shares one implementation. They're re-exported here so the
//! rest of the binary keeps calling `crate::action::fetch` etc. unchanged. The
//! desktop handoffs below stay in the TUI: web and MCP never use them.

use std::process::Command;

use camino::Utf8Path;

pub use cohors_actions::{
    commit, fetch, pull_ff, push, run_command, run_command_timeout, stash_push,
};

/// Reveal the repo in the OS file manager (spawned detached).
pub fn reveal(path: &Utf8Path) -> Result<(), String> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    Command::new(opener)
        .arg(path.as_str())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{opener}: {e}"))
}

/// Copy text to the system clipboard.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_text(text.to_owned())
        .map_err(|e| e.to_string())
}

/// Build the argv to open `path` with `editor` (which may itself include flags,
/// e.g. `code -n`).
pub fn editor_argv(editor: &str, path: &Utf8Path) -> Vec<String> {
    let mut argv: Vec<String> = editor.split_whitespace().map(str::to_string).collect();
    argv.push(path.as_str().to_string());
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_argv_splits_flags_and_appends_path() {
        let argv = editor_argv("code -n", Utf8Path::new("/repos/foo"));
        assert_eq!(argv, vec!["code", "-n", "/repos/foo"]);
    }
}
