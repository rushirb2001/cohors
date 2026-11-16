//! `cohors uninstall`: remove cohors' own install and what its updates keep
//! (the versions, the launcher link when it is ours, the update state, lock
//! and memo), then say how to remove any copy a package manager installed.
//! Nothing cohors did not make is touched: a launcher that is not our link,
//! the config, the scan cache and the logs stay.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use cohors_core::update::Channel;

use super::ctx;
use super::doctor::{classify, holders};
use super::managed::{self, Layout, Lock};

pub fn run(dry_run: bool) -> Result<()> {
    let ctx = ctx().ok_or_else(|| anyhow!("could not find your home folder"))?;
    let layout = &ctx.layout;
    // What is there before this run takes the lock (a lock left by a crash is
    // listed; the one taken here is removed with its folder).
    let items = ours(layout);
    let lock = if dry_run {
        None
    } else {
        Some(Lock::take(layout).ok_or_else(|| {
            anyhow!("another cohors is updating right now; try again in a minute")
        })?)
    };
    if items.is_empty() {
        println!("cohors' own install is not set up: nothing to remove.");
    } else {
        println!("{}", if dry_run { "Would remove:" } else { "Removed:" });
        for path in &items {
            if !dry_run {
                remove(path)?;
            }
            println!("  {}", layout.show(path));
        }
    }
    drop(lock);
    if !dry_run {
        tidy(layout);
    }

    // Copies a package manager installed: cohors does not remove those.
    let mut others: Vec<(String, Channel)> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut note = |path: &Path, channel: Channel| {
        let real = std::fs::canonicalize(path).unwrap_or(path.to_path_buf());
        if channel != Channel::Managed && !seen.contains(&real) {
            seen.push(real);
            others.push((layout.show(path), channel));
        }
    };
    if let Some(running) = &ctx.running {
        note(running, ctx.installed_as.clone());
    }
    for path in holders() {
        let found = classify(layout, &path);
        note(&found.path, found.channel);
    }
    let mut hands_off = false;
    for (path, channel) in &others {
        match channel.uninstall_command() {
            Some(command) => println!(
                "cohors is also installed with {} ({path}): remove it with `{command}`.",
                channel.label()
            ),
            None => println!("cohors is also at {path} ({}).", channel.label()),
        }
        hands_off |= channel.hands_off();
    }
    if hands_off && !dry_run {
        println!(
            "Until it is removed, that copy sets cohors' own install up again the next time it runs. To keep it without one, set updates = \"notify\" in the config."
        );
    }
    Ok(())
}

/// What `cohors uninstall` removes, of what exists.
pub(crate) fn ours(layout: &Layout) -> Vec<PathBuf> {
    let exists = |path: &Path| std::fs::symlink_metadata(path).is_ok();
    let mut items = Vec::new();
    // The launcher, only when it is our link into versions/. (On Windows it is
    // a copy that cannot be told apart, so it is left.)
    if cfg!(unix) && exists(&layout.launcher()) && managed::foreign_launcher(layout).is_none() {
        items.push(layout.launcher());
    }
    for path in [
        layout.versions(),
        layout.handoff_file(),
        layout.last_attempt_file(),
        layout.lock_file(),
        layout.memo(),
        layout.staging(),
    ] {
        if exists(&path) {
            items.push(path);
        }
    }
    items
}

fn remove(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Remove the folders the removed files leave empty (and only empty ones).
fn tidy(layout: &Layout) {
    for dir in [
        layout.lock_file().parent().map(Path::to_path_buf),
        Some(layout.state.clone()),
        Some(layout.data.clone()),
    ]
    .into_iter()
    .flatten()
    {
        let _ = std::fs::remove_dir(dir);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::update::managed::tests::installed;

    #[test]
    fn uninstall_lists_then_removes_only_what_is_ours() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = Layout::under(tmp.path());
        let binary = installed(&layout, "0.7.8");
        managed::point_launcher(&layout, &binary).unwrap();
        managed::write_atomically(&layout.handoff_file(), b"{}").unwrap();
        managed::write_atomically(&layout.memo(), b"{}").unwrap();
        // Not ours: the scan cache beside the memo, and the config.
        managed::write_atomically(&layout.cache.join("cache.json"), b"[]").unwrap();
        managed::write_atomically(&layout.config_home.join("cohors/config.toml"), b"").unwrap();

        let listed = ours(&layout);
        assert_eq!(
            listed,
            [
                layout.launcher(),
                layout.versions(),
                layout.handoff_file(),
                layout.memo()
            ]
        );
        // The dry run is the listing alone: everything is still there.
        assert!(layout.launcher().exists() && binary.exists());

        for path in &listed {
            remove(path).unwrap();
        }
        tidy(&layout);
        assert!(ours(&layout).is_empty());
        assert!(!layout.data.exists() && !layout.state.exists());
        assert!(layout.cache.join("cache.json").exists());
        assert!(layout.config_home.join("cohors/config.toml").exists());
    }

    #[test]
    fn a_launcher_that_is_not_ours_is_never_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = Layout::under(tmp.path());
        installed(&layout, "0.7.8");
        std::fs::create_dir_all(&layout.bin).unwrap();
        std::fs::write(layout.launcher(), "#!/usr/bin/env python3\n").unwrap();
        assert_eq!(ours(&layout), [layout.versions()]);
    }
}
