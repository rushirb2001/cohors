# cohors-git

Part of [cohors](https://crates.io/crates/cohors), the dashboard for every git repo you own: a terminal dashboard, CLI, browser dashboard and MCP server in one binary.

This crate finds the repositories under the configured folders (bare repos and repos nested inside other repos included, build and dependency folders skipped) and takes a snapshot of each: branch, upstream ahead/behind, uncommitted work, stashes and recent history. It reads with [gix](https://crates.io/crates/gix), with git2 as the fallback, and snapshots repos in parallel. It implements `cohors_core::RepoProvider`.

It is published so that `cargo install cohors` can build. Its API follows what the cohors binary needs and can change in any release. To use cohors, install the binary:

```sh
cargo install cohors
```

For the reusable domain model (repo snapshots, the selector language, attention), see [`cohors-core`](https://crates.io/crates/cohors-core).

[Repository](https://github.com/rushirb2001/cohors) · [Changelog](https://github.com/rushirb2001/cohors/blob/main/CHANGELOG.md) · MIT licensed.
