#!/bin/sh
# Set cohors' version everywhere it is written down, for a release commit.
#
#   scripts/bump-version.sh 0.7.3
#
# The workspace version in Cargo.toml is the source of truth. Since cohors is
# on crates.io, each internal dependency also names that version (crates.io
# needs one beside the path), and the VS Code extension, the Claude Code plugin
# and the MCP Registry manifest each carry it too. CHANGELOG.md is left to you.
set -eu
new="${1:?usage: scripts/bump-version.sh <version>, e.g. 0.7.3}"
case "$new" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "not a version: $new" >&2; exit 1 ;;
esac
cd "$(dirname "$0")/.."
old=$(sed -n 's/^version = "\([0-9][^"]*\)"/\1/p' Cargo.toml | head -n 1)
test -n "$old" || { echo "no workspace version in Cargo.toml" >&2; exit 1; }

# Cargo.toml: the workspace version, and every `cohors-* = { path = …, version = … }`.
sed -i.bak \
  -e "s/^version = \"$old\"/version = \"$new\"/" \
  -e "/^cohors-[a-z]* = { path = /s/version = \"$old\"/version = \"$new\"/" \
  Cargo.toml
for file in extension/package.json plugin/.claude-plugin/plugin.json server.json; do
  sed -i.bak "s/\"version\": \"$old\"/\"version\": \"$new\"/g" "$file"
done
sed -i.bak "s/\"cohors $old\"/\"cohors $new\"/" extension/dev/fixtures/host.json
rm -f Cargo.toml.bak extension/package.json.bak plugin/.claude-plugin/plugin.json.bak server.json.bak extension/dev/fixtures/host.json.bak
cargo update --workspace --quiet

left=$(git grep -l -F "\"$old\"" -- Cargo.toml extension/package.json plugin/.claude-plugin/plugin.json server.json || true)
if [ -n "$left" ]; then
  echo "still at $old in: $left" >&2
  exit 1
fi
echo "cohors $old -> $new. Now close the CHANGELOG's [Unreleased] section as [$new]."
