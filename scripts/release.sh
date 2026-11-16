#!/bin/sh
# Publish the current version: a public, tagged release, made when the work
# has reached a version stable enough to ship.
#
#   scripts/release.sh            # check, then tag vX.Y.Z locally
#   scripts/release.sh --push     # ... and push main and the tag (starts the pipeline)
#   scripts/release.sh --dry-run  # say what would happen, change nothing
#
# Versions are cut as the work lands (scripts/version.sh: every fix,
# refinement and feature gets its own CHANGELOG section and version). A
# release does not bump anything: it tags the version the tree is already at.
# The GitHub release notes then carry every version section since the
# previous tag (scripts/release-notes.py, run by the release-notes workflow),
# so one public release can cover 0.8.1 … 0.8.6. See ADR-059.
set -eu
push=0
dry=0
for arg in "$@"; do
  case "$arg" in
    --push) push=1 ;;
    --dry-run) dry=1 ;;
    *) echo "usage: scripts/release.sh [--push] [--dry-run]" >&2; exit 1 ;;
  esac
done
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\([0-9][^"]*\)"/\1/p' Cargo.toml | head -n 1)
tag="v$version"

# A date glued to a word means a find-and-replace of "DATE" went too far
# (it once turned COHORS_NO_UPDATE into COHORS_NO_UP2026-11-16): refuse it.
glued=$(grep -n -E '[A-Za-z_]20[0-9]{2}-[0-9]{2}-[0-9]{2}|20[0-9]{2}-[0-9]{2}-[0-9]{2}[A-Za-z_]' CHANGELOG.md || true)
if [ -n "$glued" ]; then
  echo "CHANGELOG.md has a date run into a word; fix it first:" >&2
  echo "$glued" >&2
  exit 1
fi
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "$tag already exists: version the next change first (scripts/version.sh)" >&2
  exit 1
fi
pending=$(awk '/^## \[Unreleased\]/ { on = 1; next } /^## \[/ { on = 0 } on' CHANGELOG.md | grep -v '^[[:space:]]*$' || true)
if [ -n "$pending" ]; then
  echo "[Unreleased] still has entries: give them a version first (scripts/version.sh patch|minor)" >&2
  exit 1
fi
if ! grep -q "^## \[$version\]" CHANGELOG.md; then
  echo "CHANGELOG.md has no [$version] section" >&2
  exit 1
fi
if [ -n "$(git status --porcelain -- Cargo.toml Cargo.lock CHANGELOG.md server.json extension/package.json plugin/.claude-plugin/plugin.json)" ]; then
  echo "the version files or CHANGELOG.md have uncommitted edits: commit them first" >&2
  exit 1
fi
previous=$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || echo "")
covers=$(awk -v prev="${previous#v}" '
  /^## \[/ { v = $2; gsub(/[\[\]]/, "", v); if (v == "Unreleased") next; if (v == prev) exit; print v }
' CHANGELOG.md | tr '\n' ' ')

echo "publishing $version (covers: ${covers:-$version}; previous release: ${previous:-none})"
if [ "$dry" -eq 1 ]; then
  echo "dry run: nothing tagged or pushed"
  exit 0
fi
git tag -a "$tag" -m "$tag"
echo "tagged $tag at $(git rev-parse --short HEAD)"
if [ "$push" -eq 1 ]; then
  git push origin main
  git push origin "$tag"
  echo "pushed: the Release workflow is running for $tag"
else
  echo "to publish: git push origin main && git push origin $tag"
fi
