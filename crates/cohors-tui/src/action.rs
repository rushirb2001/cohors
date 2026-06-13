//! External, side-effecting actions: fetch/pull via the `git` CLI, reveal in
//! the file manager, copy to the clipboard, and the argv for an editor handoff.
//!
//! Network git operations shell out to the user's `git` so they inherit the
//! user's credentials and remote config (see ADR-013). `pull` is
//! fast-forward-only in v0.1 — it can never merge, rebase, or lose work.

use std::process::Command;

use camino::Utf8Path;

/// Run `git -C <path> <args>`, capturing output.
fn run_git(path: &Utf8Path, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git")
        .arg("-C")
        .arg(path.as_str())
        .args(args)
        .output()
        .map_err(|e| format!("could not run git: {e}"))
}

/// Fetch the repo's default remote. Returns a short status message.
pub fn fetch(path: &Utf8Path, name: &str) -> Result<String, String> {
    let out = run_git(path, &["fetch", "--quiet"])?;
    if out.status.success() {
        Ok(format!("fetched {name}"))
    } else {
        Err(format!("fetch {name}: {}", first_line(&out.stderr)))
    }
}

/// Fast-forward-only pull. Never merges/rebases, so it can't lose work; if it
/// can't fast-forward it reports that and changes nothing.
pub fn pull_ff(path: &Utf8Path, name: &str) -> Result<String, String> {
    let out = run_git(path, &["pull", "--ff-only", "--quiet"])?;
    if out.status.success() {
        Ok(format!("pulled {name}"))
    } else {
        let msg = first_line(&out.stderr).to_lowercase();
        if msg.is_empty() || msg.contains("fast-forward") || msg.contains("diverg") {
            Err(format!("pull {name}: not fast-forward — skipped"))
        } else {
            Err(format!("pull {name}: {}", first_line(&out.stderr)))
        }
    }
}

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

/// First non-empty, trimmed line of captured output (for error messages).
fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_argv_splits_flags_and_appends_path() {
        let argv = editor_argv("code -n", Utf8Path::new("/repos/foo"));
        assert_eq!(argv, vec!["code", "-n", "/repos/foo"]);
    }

    #[test]
    fn first_line_skips_blank_lines() {
        assert_eq!(first_line(b"\n  hello \nworld"), "hello");
        assert_eq!(first_line(b""), "");
    }
}
