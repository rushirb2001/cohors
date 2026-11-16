# cohors-core

The pure, I/O-free domain logic of [cohors](https://crates.io/crates/cohors), the dashboard for every git repo you own. The terminal dashboard, the CLI, the browser dashboard (compiled to WebAssembly), the MCP server and the VS Code extension all draw from this one crate, so they always agree.

```toml
[dependencies]
cohors-core = "=0.7.7"
```

## What's in it

| Module | |
|---|---|
| `model` | `RepoSnapshot` and friends: branch, upstream ahead/behind, worktree counts, stashes, last commit, CI, pull requests |
| `provider` | The `RepoProvider` trait a data source implements (`list`, `snapshot`, `scan_fleet`) |
| `select` | The selector language (`dirty`, `behind`, `name:pay*`, `branch:feat/*`, `!clean`, …): `parse_shorthand` and `resolve` |
| `attention` | Why a repo needs you and how badly (`assess`, `fleet_summary`) |
| `overview` | The briefing every front-end draws: what to do next, and why |
| `sort`, `fuzzy`, `view` | Ordering, fuzzy filtering and the rows a list view shows |
| `launchpad` | Each repo's committed `.cohors/repo.json` (flows, agents, tasks): the model and its validation |
| `standup`, `search`, `update`, `time`, `demo` | Commit digests, metadata search, release-version logic, clock-free relative time, a sample fleet |

## Example

```rust
use cohors_core::{SortMode, demo, fleet_summary, parse_shorthand, resolve};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The core never reads the clock or the disk: you pass in the time and the
    // snapshots (here, the built-in sample fleet).
    let now = 1_700_000_000;
    let fleet = demo::fleet(now);

    // The same selector language as `cohors scan --select`.
    let selector = parse_shorthand("dirty,!name:scratch*")?;
    for id in resolve(&fleet, &selector, SortMode::DirtyFirst, now) {
        println!("{}", id.0);
    }

    let summary = fleet_summary(&fleet, now);
    println!("{} of {} need attention", summary.needs_attention, summary.total);
    Ok(())
}
```

Real snapshots come from a `RepoProvider`; [`cohors-git`](https://crates.io/crates/cohors-git) is the one cohors uses on disk.

## WebAssembly-safe

No `std::fs`, `std::process`, `std::net`, threads or `std::time::Instant`, and `unsafe` is forbidden. cohors' CI builds the crate for `wasm32-unknown-unknown` on every change, which is how the browser dashboard runs the same logic.

## Versioning

cohors-core is released in lockstep with the `cohors` binary. cohors' compatibility promise covers its command line, the `scan` JSON and the config file, not this crate's Rust API, which can change in any release. Pin an exact version, as above.

## New in 0.7.7

- Selectors: `branch:` takes globs (`branch:feat/*`), and the error for an unknown token lists the whole vocabulary.

[Changelog](https://github.com/rushirb2001/cohors/blob/main/CHANGELOG.md) · [Repository](https://github.com/rushirb2001/cohors) · MIT licensed.
