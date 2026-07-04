//! One verb → primitive dispatch for every machine surface.
//!
//! The registry (`registry.rs`) says *what* actions exist; `orchestrate.rs`
//! says *how* a selector-driven action flows. This module closes the loop:
//! given a verb string and JSON args, run the whole thing — so the MCP and the
//! web server stop re-mapping verbs to primitives and gates independently.
//! Each surface supplies only its [`Caps`] and a `gate_hint` (the command a
//! human would run to enable a tier, e.g. `cohors mcp` / `cohors web`).

use cohors_core::RepoSnapshot;
use serde_json::{Value, json};

use crate::registry::Tier;

/// Which mutating tiers the surface has enabled (ADR-025). Enforcement happens
/// here (one implementation); *choosing* the tiers stays at each surface's door.
#[derive(Debug, Clone, Copy, Default)]
pub struct Caps {
    pub allow_writes: bool,
    pub allow_run: bool,
}

/// Run `verb` over the repos matching `args`'s selector, applying the shared
/// gate policy: `dry_run` previews before any gate; Write-tier verbs need
/// `caps.allow_writes` (+ `confirm:true` when the registry says so); `run`
/// needs `caps.allow_run` + confirm + the allowlist. Returns the same
/// `{targets, results}` / preview / error shapes every surface already speaks.
#[allow(clippy::too_many_arguments)]
pub fn dispatch(
    verb: &str,
    args: &Value,
    snaps: &[RepoSnapshot],
    caps: Caps,
    allowlist: &[String],
    max_targets: usize,
    now: i64,
    gate_hint: &str,
) -> Result<Value, String> {
    let def = crate::registry::find(verb)
        .ok_or_else(|| format!("unknown action `{verb}` (no such registry verb)"))?;

    match def.tier {
        Tier::Write => {
            // `commit` carries a required message; the other verbs take none.
            let message = if verb == "commit" {
                Some(
                    args.get("message")
                        .and_then(Value::as_str)
                        .filter(|m| !m.trim().is_empty())
                        .ok_or("missing required argument `message`")?
                        .to_string(),
                )
            } else {
                None
            };
            let needs_confirm = def.needs_confirm;
            let authorize = || {
                if !caps.allow_writes {
                    return Err(format!(
                        "`{verb}` is disabled: this server is read-only. Relaunch it with `{gate_hint} --allow-writes` to enable write tools."
                    ));
                }
                if needs_confirm {
                    require_confirm(args, verb)?;
                }
                Ok(())
            };
            let label = match verb {
                "commit" => json!({ "commit": message.clone().unwrap_or_default() }),
                "pull" => json!("pull --ff-only"),
                "stash" => json!("stash push"),
                other => json!(other),
            };
            // The worktree-mutating verbs skip conflicted / mid-operation repos
            // (see `worktree_block`); fetch and push never touch the worktree.
            let guard_worktree = matches!(verb, "pull" | "commit" | "stash");
            crate::orchestrate::git_action(
                verb,
                label,
                args,
                snaps,
                max_targets,
                now,
                guard_worktree,
                authorize,
                |path, name| match verb {
                    "fetch" => crate::git::fetch(path, name),
                    "pull" => crate::git::pull_ff(path, name),
                    "push" => crate::git::push(path, name),
                    "commit" => {
                        crate::git::commit(path, name, message.as_deref().expect("commit message"))
                    }
                    "stash" => crate::git::stash_push(path, name),
                    other => Err(format!("`{other}` is not a git write action")),
                },
            )
        }
        Tier::Run => run_dispatch(args, snaps, caps, allowlist, max_targets, now, gate_hint),
        other => Err(format!(
            "`{verb}` has tier {other:?}, which selector dispatch doesn't expose"
        )),
    }
}

/// The `run` flow: command required; dry-run previews pre-gate; then the run
/// tier, a deliberate confirm, and the allowlist; fan out on the bounded pool.
fn run_dispatch(
    args: &Value,
    snaps: &[RepoSnapshot],
    caps: Caps,
    allowlist: &[String],
    max_targets: usize,
    now: i64,
    gate_hint: &str,
) -> Result<Value, String> {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .filter(|c| !c.trim().is_empty())
        .ok_or("missing required argument `command`")?
        .to_string();

    let selector = crate::orchestrate::action_selector(args);
    let targets = crate::orchestrate::resolve_targets(&selector, snaps, now);
    if targets.is_empty() {
        return Ok(crate::orchestrate::no_targets());
    }
    crate::orchestrate::within_cap(&selector, &targets, max_targets)?;
    if crate::orchestrate::is_dry_run(args) {
        return Ok(crate::orchestrate::dry_run_preview(
            json!({ "run": command }),
            &targets,
        ));
    }
    if !caps.allow_run {
        return Err(format!(
            "`run` is disabled: relaunch the server with `{gate_hint} --allow-run` to enable arbitrary commands."
        ));
    }
    require_confirm(args, "run")?;
    if !crate::orchestrate::command_allowed(&command, allowlist) {
        return Err(format!(
            "`{command}` is not permitted by the configured run_allowlist ({allowlist:?})."
        ));
    }

    let timeout = std::time::Duration::from_secs(
        args.get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(crate::runner::DEFAULT_RUN_TIMEOUT_SECS)
            .max(1),
    );
    let run_id = crate::runner::next_run_id();
    let results = crate::runner::run_each(&targets, &command, timeout);
    let ok = results.iter().filter(|r| r.ok).count();
    crate::orchestrate::audit("run", &selector, &targets, ok);
    Ok(json!({
        "run_id": run_id, "command": command, "targets": targets.len(),
        "ok": ok, "failed": targets.len() - ok, "results": results
    }))
}

