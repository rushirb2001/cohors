#!/bin/sh
# Set cohors' version everywhere it is written down, for a release commit.
#
#   scripts/bump-version.sh patch          # 0.7.6 -> 0.7.7  fixes and refinements
#   scripts/bump-version.sh minor          # 0.7.6 -> 0.8.0  something new to do
#   scripts/bump-version.sh major          # 0.7.6 -> 1.0.0  a headline change
#   scripts/bump-version.sh 0.8.0          # an explicit version
#   scripts/bump-version.sh --dry-run minor   # say what would happen, change nothing
#
# What each number means is ADR-059 (and the CHANGELOG's "Versioning policy"):
# X a headline change or a contract break, Y anything a user can newly do, Z
# fixes and refinements. The CHANGELOG's [Unreleased] section decides which one
# a release is: a patch bump is refused while it has an "### Added" entry, and
# a bump smaller than a "### Breaking" entry calls for is refused too. Pass
# --force to override on purpose.
#
# The workspace version in Cargo.toml is the source of truth. Since cohors is
# on crates.io, each internal dependency also names that version (crates.io
# needs one beside the path), and the VS Code extension, the Claude Code plugin
# and the MCP Registry manifest each carry it too. Closing the CHANGELOG's
# [Unreleased] section as the new version is left to you.
set -eu
usage="usage: scripts/bump-version.sh [--dry-run] [--force] <major|minor|patch|X.Y.Z>"
dry=0
force=0
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) dry=1 ;;
    --force) force=1 ;;
    -h|--help) echo "$usage"; exit 0 ;;
    -*) echo "unknown option: $1" >&2; echo "$usage" >&2; exit 1 ;;
    *) break ;;
  esac
  shift
done
want="${1:?$usage}"
cd "$(dirname "$0")/.."
old=$(sed -n 's/^version = "\([0-9][^"]*\)"/\1/p' Cargo.toml | head -n 1)
test -n "$old" || { echo "no workspace version in Cargo.toml" >&2; exit 1; }

# The current X.Y.Z (a pre-release suffix such as -rc.1 is dropped).
core=${old%%-*}
x=${core%%.*}
rest=${core#*.}
y=${rest%%.*}
z=${rest#*.}

case "$want" in
  major) new="$((x + 1)).0.0" ;;
  minor) new="$x.$((y + 1)).0" ;;
  patch) new="$x.$y.$((z + 1))" ;;
  [0-9]*.[0-9]*.[0-9]*) new="$want" ;;
  *) echo "not a version or a bump: $want" >&2; echo "$usage" >&2; exit 1 ;;
esac

# Which slot the new version moves, compared with the current one.
ncore=${new%%-*}
nx=${ncore%%.*}
nrest=${ncore#*.}
ny=${nrest%%.*}
nz=${nrest#*.}
if [ "$nx" -gt "$x" ]; then
  kind=major
elif [ "$nx" -eq "$x" ] && [ "$ny" -gt "$y" ]; then
  kind=minor
elif [ "$nx" -eq "$x" ] && [ "$ny" -eq "$y" ] && [ "$nz" -gt "$z" ]; then
  kind=patch
else
  echo "$new is not newer than $old" >&2
  exit 1
fi

# What the release holds, from the CHANGELOG's [Unreleased] section.
unreleased=$(awk '/^## \[Unreleased\]/ { on = 1; next } /^## \[/ { on = 0 } on' CHANGELOG.md)
has() { printf '%s\n' "$unreleased" | grep -q "^### $1"; }
needed=patch
if has Added; then needed=minor; fi
if has Breaking; then
  # Before 1.0 a contract break bumps Y (ADR-018); from 1.0 on it bumps X.
  if [ "$x" -eq 0 ]; then needed=minor; else needed=major; fi
fi
rank() { case "$1" in patch) echo 1 ;; minor) echo 2 ;; major) echo 3 ;; esac; }
if [ "$(rank "$kind")" -lt "$(rank "$needed")" ]; then
  case "$needed" in
    minor) suggest="$x.$((y + 1)).0" ;;
    major) suggest="$((x + 1)).0.0" ;;
  esac
  why="new features (### Added)"
  has Breaking && why="a breaking change (### Breaking)"
  if [ "$force" -eq 1 ]; then
    echo "note: [Unreleased] lists $why, which makes this a $needed release ($suggest); --force keeps $new." >&2
  else
    echo "refusing a $kind bump to $new: the CHANGELOG's [Unreleased] lists $why," >&2
    echo "which makes this a $needed release: $suggest (ADR-059)." >&2
    echo "Run 'scripts/bump-version.sh $needed', or pass --force to keep $new on purpose." >&2
    exit 1
  fi
fi

if [ "$dry" -eq 1 ]; then
  echo "cohors $old -> $new ($kind release). Dry run: nothing changed."
  exit 0
fi

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
echo "cohors $old -> $new ($kind release). Now close the CHANGELOG's [Unreleased] section as [$new]."
