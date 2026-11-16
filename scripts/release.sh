#!/bin/sh
# Cut a release in one step, so each finished piece of work ships on its own.
#
#   scripts/release.sh minor            # a big feature: 0.7.7 -> 0.8.0
#   scripts/release.sh patch            # a small feature or a refinement: 0.8.0 -> 0.8.1
#   scripts/release.sh patch --push     # ... and push main and the tag (starts the pipeline)
#   scripts/release.sh patch --dry-run  # say what would happen, change nothing
#
# What it does, in order (ADR-059, "Release cadence"):
#   1. refuses unless the CHANGELOG's [Unreleased] section says what ships;
#   2. bumps the version everywhere (scripts/bump-version.sh, which also
#      checks the bump against the changelog);
#   3. closes [Unreleased] as [X.Y.Z] with today's date;
#   4. commits only the release files as `chore(release): X.Y.Z` and adds the
#      annotated tag vX.Y.Z;
#   5. with --push, pushes main and then the tag, which starts the Release
#      workflow and everything chained to it.
# Other uncommitted work in the tree is left alone and is not part of it.
set -eu
usage="usage: scripts/release.sh <major|minor|patch> [--push] [--dry-run] [--force]"
kind="${1:?$usage}"
shift
push=0
dry=0
force=""
for arg in "$@"; do
  case "$arg" in
    --push) push=1 ;;
    --dry-run) dry=1 ;;
    --force) force="--force" ;;
    *) echo "unknown option: $arg" >&2; echo "$usage" >&2; exit 1 ;;
  esac
done
cd "$(dirname "$0")/.."

unreleased=$(awk '/^## \[Unreleased\]/ { on = 1; next } /^## \[/ { on = 0 } on' CHANGELOG.md | grep -v '^[[:space:]]*$' || true)
if [ -z "$unreleased" ]; then
  echo "nothing to release: the CHANGELOG's [Unreleased] section is empty" >&2
  exit 1
fi
if [ -n "$(git diff --cached --name-only)" ]; then
  echo "something is already staged; commit or unstage it first" >&2
  exit 1
fi

if [ "$dry" -eq 1 ]; then
  sh scripts/bump-version.sh --dry-run $force "$kind"
  echo "would close [Unreleased], commit chore(release), tag$( [ "$push" -eq 1 ] && echo ', and push main and the tag')."
  exit 0
fi

sh scripts/bump-version.sh $force "$kind"
version=$(sed -n 's/^version = "\([0-9][^"]*\)"/\1/p' Cargo.toml | head -n 1)
today=$(date -u +%Y-%m-%d)
# Keep an empty [Unreleased] on top for the next piece of work.
awk -v v="$version" -v d="$today" '
  !done && /^## \[Unreleased\]/ { print; print ""; print "## [" v "] — " d; done = 1; next }
  { print }
' CHANGELOG.md > CHANGELOG.md.tmp && mv CHANGELOG.md.tmp CHANGELOG.md

git add CHANGELOG.md Cargo.toml Cargo.lock server.json extension/package.json \
  extension/dev/fixtures/host.json plugin/.claude-plugin/plugin.json
git commit -q -m "chore(release): $version"
git tag -a "v$version" -m "v$version"
echo "released $version locally: commit $(git rev-parse --short HEAD), tag v$version"

if [ "$push" -eq 1 ]; then
  git push origin main
  git push origin "v$version"
  echo "pushed: the Release workflow is running for v$version"
else
  echo "to publish: git push origin main && git push origin v$version"
fi
