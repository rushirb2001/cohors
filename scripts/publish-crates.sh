#!/bin/sh
# Publish every cohors crate to crates.io, libraries first, so
# `cargo install cohors` works. Run by .github/workflows/crates.yml with
# CARGO_REGISTRY_TOKEN set; safe to run again, since a crate whose version is
# already on crates.io is skipped.
#
# The browser dashboard has to be built into crates/cohors-tui/dist first (the
# workflow does that): it ships inside the `cohors` crate, so `cohors web` works
# after a crates.io install.
set -eu
cd "$(dirname "$0")/.."

# Dependency order: each crate's internal dependencies come before it.
CRATES="cohors-core cohors-config cohors-git cohors-github cohors-actions cohors-launchpad cohors-fleet cohors-mcp cohors-web cohors"

version=$(sed -n 's/^version = "\([0-9][^"]*\)"/\1/p' Cargo.toml | head -n 1)
test -f crates/cohors-tui/dist/index.html || {
  echo "crates/cohors-tui/dist has no index.html; build the dashboard first" >&2
  exit 1
}

on_crates_io() {
  # crates.io asks API clients to say who they are.
  test "$(curl -s -o /dev/null -w '%{http_code}' -A 'cohors-release (github.com/rushirb2001/cohors)' \
    "https://crates.io/api/v1/crates/$1/$version")" = 200
}

for crate in $CRATES; do
  if on_crates_io "$crate"; then
    echo "$crate $version is already on crates.io"
    continue
  fi
  # crates.io limits how fast brand-new crates can be created (the first
  # release creates ten). When it says to slow down, wait and try again.
  attempt=1
  while :; do
    log=$(mktemp)
    # Not `cargo publish | tee`: a pipeline's status is its last command's, so
    # a failed publish would look like a success.
    status=0
    cargo publish -p "$crate" --allow-dirty >"$log" 2>&1 || status=$?
    cat "$log"
    if [ "$status" -eq 0 ]; then
      rm -f "$log"
      break
    fi
    if grep -q -i -e '429' -e 'too many' -e 'rate limit' "$log" && [ "$attempt" -lt 12 ]; then
      rm -f "$log"
      echo "crates.io asked to slow down; waiting 10 minutes before publishing $crate again"
      sleep 600
      attempt=$((attempt + 1))
      continue
    fi
    rm -f "$log"
    echo "publishing $crate failed" >&2
    exit 1
  done
done
echo "cohors $version is on crates.io: cargo install cohors"
