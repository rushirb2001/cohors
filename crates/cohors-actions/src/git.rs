//! The git mutating primitives: fetch/pull/push/commit/stash via the `git` CLI,
//! plus the shell-out command runners.
//!
//! Network git operations shell out to the user's `git` so they inherit the
//! user's credentials and remote config (see ADR-013). `pull` is
//! fast-forward-only in v0.1 — it can never merge, rebase, or lose work.

use std::process::Command;

use camino::Utf8Path;

/// Errors from running a git command.
#[derive(Debug, thiserror::Error)]
pub enum GitActionError {
    /// The `git` binary could not be spawned (e.g., not on `$PATH`).
    #[error("could not run git: {0}")]
    SpawnFailed(#[source] std::io::Error),
}

/// Convert to string so callers returning `Result<_, String>` can use `?`.
impl From<GitActionError> for String {
    fn from(e: GitActionError) -> String {
        e.to_string()
    }
}

/// Run `git -C <path> <args>`, capturing output.
fn run_git(path: &Utf8Path, args: &[&str]) -> Result<std::process::Output, GitActionError> {
    Command::new("git")
        .arg("-C")
        .arg(path.as_str())
        .args(args)
        .output()
        .map_err(GitActionError::SpawnFailed)
}

/// Fetch the repo's default remote. Returns a short status message.
pub fn fetch(path: &Utf8Path, name: &str) -> Result<String, String> {
    let out = run_git(path, &["fetch", "--quiet"])?;
    if out.status.success() {
        Ok(format!("fetched {name}"))
    } else {
        Err(format!("fetch {name}: {}", telling_line(&out.stderr)))
    }
}

/// Fast-forward-only pull. Never merges/rebases, so it can't lose work; if it
/// can't fast-forward it reports that and changes nothing.
pub fn pull_ff(path: &Utf8Path, name: &str) -> Result<String, String> {
    let out = run_git(path, &["pull", "--ff-only", "--quiet"])?;
    if out.status.success() {
        Ok(format!("pulled {name}"))
    } else {
        // Match against *all* of stderr: git often leads with `hint:` lines and
        // puts the actual verdict (`fatal: Not possible to fast-forward`) last.
        let all = String::from_utf8_lossy(&out.stderr).to_lowercase();
        if all.trim().is_empty() || all.contains("fast-forward") || all.contains("diverg") {
            Err(format!("pull {name}: not fast-forward — skipped"))
        } else {
            Err(format!("pull {name}: {}", telling_line(&out.stderr)))
        }
    }
}

/// True when git's push stderr says the remote has commits this branch lacks
/// (`! [rejected] main -> main (fetch first)` / `(non-fast-forward)`), i.e. the
/// fix is to pull first. A `[remote rejected]` (a hook or branch protection said
/// no) is deliberately *not* this: pulling would not help, so its own reason is
/// reported instead.
fn rejected_as_behind(stderr: &[u8]) -> bool {
    let all = String::from_utf8_lossy(stderr).to_lowercase();
    all.contains("[rejected]") || all.contains("non-fast-forward") || all.contains("fetch first")
}

/// Push the current branch to its upstream (`git push`). Non-fast-forward
/// pushes are *rejected by git* (we never pass `--force`), so this can't
/// overwrite remote history. Returns a short status message.
///
/// A branch that was never pushed has no upstream, and plain `git push`
/// refuses it. That is exactly the work most at risk (it exists on one disk),
/// so instead of skipping it we *publish* it: `git push -u origin HEAD`, which
/// creates the branch on the remote and starts tracking it. It is still an
/// ordinary push — nothing is forced and nothing existing is overwritten.
pub fn push(path: &Utf8Path, name: &str) -> Result<String, String> {
    let out = run_git(path, &["push"])?;
    if out.status.success() {
        Ok(format!("pushed {name}"))
    } else {
        // Match against *all* of stderr, never its first line: a push always
        // opens with `To <remote-url>`, and the reason comes after it.
        let all = String::from_utf8_lossy(&out.stderr).to_lowercase();
        if all.contains("no upstream") || all.contains("no configured push") {
            let published = run_git(path, &["push", "-u", "origin", "HEAD"])?;
            if published.status.success() {
                Ok(format!("published {name} (new branch on origin)"))
            } else {
                Err(format!(
                    "push {name}: could not publish — {}",
                    telling_line(&published.stderr)
                ))
            }
        } else if rejected_as_behind(&out.stderr) {
            Err(format!("push {name}: rejected (pull first)"))
        } else {
            Err(format!("push {name}: {}", telling_line(&out.stderr)))
        }
    }
}

/// Push one specific local branch to `origin` (`git push [-u] origin <branch>`),
/// used when the user picks a branch that isn't the checked-out one. `set_upstream`
/// adds `-u` for a branch that was never pushed, so it starts tracking. Non-fast-
/// forward pushes are still rejected by git (never `--force`), so remote history is
/// safe. Assumes the remote is named `origin` (the near-universal default); any
/// other name surfaces git's own error verbatim.
pub fn push_branch(
    path: &Utf8Path,
    name: &str,
    branch: &str,
    set_upstream: bool,
) -> Result<String, String> {
    let mut args = vec!["push"];
    if set_upstream {
        args.push("-u");
    }
    args.extend_from_slice(&["origin", branch]);
    let out = run_git(path, &args)?;
    if out.status.success() {
        Ok(format!("pushed {name} ({branch})"))
    } else {
        if rejected_as_behind(&out.stderr) {
            Err(format!("push {name} ({branch}): rejected (pull first)"))
        } else {
            Err(format!(
                "push {name} ({branch}): {}",
                telling_line(&out.stderr)
            ))
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
        return Err(format!("commit {name}: {}", telling_line(&add.stderr)));
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
        Err(format!("commit {name}: {}", telling_line(&out.stderr)))
    }
}

/// Stash the repo's tracked changes (`git stash push`). Untracked files are
/// left alone (predictable). "No local changes" is a no-op success, not a
/// failure. Returns a short status message.
pub fn stash_push(path: &Utf8Path, name: &str) -> Result<String, String> {
    let out = run_git(path, &["stash", "push"])?;
    if !out.status.success() {
        return Err(format!("stash {name}: {}", telling_line(&out.stderr)));
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
///
/// The deadline covers the whole *job*, not just the shell: `sleep 6; echo done`
/// and `sleep 7 & echo started` both leave work running after (or instead of)
/// the shell's own exit, and a background child keeps our output pipes open.
/// On unix the shell therefore starts in its own process group, and on timeout
/// the whole group is killed, so nothing it started outlives the run. The run
/// returns once the shell has exited *and* its output is closed, or at the
/// deadline (plus a short grace for the readers), whichever comes first.
///
/// stdin is closed (`/dev/null`): a non-interactive fleet run must never read
/// the terminal (or, under the MCP server, the JSON-RPC stream on stdin).
pub fn run_command_timeout(path: &Utf8Path, cmd: &str, timeout: std::time::Duration) -> RunOutcome {
    run_command_until(path, cmd, timeout, &INTERRUPT)
}

/// Set when the user interrupts the runs (Ctrl-C at the CLI): every running
/// job's process group is killed and its run returns at once. Because jobs run
/// in their own process groups, the terminal's SIGINT no longer reaches them by
/// itself; a front-end forwards it by setting this flag from a signal handler
/// ([`interrupt_flag`]). Nothing in this crate installs a handler.
static INTERRUPT: std::sync::LazyLock<std::sync::Arc<std::sync::atomic::AtomicBool>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));

/// The flag that stops every running job (see [`INTERRUPT`]), for a front-end
/// to register with its signal handling.
pub fn interrupt_flag() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    std::sync::Arc::clone(&INTERRUPT)
}

/// [`run_command_timeout`], stopping early when `stop` is set.
fn run_command_until(
    path: &Utf8Path,
    cmd: &str,
    timeout: std::time::Duration,
    stop: &std::sync::atomic::AtomicBool,
) -> RunOutcome {
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    let mut command = Command::new(shell);
    command
        .arg(flag)
        .arg(cmd)
        .current_dir(path.as_str())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // `process_group(0)` makes the shell the leader of a new process group whose
    // id is the shell's pid; everything it spawns joins that group unless it
    // asks otherwise. That is what lets a timeout kill grandchildren too.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = match command.spawn() {
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

    let (out_buf, out_reader) = drain(child.stdout.take());
    let (err_buf, err_reader) = drain(child.stderr.take());
    let readers_done = || out_reader.is_finished() && err_reader.is_finished();

    let start = Instant::now();
    let mut exit: Option<i32> = None;
    let mut timed_out = false;
    let mut interrupted = false;
    loop {
        if exit.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => exit = Some(status.code().unwrap_or(-1)),
                Ok(None) => {}
                Err(_) => exit = Some(-1),
            }
        }
        if exit.is_some() && readers_done() {
            break;
        }
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            interrupted = true;
            kill_job(&mut child);
            break;
        }
        if start.elapsed() >= timeout {
            timed_out = true;
            kill_job(&mut child);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    // After a kill the readers see EOF almost at once. Give them a short grace,
    // but never block on them: a process that escaped the group (it called
    // `setsid`, say) could hold the pipe open indefinitely. We then take
    // whatever output arrived and leave the reader threads to finish on their own.
    let grace = Instant::now();
    while !readers_done() && grace.elapsed() < Duration::from_millis(500) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let take = |buf: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>| {
        let bytes = buf.lock().map(|b| b.clone()).unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let mut stderr = take(&err_buf);
    if interrupted {
        // 130: the shell's status for a run ended by SIGINT.
        stderr.push_str("interrupted\n");
    }
    RunOutcome {
        code: match (timed_out, interrupted) {
            (true, _) => -1,
            (_, true) => 130,
            _ => exit.unwrap_or(-1),
        },
        stdout: take(&out_buf),
        stderr,
        timed_out,
    }
}

/// Read `pipe` to EOF on a thread, appending into a shared buffer so the caller
/// can take partial output without joining the thread.
fn drain<R: std::io::Read + Send + 'static>(
    pipe: Option<R>,
) -> (
    std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    std::thread::JoinHandle<()>,
) {
    let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&buf);
    let handle = std::thread::spawn(move || {
        let Some(mut pipe) = pipe else { return };
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => match sink.lock() {
                    Ok(mut b) => b.extend_from_slice(&chunk[..n]),
                    Err(_) => break,
                },
            }
        }
    });
    (buf, handle)
}

