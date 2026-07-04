#!/usr/bin/env bash
# Launch cohors *inside* Rio with the Glyph Protocol tier forced on, so you can see
# cohors's own vector icons (CI check/cross, branch/stash/PR badges) instead of the
# text/Unicode fallbacks. Rio is the only terminal that implements the protocol
# today; `scripts/dev-install.sh` installs it on macOS.
#
# Usage: scripts/cohors-rio.sh [ROOT]
#   ROOT defaults to ~/Main/code/git-commits.
set -euo pipefail

root="${1:-$HOME/Main/code/git-commits}"

if ! command -v rio >/dev/null 2>&1; then
  echo "Rio isn't installed. Run scripts/dev-install.sh (macOS) or 'brew install --cask rio'." >&2
  exit 1
fi

# `rio -e <cmd…>` runs a command in a fresh Rio window; COHORS_GLYPH_PROBE forces
# the capability probe even though we'd auto-detect Rio via TERM_PROGRAM anyway.
exec rio -e env COHORS_GLYPH_PROBE=1 cohors --root "$root" --watch
