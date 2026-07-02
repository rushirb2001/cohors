//! Structural dependency discipline: front-ends read through the fleet facade.
//!
//! The rule this enforces: **no front-end depends on `cohors-git` or
//! `cohors-github` directly.** Reads funnel through `cohors-fleet` (this crate)
//! and writes through `cohors-actions`, so an adapter change lands in exactly
//! one place instead of flooding into every surface — and a new feature that
//! tries to bypass the funnel fails CI here (its direct import wouldn't even
//! compile without the dep this test forbids declaring). The same mechanism as
//! the action-registry parity test: architecture as a failing test, not a memo.

use std::path::Path;

/// The front-end crates the rule applies to. Add new surfaces here as they
/// appear — forgetting to is caught by `every_front_end_is_listed`.
const FRONT_ENDS: &[&str] = &["cohors-tui", "cohors-mcp", "cohors-web"];

/// Adapters no front-end may name directly.
const FORBIDDEN: &[&str] = &["cohors-git", "cohors-github"];

fn manifest(krate: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .join(krate)
        .join("Cargo.toml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path:?}: {e}"))
}

/// A manifest "names a dependency" when a line starts with that key. Comments
/// and doc strings don't count; a `dep = { workspace = true }` line does.
fn declares_dep(manifest: &str, dep: &str) -> bool {
    manifest.lines().any(|l| {
        let l = l.trim_start();
        !l.starts_with('#')
            && (l.starts_with(&format!("{dep} ")) || l.starts_with(&format!("{dep}=")))
    })
}

#[test]
fn front_ends_never_bypass_the_fleet_facade() {
    for fe in FRONT_ENDS {
        let m = manifest(fe);
        for dep in FORBIDDEN {
            assert!(
                !declares_dep(&m, dep),
                "`{fe}` declares `{dep}` — front-ends must read through cohors-fleet \
                 (and write through cohors-actions), never the raw adapters"
            );
        }
        assert!(
            declares_dep(&m, "cohors-fleet"),
            "`{fe}` doesn't depend on cohors-fleet — every surface reads through the facade"
        );
    }
}

/// Guard the guard: if a new front-end crate appears in the workspace without
/// being added to FRONT_ENDS, fail — the discipline must cover every surface.
#[test]
fn every_front_end_is_listed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let known_non_front_ends = [
        "cohors-core",
        "cohors-config",
        "cohors-git",
        "cohors-github",
        "cohors-actions",
        "cohors-fleet",
    ];
    for entry in std::fs::read_dir(root).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !entry.path().join("Cargo.toml").exists() {
            continue;
        }
        assert!(
            FRONT_ENDS.contains(&name.as_str()) || known_non_front_ends.contains(&name.as_str()),
            "new crate `{name}` — classify it: add to FRONT_ENDS (surface) or \
             known_non_front_ends (library) in cohors-fleet/tests/discipline.rs"
        );
    }
}
