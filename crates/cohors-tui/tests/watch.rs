//! `cohors watch --json`: one NDJSON line when the fleet is first read, one
//! per change after that, silence otherwise, and a clean end when its input
//! or its output closes.
//!
//! Every run gets its own HOME (config, cache, launchpad home and agent
//! sessions all live under it), so nothing reads the developer's own.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Long enough for a slow CI machine; a passing run takes a few seconds.
const WAIT: Duration = Duration::from_secs(20);

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

fn make_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["-c", "init.defaultBranch=main", "init", "-q"]);
    git(dir, &["config", "user.email", "dev@example.com"]);
    git(dir, &["config", "user.name", "Dev"]);
    std::fs::write(dir.join("README.md"), "hi").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "init"]);
}

/// A home with two repos under `code/` and a config naming that root.
struct Home {
    _tmp: tempfile::TempDir,
    home: PathBuf,
}

impl Home {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        make_repo(&home.join("code/alpha"));
        make_repo(&home.join("code/beta"));
        let config = home.join(".config/cohors/config.toml");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, format!("roots = [\"{}/code\"]\n", home.display())).unwrap();
        Self { _tmp: tmp, home }
    }

    fn repo(&self, name: &str) -> PathBuf {
        self.home.join("code").join(name)
    }

    /// `cohors watch --json`, with its input and output piped to the test,
    /// never looking for a newer cohors.
    fn spawn(&self) -> Child {
        let mut cmd = self.command();
        cmd.env("COHORS_NO_UPDATE", "1");
        cmd.spawn().expect("cohors starts")
    }

    /// The command [`Home::spawn`] runs, before the update switch.
    fn command(&self) -> Command {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin("cohors"));
        cmd.args(["watch", "--json"])
            .current_dir(&self.home)
            .env("HOME", &self.home)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_CACHE_HOME")
            .env_remove("COHORS_HOME")
            .env_remove("COHORS_NO_UPDATE")
            .env_remove("COHORS_DISABLE_UPDATES")
            .env_remove("CI")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    /// [`Home::spawn`], with every output line parsed on a reader thread.
    fn watch(&self) -> (Child, Receiver<Value>) {
        let child = self.spawn();
        Self::lines(child)
    }

    /// Every output line of `child`, parsed on a reader thread.
    fn lines(mut child: Child) -> (Child, Receiver<Value>) {
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let value: Value = serde_json::from_str(&line)
                    .unwrap_or_else(|e| panic!("not one JSON object per line ({e}): {line}"));
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        (child, rx)
    }
}

fn next(rx: &Receiver<Value>) -> Value {
    rx.recv_timeout(WAIT).expect("a line for the change")
}

/// Lines until one satisfies `done` (a git command can land in two steps).
fn until(rx: &Receiver<Value>, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = rx.recv_timeout(left).expect("a line showing the change");
        if done(&line) {
            return line;
        }
    }
}

fn repo_line<'a>(line: &'a Value, name: &str) -> &'a Value {
    line["overview"]["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("{name} is not in the line"))
}

fn wait_for_exit(child: &mut Child) -> std::process::ExitStatus {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "cohors watch did not exit");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_line_per_change_and_silence_in_between() {
    let home = Home::new();
    let (mut child, rx) = home.watch();

    let first = next(&rx);
    assert_eq!(first["type"], "fleet");
    assert_eq!(first["seq"], 1);
    assert_eq!(first["reason"], "start");
    // The overview is what `cohors overview --json` prints, roots included;
    // the launchpad what `cohors launchpad --json` prints.
    assert_eq!(first["overview"]["repos"].as_array().unwrap().len(), 2);
    assert_eq!(first["overview"]["figures"]["repos"], 2);
    assert!(
        first["overview"]["roots"][0]
            .as_str()
            .unwrap()
            .ends_with("/code")
    );
    assert_eq!(first["launchpad"]["repos"].as_array().unwrap().len(), 2);
    assert_eq!(first["launchpad"]["summary"]["repos"], 2);
    assert!(first["suggestions"].is_array());

    // Nothing changes: nothing is printed (the watch keeps probing and
    // rescanning meanwhile).
    assert!(
        rx.recv_timeout(Duration::from_secs(4)).is_err(),
        "a line was printed with nothing changed"
    );

    // A commit in one repo.
    let alpha = home.repo("alpha");
    std::fs::write(alpha.join("new.txt"), "x").unwrap();
    git(&alpha, &["add", "."]);
    git(&alpha, &["commit", "-q", "-m", "second"]);
    let line = until(&rx, |line| {
        repo_line(line, "alpha")["commit"]["summary"] == "second"
    });
    assert!(line["seq"].as_u64().unwrap() >= 2);
    assert!(
        line["changed"]
            .as_array()
            .unwrap()
            .iter()
            .all(|id| id.as_str().unwrap().ends_with("/alpha")),
        "only alpha changed: {}",
        line["changed"]
    );

    // An edited file (git records nothing): the periodic rescan finds it.
    std::fs::write(home.repo("beta").join("README.md"), "edited").unwrap();
    until(&rx, |line| repo_line(line, "beta")["modified"] == 1);

    // A new repo under the root.
    make_repo(&home.repo("gamma"));
    until(&rx, |line| line["overview"]["figures"]["repos"] == 3);

    // Closing its input (the program that started it went away) ends it.
    drop(child.stdin.take());
    let status = wait_for_exit(&mut child);
    assert!(status.success(), "{status}");
}

#[test]
fn a_newer_cohors_rides_on_the_stream_in_the_words_every_surface_uses() {
    let home = Home::new();
    // The update source, pointed at a local file (the test seam), says a
    // newer release is out. Without it the line carries `update: null`.
    let latest = home.home.join("latest.txt");
    std::fs::write(&latest, "99.0.0\n").unwrap();
    let mut cmd = home.command();
    cmd.env("COHORS_UPDATE_SOURCE", &latest);
    let (mut child, rx) = Home::lines(cmd.spawn().expect("cohors starts"));

    // The background look may land before or after the first line; either
    // way a line carries it within seconds.
    let line = until(&rx, |line| line["update"]["version"] == "99.0.0");
    assert!(
        line["update"]["notice"]
            .as_str()
            .unwrap()
            .starts_with("Update available: 99.0.0"),
        "{}",
        line["update"]
    );
    assert_eq!(line["update"]["detail"]["state"], "available");
    drop(child.stdin.take());
    assert!(wait_for_exit(&mut child).success());

    // Told not to look, it says nothing about updates.
    let (mut child, rx) = home.watch();
    assert!(next(&rx)["update"].is_null());
    drop(child.stdin.take());
    assert!(wait_for_exit(&mut child).success());
}

#[test]
fn a_closed_output_ends_the_stream_quietly() {
    let home = Home::new();
    let mut child = home.spawn();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let first: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(first["seq"], 1);
    // The reader goes away (`cohors watch --json | head -1`)...
    drop(reader);
    // ...and the next line has nowhere to go. The input stays open, so it is
    // the closed output that ends the stream.
    git(
        &home.repo("alpha"),
        &["commit", "-q", "--allow-empty", "-m", "more"],
    );
    let status = wait_for_exit(&mut child);
    let mut stderr = String::new();
    std::io::Read::read_to_string(child.stderr.as_mut().unwrap(), &mut stderr).unwrap();
    assert!(status.success(), "{status}: {stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
    let _ = child.stdin.as_mut().map(|input| input.write_all(b""));
}
