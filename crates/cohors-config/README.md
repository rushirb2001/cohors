# cohors-config

Part of [cohors](https://crates.io/crates/cohors), the dashboard for every git repo you own: a terminal dashboard, CLI, browser dashboard and MCP server in one binary.

This crate reads and writes cohors' config file: the folders to scan, what to ignore, how deep to look, the editor to open repos in, the update setting, icons and theme. It finds the file in the platform's usual place (or `--config`, or `COHORS_CONFIG`), applies the `COHORS_*` environment overrides, and reports a broken file with its line and column and an unknown key with a suggestion.

It is published so that `cargo install cohors` can build. Its API follows what the cohors binary needs and can change in any release. To use cohors, install the binary:

```sh
cargo install cohors
```

For the reusable domain model (repo snapshots, the selector language, attention), see [`cohors-core`](https://crates.io/crates/cohors-core).

[Repository](https://github.com/rushirb2001/cohors) · [Changelog](https://github.com/rushirb2001/cohors/blob/main/CHANGELOG.md) · MIT licensed.