/// `confirm: true` gate for the destructive / arbitrary-shell verbs.
fn require_confirm(args: &Value, verb: &str) -> Result<(), String> {
    if args.get("confirm").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(format!(
            "`{verb}` needs a deliberate `confirm`: pass \"confirm\": true (preview first with \"dry_run\": true)."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;
    fn fleet() -> Vec<RepoSnapshot> {
        cohors_core::demo::fleet(NOW)
    }
    fn call(verb: &str, args: Value, caps: Caps) -> Result<Value, String> {
        dispatch(verb, &args, &fleet(), caps, &[], 0, NOW, "cohors test")
    }

    #[test]
    fn write_verb_is_gated_and_names_the_hint() {
        let err = call("fetch", json!({"selector": {"all": true}}), Caps::default()).unwrap_err();
        assert!(err.contains("cohors test --allow-writes"), "{err}");
    }

    #[test]
    fn dry_run_previews_before_any_gate() {
        let v = call(
            "stash",
            json!({"selector": {"all": true}, "dry_run": true}),
            Caps::default(),
        )
        .unwrap();
        assert_eq!(v["dry_run"], true);
    }

    #[test]
    fn confirm_verbs_reject_without_confirm_even_with_writes() {
        let caps = Caps {
            allow_writes: true,
            allow_run: false,
        };
        let err = call("stash", json!({"selector": {"all": true}}), caps).unwrap_err();
        assert!(err.contains("confirm"), "{err}");
        let err = call(
            "commit",
            json!({"selector": {"all": true}, "message": "m"}),
            caps,
        )
        .unwrap_err();
        assert!(err.contains("confirm"), "{err}");
    }

    #[test]
    fn commit_requires_a_message_before_anything() {
        let err = call(
            "commit",
            json!({"selector": {"all": true}}),
            Caps::default(),
        )
        .unwrap_err();
        assert!(err.contains("message"), "{err}");
    }

    #[test]
    fn worktree_mutations_skip_conflicted_and_mid_operation_repos() {
        let mut repos = fleet();
        repos[0].worktree.conflicted = 1;
        repos[1].operation = Some(cohors_core::RepoOperation::Merge);
        // Make the two guarded repos unambiguously actionable (a target must be
        // readable and have a path), whatever the demo fleet's defaults are.
        for r in &mut repos[..2] {
            r.error = None;
            r.path = Some("/nonexistent/cohors-guard-test".into());
        }
        let caps = Caps {
            allow_writes: true,
            allow_run: false,
        };
        let v = dispatch(
            "stash",
            &json!({"selector": {"all": true}, "confirm": true}),
            &repos,
            caps,
            &[],
            0,
            NOW,
            "t",
        )
        .unwrap();
        let results = v["results"].as_array().unwrap();
        let skipped: Vec<&Value> = results.iter().filter(|r| r["skipped"] == true).collect();
        assert_eq!(skipped.len(), 2, "results: {results:?}");
        assert!(skipped.iter().all(|r| r["ok"] == false));
        // The skip message says why, so a bulk result reads as triage.
        assert!(
            skipped
                .iter()
                .any(|r| r["message"].as_str().unwrap().contains("conflict"))
        );
        assert!(
            skipped
                .iter()
                .any(|r| r["message"].as_str().unwrap().contains("merge in progress"))
        );
    }

    #[test]
    fn run_is_gated_separately_and_unknown_verbs_rejected() {
        let caps = Caps {
            allow_writes: true,
            allow_run: false,
        };
        let err = call(
            "run",
            json!({"selector": {"all": true}, "command": "echo hi", "confirm": true}),
            caps,
        )
        .unwrap_err();
        assert!(err.contains("--allow-run"), "{err}");
        assert!(call("nuke", json!({}), Caps::default()).is_err());
    }
}
