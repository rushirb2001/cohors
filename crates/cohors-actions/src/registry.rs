//! The one enumerable registry of mutating actions.
//!
//! Every surface reads this: the TUI binds keys/commands to these verbs, the MCP
//! generates its tool catalog from them, and the web renders buttons from them. A
//! structural parity test walks `registry()` and asserts each verb is wired to
//! every surface — so adding an action here without wiring a surface fails the
//! build, rather than drifting silently (the old hardcoded list could not catch
//! that). See MCP-DESIGN §10.

/// Which capability gate an action sits behind. The gate is enforced per-surface
/// (the MCP's `--allow-*` flags, the web server's flags); the tier names which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Read-only (no action verbs use this yet; reserved for symmetry).
    Read,
    /// Mutates git state but cannot lose work — needs the write gate
    /// (`--allow-writes`). fetch / pull / push / commit.
    Write,
    /// Arbitrary shell — needs the separate run gate (`--allow-run`).
    Run,
    /// Desktop-only handoff (reveal, clipboard, editor); never exposed to MCP/web.
    Local,
}

/// One mutating action's identity and safety metadata. The catalog description,
/// argument schema, and key bindings are derived from this on each surface.
#[derive(Debug, Clone, Copy)]
pub struct ActionDef {
    /// The stable verb: the MCP tool name, the CLI subcommand, the web button id.
    pub verb: &'static str,
    /// Which capability gate this action sits behind.
    pub tier: Tier,
    /// Requires a deliberate `confirm: true` (MCP/web) / a confirm modal (TUI).
    pub needs_confirm: bool,
    /// Could surprise the user, so it's never bound to a bare keystroke (ADR-008).
    pub destructive: bool,
    /// One-line description, used verbatim as the MCP catalog `description`.
    pub summary: &'static str,
}

/// The full set of mutating actions, in catalog order. Single source of truth.
pub fn registry() -> &'static [ActionDef] {
    const R: &[ActionDef] = &[
        ActionDef {
            verb: "fetch",
            tier: Tier::Write,
            needs_confirm: false,
            destructive: false,
            summary: "git fetch across the selected repos — non-destructive (updates remote-tracking refs only). Use to refresh remote state before checking what is behind. Requires a selector. Executing needs the server launched with --allow-writes; without it (or with dry_run:true) the call just previews the resolved target set.",
        },
        ActionDef {
            verb: "pull",
            tier: Tier::Write,
            needs_confirm: false,
            destructive: false,
            summary: "git pull --ff-only across the selected repos — fast-forward only, so it never merges, rebases, or loses work. Typical flow: find what is behind with {\"behind\": true}, then pull those. Requires a selector. Executing needs --allow-writes; dry_run:true (or a read-only server) previews the targets.",
        },
        ActionDef {
            verb: "push",
            tier: Tier::Write,
            needs_confirm: false,
            destructive: false,
            summary: "git push the current branch to its upstream across the selected repos. Never force-pushes (git rejects non-fast-forward pushes), so it can't overwrite remote history. Pair after commit to land a cross-repo change. Requires a selector. Executing needs --allow-writes; dry_run:true previews the targets.",
        },
        ActionDef {
            verb: "commit",
            tier: Tier::Write,
            needs_confirm: true,
            destructive: false,
            summary: "git add -A + git commit -m <message> across the selected repos (stages tracked AND untracked; \"nothing to commit\" is a no-op). Never amends or rewrites history. Pair with push to finish a cross-repo change. Requires selector and message. Executing needs --allow-writes AND confirm:true; dry_run:true previews the target set with neither.",
        },
        ActionDef {
            verb: "stash",
            tier: Tier::Write,
            needs_confirm: true,
            destructive: true,
            summary: "git stash push (tracked changes) across the selected repos — a safe way to park work, e.g. before a bulk pull or run on dirty repos. Requires a selector. Executing needs --allow-writes AND confirm:true; dry_run:true previews the target set with neither.",
        },
        ActionDef {
            verb: "run",
            tier: Tier::Run,
            needs_confirm: true,
            destructive: true,
            summary: "Run one shell command in each selected repo and collect per-repo {exit_code, stdout, stderr, truncated, timed_out} — the fleet codemod/audit/test primitive (e.g. \"cargo fmt --check\", \"rg TODO\", \"npm test\"). The command runs in each repo's own directory. Requires command and selector. Executing needs --allow-run AND confirm:true; dry_run:true previews the target set with neither. Each repo is bounded by timeout_secs (default 120).",
        },
    ];
    R
}

/// Look up an action by verb.
pub fn find(verb: &str) -> Option<&'static ActionDef> {
    registry().iter().find(|d| d.verb == verb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbs_are_unique_and_findable() {
        let mut seen = std::collections::HashSet::new();
        for def in registry() {
            assert!(seen.insert(def.verb), "duplicate verb {}", def.verb);
            assert!(find(def.verb).is_some());
        }
    }

    #[test]
    fn run_is_the_only_run_tier() {
        let run: Vec<_> = registry()
            .iter()
            .filter(|d| d.tier == Tier::Run)
            .map(|d| d.verb)
            .collect();
        assert_eq!(run, ["run"]);
    }
}
