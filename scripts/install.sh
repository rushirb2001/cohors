#!/usr/bin/env bash
#
# (Re)install the `cohors` binary from the current source — globally (into
# ~/.cargo/bin) and in-repo (into ./.local/bin for testing).
#
# Why --force: `cargo install` SKIPS silently when the crate version is
# unchanged, so an install after a same-version code change is a no-op and you
# keep running the stale binary. --force always overwrites. (We also bump the
# version per change — see CHANGELOG.md / ADR-018 — but --force is the belt.)
#
# Usage:  scripts/install.sh
set -euo pipefail
cd "$(dirname "$0")/.."

cargo install --path crates/cohors-tui --force
cargo install --path crates/cohors-tui --root "$PWD/.local" --force

echo "installed: $(./.local/bin/cohors --version)"