/// Kill a timed-out run and everything it started, then reap the shell.
///
/// On unix the shell leads its own process group (see [`run_command_timeout`]),
/// so `kill -s KILL -- -<pgid>` takes down the shell *and* its grandchildren.
/// The crates forbid `unsafe`, so rather than calling `killpg` through `libc`
/// this runs the standard `kill` utility. `child.kill()` follows as a fallback
/// (and is the whole story on Windows, unchanged).
fn kill_job(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-s", "KILL", "--", &format!("-{}", child.id())])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The most telling line of git's stderr, for an error message.
///
/// Not simply the first line: a push always opens with `To <remote-url>`, and
/// many failures lead with `hint:` lines before the verdict. So prefer, in
/// order: a `fatal:` line or a `!` ref-status line (`! [rejected] main -> main
/// (fetch first)`), then an `error:` line, then any other line, and only then
/// `hint:` / `To …` lines. Ties go to the earliest line. Runs of whitespace are
/// collapsed (git pads its ref-status columns).
fn telling_line(bytes: &[u8]) -> String {
    let rank = |line: &str| -> u8 {
        if line.starts_with("fatal:") || line.starts_with('!') {
            0
        } else if line.starts_with("error:") {
            1
        } else if line.starts_with("hint:") || line.starts_with("To ") {
            3
        } else {
            2
        }
    };
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        // `min_by_key` returns the *first* of equally ranked lines.
        .min_by_key(|l| rank(l))
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telling_line_skips_blank_lines_and_falls_back_to_the_first() {
        assert_eq!(telling_line(b"\n  hello \nworld"), "hello");
        assert_eq!(telling_line(b""), "");
    }

    /// A push's stderr opens with `To <url>`; the reason is the `!` line, and the
    /// verdict of a failed `pull --ff-only` is the `fatal:` line after the hints.
    #[test]
    fn telling_line_prefers_the_reason_over_to_url_and_hints() {
        let push = b"To /tmp/remote.git\n ! [remote rejected]   main -> main (pre-receive hook declined)\nerror: failed to push some refs to '/tmp/remote.git'\n";
        assert_eq!(
            telling_line(push),
            "! [remote rejected] main -> main (pre-receive hook declined)"
        );
        let pull = b"hint: Diverging branches can't be fast-forwarded, you need to either:\nhint:\nhint: \tgit merge --no-ff\nfatal: Not possible to fast-forward, aborting.\n";
        assert_eq!(
            telling_line(pull),
            "fatal: Not possible to fast-forward, aborting."
        );
        let fetch = b"fatal: 'origin' does not appear to be a git repository\nfatal: Could not read from remote repository.\n";
        assert_eq!(
            telling_line(fetch),
            "fatal: 'origin' does not appear to be a git repository"
        );
    }

    /// Only "the remote has commits you lack" means "pull first"; a hook or
    /// branch protection refusing the push (`[remote rejected]`) does not.
    #[test]
    fn rejected_as_behind_is_only_the_pull_first_case() {
        assert!(rejected_as_behind(
            b"To x\n ! [rejected]        main -> main (fetch first)\nerror: failed to push some refs to 'x'\n"
        ));
        assert!(rejected_as_behind(
            b"To x\n ! [rejected]        main -> main (non-fast-forward)\n"
        ));
        assert!(!rejected_as_behind(
            b"To x\n ! [remote rejected] main -> main (pre-receive hook declined)\n"
        ));
    }

    /// The real case from review: the remote moved on (another clone pushed), so
    /// our push is rejected. The message must say so and point at pulling, not
    /// echo git's first stderr line (`To <remote-url>`).
    #[test]
    fn rejected_push_says_rejected_and_pull_first_not_the_remote_url() {
        use std::process::Command;
        let tmp = tempfile::tempdir().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let bare = root.join("remote.git");
        let work = root.join("work");
        let other = root.join("other");
        let git = |dir: &Utf8Path, args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir.as_str())
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
        };
        let ok = |args: &[&str]| {
            let out = Command::new("git").args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
        };
        ok(&["init", "--bare", bare.as_str()]);
        ok(&["init", "-b", "main", work.as_str()]);
        git(&work, &["config", "user.email", "t@example.com"]);
        git(&work, &["config", "user.name", "Tester"]);
        git(&work, &["remote", "add", "origin", bare.as_str()]);
        std::fs::write(work.join("a.txt"), "1").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", "one"]);
        git(&work, &["push", "-u", "origin", "main"]);

        // A second clone pushes a commit `work` has never seen…
        ok(&["clone", "-q", "-b", "main", bare.as_str(), other.as_str()]);
        git(&other, &["config", "user.email", "o@example.com"]);
        git(&other, &["config", "user.name", "Other"]);
        std::fs::write(other.join("b.txt"), "from other").unwrap();
        git(&other, &["add", "."]);
        git(&other, &["commit", "-m", "theirs"]);
        git(&other, &["push", "origin", "main"]);

        // …while `work` commits on top of the old tip: the histories diverge.
        std::fs::write(work.join("a.txt"), "2").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", "ours"]);

        let err = push(&work, "work").unwrap_err();
        assert_eq!(err, "push work: rejected (pull first)");
        assert!(!err.contains("To "), "leaked the remote URL line: {err}");

        // Same for the named-branch variant.
        let err = push_branch(&work, "work", "main", false).unwrap_err();
        assert_eq!(err, "push work (main): rejected (pull first)");
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

    /// Ctrl-C at the CLI sets the interrupt flag: the running job (here a
    /// compound command) is killed at once rather than at its timeout.
    #[cfg(unix)]
    #[test]
    fn an_interrupt_stops_a_run_at_once() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = camino::Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let stop = std::sync::atomic::AtomicBool::new(false);
        let started = std::time::Instant::now();
        let r = std::thread::scope(|scope| {
            let run = scope.spawn(|| {
                run_command_until(
                    &dir,
                    "sleep 5; echo x",
                    std::time::Duration::from_secs(30),
                    &stop,
                )
            });
            std::thread::sleep(std::time::Duration::from_millis(300));
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            run.join().unwrap()
        });
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        assert_eq!(r.code, 130);
        assert!(!r.timed_out);
        assert!(r.stderr.contains("interrupted"));
        assert!(!r.stdout.contains('x'), "the rest of the job never ran");
    }

    /// A compound command keeps the shell alive as a parent of `sleep`; killing
    /// only `sh` would leave `sleep` holding our pipes until it finished. The
    /// timeout must stop the whole job promptly.
    #[cfg(unix)]
    #[test]
    fn run_command_timeout_stops_a_compound_command() {
        let dir = camino::Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap();
        let start = std::time::Instant::now();
        let r = run_command_timeout(&dir, "sleep 5; echo x", std::time::Duration::from_secs(1));
        let elapsed = start.elapsed();
        assert!(r.timed_out);
        assert_eq!(r.code, -1);
        assert!(!r.stdout.contains('x'), "the command ran to the end");
        assert!(
            elapsed < std::time::Duration::from_secs(3),
            "took {elapsed:?}"
        );
    }

    /// A background child (`… &`) outlives the shell and keeps the output pipes
    /// open. The run must still return at the deadline with what was printed,
    /// and must not leave the background job running after it returns.
    #[cfg(unix)]
    #[test]
    fn run_command_timeout_kills_background_children_too() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let start = std::time::Instant::now();
        let r = run_command_timeout(
            dir,
            "(sleep 2; touch leaked) & echo started",
            std::time::Duration::from_millis(300),
        );
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(1500),
            "took {elapsed:?}"
        );
        assert!(r.timed_out);
        assert_eq!(r.stdout.trim(), "started");
        // Had the background job survived, it would create the file by now.
        std::thread::sleep(std::time::Duration::from_millis(2200));
        assert!(
            !dir.join("leaked").exists(),
            "background job outlived the run"
        );
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

    /// A branch that was never pushed has no upstream; `push` publishes it
    /// (`-u origin HEAD`) rather than skipping the work most at risk.
    #[test]
    fn push_publishes_a_branch_that_has_no_upstream() {
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
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        Command::new("git")
            .args(["init", "--bare", bare.as_str()])
            .output()
            .unwrap();
        Command::new("git")
            .args(["init", "-b", "feature", work.as_str()])
            .output()
            .unwrap();
        git(&work, &["config", "user.email", "t@example.com"]);
        git(&work, &["config", "user.name", "Tester"]);
        git(&work, &["remote", "add", "origin", bare.as_str()]);
        std::fs::write(work.join("a.txt"), "1").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", "one"]);

        let res = push(&work, "work");
        assert_eq!(res, Ok("published work (new branch on origin)".to_string()));
        // The remote has the branch, and the local branch now tracks it.
        // `--git-dir` names the bare remote outright: `-C` into a bare repo is
        // refused where git runs with `safe.bareRepository=explicit`.
        let git_dir = format!("--git-dir={bare}");
        assert_eq!(
            git(root, &[&git_dir, "rev-parse", "refs/heads/feature"]),
            git(&work, &["rev-parse", "HEAD"])
        );
        assert_eq!(
            git(&work, &["rev-parse", "--abbrev-ref", "feature@{upstream}"]),
            "origin/feature"
        );
    }

    /// `push_branch` pushes a branch that is *not* checked out, setting upstream on
    /// a never-pushed branch. The bare remote gains the ref without a checkout.
    #[test]
    fn push_branch_pushes_a_non_current_branch() {
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
        // A feature branch with its own commit, then back to main so it isn't current.
        git(&work, &["checkout", "-b", "feat"]);
        std::fs::write(work.join("b.txt"), "2").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", "feat work"]);
        git(&work, &["checkout", "main"]);

        let res = push_branch(&work, "work", "feat", true);
        assert_eq!(res, Ok("pushed work (feat)".to_string()));
        // `--git-dir`, not `-C`: see push_publishes_a_branch_that_has_no_upstream.
        let refs = Command::new("git")
            .arg(format!("--git-dir={bare}"))
            .args(["branch", "--list"])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&refs.stdout).contains("feat"));
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
