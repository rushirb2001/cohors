//! Surface-agnostic action orchestration: parse a selector from JSON args,
//! resolve it to action targets, enforce the fan-out cap, preview a dry run,
//! run a per-repo op, and audit the result.
//!
//! Every surface (MCP, web) shares this. What stays surface-side is *policy*:
//! which tier is enabled and whether a `confirm` was given. That's expressed
//! through `git_action`'s `authorize` closure, so each surface composes its own
//! gate while the resolve → cap → dry-run → run → audit flow lives here once.

use camino::Utf8Path;
use cohors_core::{RepoSnapshot, Selector, SortMode, resolve};
use serde_json::{Value, json};

/// Whether the args request a side-effect-free preview.
pub fn is_dry_run(args: &Value) -> bool {
    args.get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Parse an action's selector (required — an empty selector matches nothing).
pub fn action_selector(args: &Value) -> Selector {
    args.get("selector")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

/// Resolve a parsed selector to action targets, scoped to the configured roots,
/// with error/path-less repos excluded since they can't be acted on (ADR-019).
pub fn resolve_targets(selector: &Selector, snaps: &[RepoSnapshot], now: i64) -> Vec<RepoSnapshot> {
    if selector.is_empty() {
        return Vec::new();
    }
    let order = resolve(snaps, selector, SortMode::DirtyFirst, now);
    let by_id: std::collections::HashMap<&str, &RepoSnapshot> =
        snaps.iter().map(|s| (s.id.0.as_str(), s)).collect();
    order
        .iter()
        .filter_map(|id| by_id.get(id.0.as_str()).copied())
        .filter(|s| !s.has_error() && s.path.is_some())
        .cloned()
        .collect()
}

/// Enforce the action-target cap: too many targets without an explicit
/// `{all: true}` is refused (ADR-025), so a fumbled selector can't fan out.
pub fn within_cap(selector: &Selector, targets: &[RepoSnapshot], max: usize) -> Result<(), String> {
    if max > 0 && !selector.all && targets.len() > max {
        Err(format!(
            "{} repos match — over the configured cap of {max}. Narrow the selector, or pass {{\"all\": true}} to act on the whole fleet deliberately.",
            targets.len()
        ))
    } else {
        Ok(())
    }
}

/// Write a one-line audit record of an action to the log (`cohors.log`, ADR-025).
pub fn audit(tool: &str, selector: &Selector, targets: &[RepoSnapshot], ok: usize) {
    let ids: Vec<&str> = targets.iter().map(|s| s.id.0.as_str()).collect();
    tracing::info!(
        target: "cohors::audit",
        tool,
        selector = %serde_json::to_string(selector).unwrap_or_default(),
        targets = targets.len(),
        ok,
        failed = targets.len() - ok,
        ids = ?ids,
        "action",
    );
}

/// Whether `cmd` is permitted by the allowlist (empty allows anything).
pub fn command_allowed(cmd: &str, allowlist: &[String]) -> bool {
    allowlist.is_empty() || allowlist.iter().any(|pat| command_matches(pat, cmd))
}

/// A small `*`-glob match for allowlist patterns like `cargo *` or `git *`.
pub fn command_matches(pattern: &str, cmd: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = cmd.chars().collect();
    let (mut pi, mut ti, mut star, mut resume) = (0usize, 0usize, None, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            resume = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            resume += 1;
            ti = resume;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// The "you selected nothing" result — explicit, so the agent doesn't read an
/// empty action as "done."
pub fn no_targets() -> Value {
    json!({
        "targets": 0,
        "results": [],
        "note": "Selector resolved to 0 repos. An empty selector matches nothing — pass predicates or {\"all\": true}. (Repos that errored or have no local path are excluded from actions.)"
    })
}

/// A `dry_run` preview: the exact target set + the action, with no side effects.
pub fn dry_run_preview(action: Value, targets: &[RepoSnapshot]) -> Value {
    let repos: Vec<Value> = targets
        .iter()
        .map(|s| json!({ "repo": s.id.0, "name": s.name, "path": s.path }))
        .collect();
    json!({ "dry_run": true, "action": action, "targets": repos.len(), "repos": repos })
}

/// Run a per-repo git action (fetch / pull / stash …) over the targets resolved
/// from `args`'s selector and collect `{repo, ok, message}` results.
///
/// `authorize` is the surface's tier+confirm gate; it runs *after* the dry-run
/// shortcut, so a preview works on a read-only surface (ADR-031). `op` is the
/// per-repo primitive from [`crate::git`].
#[allow(clippy::too_many_arguments)]
pub fn git_action(
    tool: &str,
    action: Value,
    args: &Value,
    snaps: &[RepoSnapshot],
    max_targets: usize,
    now: i64,
    authorize: impl Fn() -> Result<(), String>,
    op: impl Fn(&Utf8Path, &str) -> Result<String, String>,
) -> Result<Value, String> {
    let selector = action_selector(args);
    let targets = resolve_targets(&selector, snaps, now);
    if targets.is_empty() {
        return Ok(no_targets());
    }
    within_cap(&selector, &targets, max_targets)?;
    // A dry run is side-effect-free, so it's allowed *before* the gate/confirm:
    // a caller can preview the exact target set (even on a read-only surface) for
    // a human to approve, then enable the tier and act.
    if is_dry_run(args) {
        return Ok(dry_run_preview(action, &targets));
    }
    authorize()?;
    let results: Vec<Value> = targets
        .iter()
        .map(|s| {
            let path = s.path.as_ref().expect("action targets have a path");
            match op(path, &s.name) {
                Ok(message) => json!({ "repo": s.id.0, "ok": true, "message": message }),
                Err(message) => json!({ "repo": s.id.0, "ok": false, "message": message }),
            }
        })
        .collect();
    let ok = results.iter().filter(|r| r["ok"] == true).count();
    audit(tool, &selector, &targets, ok);
    Ok(json!({ "targets": targets.len(), "results": results }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_matches_globs() {
        assert!(command_matches("cargo *", "cargo build --release"));
        assert!(command_matches("git *", "git status -s"));
        assert!(!command_matches("git *", "rm -rf /"));
        assert!(command_matches("*", "anything goes"));
        assert!(command_matches("exact", "exact"));
        assert!(!command_matches("exact", "exactly"));
    }
}
