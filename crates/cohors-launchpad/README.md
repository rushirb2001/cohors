# cohors-launchpad

Part of [cohors](https://crates.io/crates/cohors), the dashboard for every git repo you own: a terminal dashboard, CLI, browser dashboard and MCP server in one binary.

This crate reads and writes the launchpad: what each repo says about itself in its committed `.cohors/repo.json` (flows to run, agent prompts, tasks), your cross-repo focus board and the agent activity log. Writes are validated whole before anything touches disk and never follow a symbolic link. The format is an open spec: [spec/v1](https://github.com/rushirb2001/cohors/blob/main/spec/v1/SPEC.md).

It is published so that `cargo install cohors` can build. Its API follows what the cohors binary needs and can change in any release. To use cohors, install the binary:

```sh
cargo install cohors
```

For the reusable domain model (repo snapshots, the selector language, attention), see [`cohors-core`](https://crates.io/crates/cohors-core).

[Repository](https://github.com/rushirb2001/cohors) · [Changelog](https://github.com/rushirb2001/cohors/blob/main/CHANGELOG.md) · MIT licensed.
