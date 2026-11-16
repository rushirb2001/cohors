//! `cohors install`, `cohors doctor` and `cohors update` end to end, in a
//! temporary home, against a fake release served from a folder through the
//! test seam (`COHORS_UPDATE_SOURCE`, `COHORS_UPDATE_RELEASES`). Nothing is
//! downloaded.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command as Std;

use assert_cmd::Command;
use sha2::{Digest, Sha256};

fn target() -> &'static str {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("aarch64", "macos") => "aarch64-apple-darwin",
        ("x86_64", "macos") => "x86_64-apple-darwin",
        ("aarch64", "linux") => "aarch64-unknown-linux-gnu",
        _ => "x86_64-unknown-linux-gnu",
    }
}

/// `releases/v<version>/cohors-<target>.tar.xz` and its `.sha256`, holding a
/// script that answers `--version` like the real binary.
fn fake_release(releases: &Path, version: &str) {
    let folder = releases.join(format!("v{version}"));
    let name = format!("cohors-{}", target());
    let pack = folder.join("pack").join(&name);
    std::fs::create_dir_all(&pack).unwrap();
    let script = pack.join("cohors");
    std::fs::write(&script, format!("#!/bin/sh\necho 'cohors {version}'\n")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let archive = format!("{name}.tar.xz");
    let packed = Std::new("tar")
        .arg("-cJf")
        .arg(folder.join(&archive))
        .arg("-C")
        .arg(folder.join("pack"))
        .arg(&name)
        .status()
        .unwrap();
    assert!(packed.success());
    let sum: String = Sha256::digest(std::fs::read(folder.join(&archive)).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::fs::write(
        folder.join(format!("{archive}.sha256")),
        format!("{sum} *{archive}\n"),
    )
    .unwrap();
}

/// cohors in a temporary home whose PATH has the managed launcher's folder
/// first, looking up versions in `latest.txt` and downloading from
/// `releases/`.
fn cohors(root: &Path) -> Command {
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut cmd = Command::cargo_bin("cohors").unwrap();
    cmd.env("HOME", &home)
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", home.join(".local/bin").display()),
        )
        .env("CARGO_HOME", home.join(".cargo"))
        .env("COHORS_UPDATE_SOURCE", root.join("latest.txt"))
        .env("COHORS_UPDATE_RELEASES", root.join("releases"));
    for name in [
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_BIN_HOME",
        "COHORS_INSTALLER",
        "COHORS_PYTHON",
        "COHORS_NO_UPDATE",
        "COHORS_DISABLE_UPDATES",
    ] {
        cmd.env_remove(name);
    }
    cmd
}

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stdout).into_owned()
}

#[test]
fn install_sets_up_the_self_updating_copy_and_doctor_reports_it() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fake_release(&root.join("releases"), "0.7.8");
    std::fs::write(root.join("latest.txt"), "0.7.8\n").unwrap();

    let out = stdout(&cohors(root).arg("install").assert().success());
    assert!(out.contains("Installing cohors 0.7.8"), "{out}");
    assert!(
        out.contains("~/.local/bin/cohors → ~/.local/share/cohors/versions/0.7.8/cohors"),
        "{out}"
    );
    assert!(out.contains("comes first on your PATH"), "{out}");
    let launcher = root.join("home/.local/bin/cohors");
    let said = Std::new(&launcher).arg("--version").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&said.stdout).trim(), "cohors 0.7.8");

    let report = stdout(&cohors(root).arg("doctor").assert().success());
    for wanted in [
        "cohors doctor",
        "1. ~/.local/bin/cohors",
        "cohors install",
        "versions  0.7.8 (active)",
        "0.7.8 with cohors install",
    ] {
        assert!(report.contains(wanted), "missing {wanted:?} in:\n{report}");
    }
}

#[test]
fn install_says_when_the_launcher_folder_is_not_on_the_path() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fake_release(&root.join("releases"), "0.7.8");
    let out = stdout(
        &cohors(root)
            .env("PATH", "/usr/bin:/bin")
            .args(["install", "0.7.8"])
            .assert()
            .success(),
    );
    assert!(out.contains("~/.local/bin is not on your PATH"), "{out}");
    assert!(
        out.contains("export PATH=\"$HOME/.local/bin:$PATH\""),
        "{out}"
    );
}

