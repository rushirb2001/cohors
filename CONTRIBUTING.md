# Contributing to grove

grove is open source (MIT) and built in public. Issues, ideas, and PRs are welcome.

## Before you start

- Read [docs/VISION.md](docs/VISION.md) for what grove is (and isn't), and [docs/ROADMAP.md](docs/ROADMAP.md) for what's being built now.
- For anything non-trivial, open an issue first so we can agree on the approach before you write code.

## Dev setup

```sh
git clone <repo-url> grove && cd grove
rustup toolchain install stable        # or whatever rust-toolchain.toml pins
cargo build
cargo run -p grove-tui                 # launch the dashboard
cargo test                             # run the suite
```

## Quality gates (all must pass)

```sh
cargo fmt --all                        # formatting
cargo clippy --all-targets -- -D warnings   # lints (warnings are errors)
cargo test --all                       # unit + integration + snapshots
cargo build -p grove-core --target wasm32-unknown-unknown   # core must stay WASM-safe
```

CI runs all of these; a PR is only mergeable when they're green.

## Conventions

- **Errors:** libraries use `thiserror`; the binary uses `anyhow` at the edges. No `unwrap()`/`panic!` in library code paths that handle user data.
- **Core stays pure:** no `std::fs`, `std::process`, `std::net`, threads, or `std::time::Instant` in `grove-core` — it must compile to WASM.
- **Tests with changes:** new logic in `grove-core` needs unit tests; new TUI states should get an `insta` snapshot.
- **Commits:** [Conventional Commits](https://www.conventionalcommits.org/), **single subject line, no body** (e.g. `feat(tui): add dirty-only filter`). No AI-attribution trailers.
- **Docs:** if you change behavior, update the relevant file in `docs/` and tick the box in `ROADMAP.md`.

## Good first issues

Look for the `good first issue` label. Discovery edge-cases, new sort modes, and TUI polish are friendly entry points; the git/WASM internals are deeper water.
