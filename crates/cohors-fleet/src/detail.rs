//! The one-repo drill-in read, composed once.
//!
//! A "full look at one repo" is three reads — the local detail (commits /
//! branches / stashes), the working-tree changes (file list + capped patch),
//! and the remote detail (PRs / contributors / issues / release). Every surface
//! that offers a drill-in wants the same trio, so the composition lives here
//! rather than being re-assembled per front-end.

use camino::Utf8Path;
use cohors_core::{RemoteDetail, RepoChanges, RepoDetail};
use serde::Serialize;

/// Everything a drill-in view needs for one repo. Serializable as-is, so an
/// HTTP surface can return it directly.
#[derive(Serialize)]
pub struct DetailBundle {
    pub local: RepoDetail,
    pub changes: RepoChanges,
    pub remote: Option<RemoteDetail>,
}

/// Assemble the [`DetailBundle`] for one repo. `path` drives the two local
/// reads (skipped to defaults when `None`); `remote_url` + `token` drive the
/// remote one (skipped when either is absent). `patch_bytes` caps the diff.
pub fn detail_bundle(
    path: Option<&Utf8Path>,
    remote_url: Option<&str>,
    token: Option<&str>,
    patch_bytes: usize,
) -> DetailBundle {
    let (local, changes) = match path {
        Some(p) => (
            cohors_git::repo_detail(p),
            cohors_git::repo_changes(p, true, patch_bytes),
        ),
        None => (RepoDetail::default(), RepoChanges::default()),
    };
    let remote = match (token, remote_url) {
        (Some(t), Some(url)) if !url.is_empty() => cohors_github::fetch_repo_detail(t, url),
        _ => None,
    };
    DetailBundle {
        local,
        changes,
        remote,
    }
}