#[test]
fn a_release_that_fails_its_checksum_is_never_installed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fake_release(&root.join("releases"), "0.7.8");
    let sums = root
        .join("releases/v0.7.8")
        .join(format!("cohors-{}.tar.xz.sha256", target()));
    std::fs::write(&sums, format!("{} *x\n", "0".repeat(64))).unwrap();
    let failed = cohors(root).args(["install", "0.7.8"]).assert().failure();
    let err = String::from_utf8_lossy(&failed.get_output().stderr).into_owned();
    assert!(err.contains("checksum"), "{err}");
    assert!(!root.join("home/.local/bin/cohors").exists());
    assert!(
        !root
            .join("home/.local/share/cohors/versions/0.7.8")
            .exists()
    );
    // doctor shows the failed attempt and how to recover.
    let report = stdout(&cohors(root).arg("doctor").assert().success());
    assert!(report.contains(": failed"), "{report}");
}

#[test]
fn update_check_asks_the_source_and_disable_updates_stops_everything() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("latest.txt"), "9.9.9\n").unwrap();
    let json = stdout(
        &cohors(root)
            .args(["update", "--check", "--json"])
            .assert()
            .success(),
    );
    let answer: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(answer["newer"], "9.9.9");
    assert_eq!(answer["reached"], true);

    for args in [&["update"][..], &["install"][..]] {
        let refused = cohors(root)
            .env("COHORS_DISABLE_UPDATES", "1")
            .args(args)
            .assert()
            .failure();
        let err = String::from_utf8_lossy(&refused.get_output().stderr).into_owned();
        assert!(err.contains("COHORS_DISABLE_UPDATES"), "{args:?}: {err}");
    }
}

#[test]
fn uninstall_lists_with_dry_run_then_removes_only_what_is_ours() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fake_release(&root.join("releases"), "0.7.8");
    cohors(root).args(["install", "0.7.8"]).assert().success();
    let home = root.join("home");
    let config = home.join(".config/cohors/config.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "roots = []\n").unwrap();

    let listed = stdout(
        &cohors(root)
            .args(["uninstall", "--dry-run"])
            .assert()
            .success(),
    );
    for wanted in [
        "Would remove:",
        "~/.local/bin/cohors",
        "~/.local/share/cohors/versions",
        "~/.local/state/cohors/last-update.json",
    ] {
        assert!(listed.contains(wanted), "missing {wanted:?} in:\n{listed}");
    }
    assert!(home.join(".local/bin/cohors").exists());
    assert!(
        home.join(".local/share/cohors/versions/0.7.8/cohors")
            .exists()
    );

    let removed = stdout(&cohors(root).arg("uninstall").assert().success());
    assert!(removed.contains("Removed:"), "{removed}");
    assert!(std::fs::symlink_metadata(home.join(".local/bin/cohors")).is_err());
    assert!(!home.join(".local/share/cohors").exists());
    assert!(!home.join(".local/state/cohors").exists());
    // The config is not cohors' install to remove.
    assert!(config.exists());
    let again = stdout(&cohors(root).arg("uninstall").assert().success());
    assert!(again.contains("nothing to remove"), "{again}");
}

#[test]
fn a_dev_build_never_sets_up_or_hands_off_even_as_a_homebrew_copy() {
    // The test binary is a debug build. Put it where Homebrew would, so it is
    // recognised as a Homebrew copy, and give it a newer managed copy to hand
    // off to: it must run itself, and set nothing up.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let cellar = root.join("brew/Cellar/cohors/0.7.7/bin");
    std::fs::create_dir_all(&cellar).unwrap();
    std::fs::copy(
        assert_cmd::cargo::cargo_bin("cohors"),
        cellar.join("cohors"),
    )
    .unwrap();
    let newer = root.join("home/.local/share/cohors/versions/9.9.9");
    std::fs::create_dir_all(&newer).unwrap();
    std::fs::write(newer.join("cohors"), "#!/bin/sh\necho 'cohors 9.9.9'\n").unwrap();
    std::fs::set_permissions(newer.join("cohors"), std::fs::Permissions::from_mode(0o755)).unwrap();

    let brew = || {
        let mut cmd = cohors(root);
        cmd.env("COHORS_UPDATE_SOURCE", root.join("nothing-here"));
        let program = cellar.join("cohors");
        let mut real = Command::new(program);
        // Same environment as `cohors(root)`, but this binary.
        for (key, value) in cmd.get_envs() {
            match value {
                Some(value) => real.env(key, value),
                None => real.env_remove(key),
            };
        }
        real
    };
    let report = stdout(&brew().arg("doctor").assert().success());
    assert!(report.contains("installed with Homebrew"), "{report}");
    assert!(report.contains("a dev build never hands off"), "{report}");
    let _ = brew().args(["path", "x", "--root"]).arg(root).assert();
    assert!(
        !root
            .join("home/.local/share/cohors/versions/0.7.7")
            .exists()
    );
    assert!(!root.join("home/.local/state/cohors/handoff.json").exists());
}
