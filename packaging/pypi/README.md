# cohors

A fast dashboard for **all your git repos** — one terminal view of every
repository on your machine, showing which ones need you and why (unpushed
branches, uncommitted changes, stale stashes, failing CI, open PRs), plus web
and MCP front-ends that render the same scan.

```sh
pip install cohors
cohors init     # auto-detect where your repos live
cohors          # launch the dashboard
```

`pip install cohors` fetches a prebuilt binary for your platform (macOS Apple
Silicon / Intel, Linux x86_64) — no Rust toolchain required. On other platforms,
install from source with `cargo install --git https://github.com/rushirb2001/cohors cohors-tui`.

Full docs, the web dashboard, and the MCP server: **https://github.com/rushirb2001/cohors**

MIT licensed.
