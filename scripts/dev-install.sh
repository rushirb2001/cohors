#!/usr/bin/env bash
# Fast local reinstall of the `cohors` binary, for day-to-day iteration.
#
# The release profile uses LTO + codegen-units=1 — great for the *shipped* binary
# (smaller/faster), but it makes `cargo install --path crates/cohors-tui` take
# ~1-2 min because of the whole-program link. For local iteration that's overkill:
# this builds the *dev* profile (incremental, no LTO — a couple of seconds) and
# drops it straight into ~/.cargo/bin, so rebuild+reinstall is near-instant.
#
# Use the optimized binary (`cargo install --path crates/cohors-tui`, release)
# only when you actually want the shipped artifact; CI/releases build that anyway.
#
# Usage: scripts/dev-install.sh        # fast dev reinstall
#        scripts/dev-install.sh --release   # pass through extra cargo flags
set -euo pipefail
cd "$(dirname "$0")/.."

profile_dir="debug"
case " $* " in *" --release "*) profile_dir="release" ;; esac

cargo build -p cohors-tui "$@"
dest="${CARGO_HOME:-$HOME/.cargo}/bin/cohors"
install -m 0755 "target/${profile_dir}/cohors" "$dest"
echo "✓ cohors (${profile_dir}) → ${dest}"
