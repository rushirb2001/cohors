# cohors-mcp

Part of [cohors](https://crates.io/crates/cohors), the dashboard for every git repo you own: a terminal dashboard, CLI, browser dashboard and MCP server in one binary.

This crate is the Model Context Protocol server behind `cohors mcp`: your agent gets every repo's state, a search across all of them, recent commits and each repo's tasks. Reads are always on; git actions, running commands and launchpad writes each need their own flag (`--allow-writes`, `--allow-run`, `--allow-launchpad`).

To run it, install the binary and register it with its absolute path:

```sh
cargo install cohors
claude mcp add cohors -- "$HOME/.cargo/bin/cohors" mcp
```

It is published so that `cargo install cohors` can build. Its API follows what the cohors binary needs and can change in any release.

For the reusable domain model (repo snapshots, the selector language, attention), see [`cohors-core`](https://crates.io/crates/cohors-core).

[Repository](https://github.com/rushirb2001/cohors) · [Changelog](https://github.com/rushirb2001/cohors/blob/main/CHANGELOG.md) · MIT licensed.
