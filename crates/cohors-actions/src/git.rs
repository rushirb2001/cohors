//! The git mutating primitives: fetch/pull/push/commit/stash via the `git` CLI,
//! plus the shell-out command runners.
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

/// Push the current branch to its upstream (`git push`). Non-fast-forward
/// pushes are *rejected by git* (we never pass `--force`), so this can't
/// overwrite remote history. Returns a short status message.
pub fn push(path: &Utf8Path, name: &str) -> Result<String, String> {
    let out = run_git(path, &["push"])?;
    if out.status.success() {
        Ok(format!("pushed {name}"))
    } else {
        let msg = first_line(&out.stderr).to_lowercase();
        if msg.contains("no upstream") || msg.contains("no configured push") {
            Err(format!("push {name}: no upstream — skipped"))
        } else if msg.contains("rejected") || msg.contains("non-fast-forward") {
            Err(format!("push {name}: rejected (pull first)"))
        } else {
            Err(format!("push {name}: {}", first_line(&out.stderr)))
        }
    }
}

/// Stage everything (`git add -A`) and commit it with `message`. Stages tracked
/// *and* untracked changes (a fleet-wide "snapshot my WIP"). "Nothing to commit"
/// is a no-op success, not a failure (like [`stash_push`]). Never amends and
/// never rewrites history. Returns a short status message.
pub fn commit(path: &Utf8Path, name: &str, message: &str) -> Result<String, String> {
    let add = run_git(path, &["add", "-A"])?;
    if !add.status.success() {
        return Err(format!("commit {name}: {}", first_line(&add.stderr)));
    }
    let out = run_git(path, &["commit", "-m", message])?;
    if out.status.success() {
        return Ok(format!("committed {name}"));
    }
    // `git commit` exits non-zero when there's nothing staged; it writes that to
    // stdout, not stderr. Treat an empty index as a benign no-op.
    let combined = format!(
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .to_lowercase();
    if combined.contains("nothing to commit") || combined.contains("no changes added") {
        Ok(format!("{name}: nothing to commit"))
    } else {
        Err(format!("commit {name}: {}", first_line(&out.stderr)))
    }
}

/// Stash the repo's tracked changes (`git stash push`). Untracked files are
/// left alone (predictable). "No local changes" is a no-op success, not a
/// failure. Returns a short status message.
pub fn stash_push(path: &Utf8Path, name: &str) -> Result<String, String> {
    let out = run_git(path, &["stash", "push"])?;
    if !out.status.success() {
        return Err(format!("stash {name}: {}", first_line(&out.stderr)));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.to_lowercase().contains("no local changes") {
        Ok(format!("{name}: nothing to stash"))
    } else {
        Ok(format!("stashed {name}"))
    }
}

/// Run an arbitrary `cmd` via the user's shell inside `path`, capturing
/// stdout/stderr and the exit code. Uses `sh -c` / `cmd /C` so the user can
/// pipe and glob as in a terminal (consistent with ADR-013's shell-out model).
/// The shell is non-interactive, so shell aliases/functions don't apply — use
/// full commands. Returns `(code, stdout, stderr)`; `code` = -1 if the process
/// could not be spawned.
pub fn run_command(path: &Utf8Path, cmd: &str) -> (i32, String, String) {
    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    match Command::new(shell)
        .arg(flag)
        .arg(cmd)
        .current_dir(path.as_str())
        .output()
    {
        Ok(out) => (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ),
        Err(e) => (-1, String::new(), format!("could not run command: {e}")),
    }
}

/// Outcome of a time-bounded command run.
pub struct RunOutcome {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    /// The command exceeded its deadline and was killed.
    pub timed_out: bool,
}

/// Like [`run_command`], but kills the command (and reports `timed_out`) once it
/// exceeds `timeout`. Output pipes are drained on threads so a chatty command
/// can't deadlock on a full pipe, and partial output is still returned on a kill.
pub fn run_command_timeout(path: &Utf8Path, cmd: &str, timeout: std::time::Duration) -> RunOutcome {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::Instant;

    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    let mut child = match Command::new(shell)
        .arg(flag)
        .arg(cmd)
        .current_dir(path.as_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return RunOutcome {
                code: -1,
                stdout: String::new(),
                stderr: format!("could not run command: {e}"),
                timed_out: false,
            };
        }
    };

    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });

    let start = Instant::now();
    let mut timed_out = false;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break -1;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => break -1,
        }
    };

    let stdout = String::from_utf8_lossy(&out_handle.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_handle.join().unwrap_or_default()).into_owned();
    RunOutcome {
        code,
        stdout,
        stderr,
        timed_out,
    }
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
    fn first_line_skips_blank_lines() {
        assert_eq!(first_line(b"\n  hello \nworld"), "hello");
        assert_eq!(first_line(b""), "");
    }

    #[cfg(unix)]
    #[test]
    fn run_command_captures_exit_and_output() {
        let dir = camino::Utf8PathBuf::from_path_buf(std::env::temp_dir())
            .expect("temp dir is valid UTF-8");
        let (code, out, err) = run_command(&dir, "printf hi; printf oops 1>&2; exit 3");
        assert_eq!(code, 3);
        assert_eq!(out, "hi");
        assert_eq!(err, "oops");
    }

    #[cfg(unix)]
    #[test]
    fn run_command_timeout_completes_fast_command() {
        let dir = camino::Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        let r = run_command_timeout(&dir, "printf hi; exit 0", std::time::Duration::from_secs(5));
        assert!(!r.timed_out);
        assert_eq!(r.code, 0);
        assert_eq!(r.stdout, "hi");
    }

    #[cfg(unix)]
    #[test]
    fn run_command_timeout_kills_a_hang() {
        let dir = camino::Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        let r = run_command_timeout(&dir, "sleep 30", std::time::Duration::from_millis(300));
        assert!(r.timed_out);
    }

    /// `push` succeeds against a configured (local, offline) upstream. Exercises
    /// the happy path without touching the network.
    #[test]
    fn push_succeeds_to_a_configured_local_remote() {
        use std::process::Command;
        let tmp = tempfile::tempdir().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let bare = root.join("remote.git");
        let work = root.join("work");
        let git = |dir: &Utf8Path, args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir.as_str())
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {:?}", out);
        };
        Command::new("git")
            .args(["init", "--bare", bare.as_str()])
            .output()
            .unwrap();
        Command::new("git")
            .args(["init", "-b", "main", work.as_str()])
            .output()
            .unwrap();
        git(&work, &["config", "user.email", "t@example.com"]);
        git(&work, &["config", "user.name", "Tester"]);
        git(&work, &["remote", "add", "origin", bare.as_str()]);
        std::fs::write(work.join("a.txt"), "1").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", "one"]);
        git(&work, &["push", "-u", "origin", "main"]);
        // A new commit, then push it through our action.
        std::fs::write(work.join("a.txt"), "2").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", "two"]);

        let res = push(&work, "work");
        assert_eq!(res, Ok("pushed work".to_string()));
    }

    /// `commit` stages tracked + untracked and commits; a second call with a
    /// clean tree reports "nothing to commit" as a no-op success.
    #[test]
    fn commit_stages_then_reports_nothing_to_commit() {
        use std::process::Command;
        let tmp = tempfile::tempdir().unwrap();
        let work = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(work.as_str())
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
        };
        Command::new("git")
            .args(["init", "-b", "main", work.as_str()])
            .output()
            .unwrap();
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "Tester"]);
        std::fs::write(work.join("a.txt"), "1").unwrap();

        // First commit stages the new (untracked) file and commits it.
        assert_eq!(
            commit(work, "work", "snap"),
            Ok("committed work".to_string())
        );
        // Nothing changed since: a benign no-op, not an error.
        assert_eq!(
            commit(work, "work", "snap again"),
            Ok("work: nothing to commit".to_string())
        );
    }
}
