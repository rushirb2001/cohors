#!/bin/sh
# Give the work in CHANGELOG.md's [Unreleased] section its own version.
#
#   scripts/version.sh patch    # a fix or refinement: 0.8.0 -> 0.8.1
#   scripts/version.sh minor    # a feature:           0.8.4 -> 0.9.0
#   scripts/version.sh major    # a headline change
#
# The changelog is the version log (ADR-059, "Versions and releases"): every
# fix, refinement and feature gets a section of its own as it lands, so the
# history reads 0.8.1, 0.8.2, 0.8.3 … and `cohors --version` on a dev build
# names exactly where the work stands. This script:
#   1. takes what is under [Unreleased] and puts it under `## [X.Y.Z] — date`,
#      leaving an empty [Unreleased] on top for the next piece;
#   2. bumps the version everywhere it is written (scripts/bump-version.sh).
# Commit the result together with the change it describes. Nothing is tagged
# or published: that is a separate decision, scripts/release.sh.
set -eu
usage="usage: scripts/version.sh <major|minor|patch> [--dry-run]"
kind="${1:?$usage}"
dry=0
[ "${2:-}" = "--dry-run" ] && dry=1
case "$kind" in major|minor|patch) ;; *) echo "$usage" >&2; exit 1 ;; esac
cd "$(dirname "$0")/.."

# A date glued to a word means a find-and-replace of "DATE" went too far
# (it once turned COHORS_NO_UPDATE into COHORS_NO_UP2026-11-16): refuse it.
glued=$(grep -n -E '[A-Za-z_]20[0-9]{2}-[0-9]{2}-[0-9]{2}|20[0-9]{2}-[0-9]{2}-[0-9]{2}[A-Za-z_]' CHANGELOG.md || true)
if [ -n "$glued" ]; then
  echo "CHANGELOG.md has a date run into a word; fix it first:" >&2
  echo "$glued" >&2
  exit 1
fi
entries=$(awk '/^## \[Unreleased\]/ { on = 1; next } /^## \[/ { on = 0 } on' CHANGELOG.md | grep -v '^[[:space:]]*$' || true)
if [ -z "$entries" ]; then
  echo "nothing to version: write what changed under ## [Unreleased] in CHANGELOG.md first" >&2
  exit 1
fi
if [ "$dry" -eq 1 ]; then
  sh scripts/bump-version.sh --dry-run "$kind"
  echo "would move [Unreleased] under the new version's heading."
  exit 0
fi

sh scripts/bump-version.sh "$kind" | tail -n 1 >/dev/null
version=$(sed -n 's/^version = "\([0-9][^"]*\)"/\1/p' Cargo.toml | head -n 1)
today=$(date -u +%Y-%m-%d)
awk -v v="$version" -v d="$today" '
  !done && /^## \[Unreleased\]/ { print; print ""; print "## [" v "] — " d; done = 1; next }
  { print }
' CHANGELOG.md > CHANGELOG.md.tmp && mv CHANGELOG.md.tmp CHANGELOG.md
echo "versioned $version: commit it with the change it describes"
