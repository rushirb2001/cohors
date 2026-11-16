# cohors-actions

Part of [cohors](https://crates.io/crates/cohors), the dashboard for every git repo you own: a terminal dashboard, CLI, browser dashboard and MCP server in one binary.

This crate is everything cohors changes: the git actions (fetch, pull, push, commit, stash), running one command in many repos, and the registry every front-end (CLI, dashboard, browser, MCP) dispatches through, on a shared bounded pool. Pull is fast-forward only, push never forces, and a commit that would include a file that looks like a secret is refused.

It is published so that `cargo install cohors` can build. Its API follows what the cohors binary needs and can change in any release. To use cohors, install the binary:

```sh
cargo install cohors
```

For the reusable domain model (repo snapshots, the selector language, attention), see [`cohors-core`](https://crates.io/crates/cohors-core).

[Repository](https://github.com/rushirb2001/cohors) · [Changelog](https://github.com/rushirb2001/cohors/blob/main/CHANGELOG.md) · MIT licensed.
