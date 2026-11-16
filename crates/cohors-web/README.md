# cohors-web

Part of [cohors](https://crates.io/crates/cohors), the dashboard for every git repo you own: a terminal dashboard, CLI, browser dashboard and MCP server in one binary.

This crate is the dashboard `cohors web` opens in your browser. Its native half serves the page on localhost and proxies GitHub so the browser never sees your token; its WebAssembly half (Leptos) draws the same overview the terminal does, from `cohors-core`. The browser dashboard starts read-only.

It is published so that `cargo install cohors` can build. Its API follows what the cohors binary needs and can change in any release. To use cohors, install the binary:

```sh
cargo install cohors
```

For the reusable domain model (repo snapshots, the selector language, attention), see [`cohors-core`](https://crates.io/crates/cohors-core).

[Repository](https://github.com/rushirb2001/cohors) · [Changelog](https://github.com/rushirb2001/cohors/blob/main/CHANGELOG.md) · MIT licensed.
