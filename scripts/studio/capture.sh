#!/usr/bin/env bash
# Capture what cohors really draws, on the made-up fleet, for the studio pages.
#
# Everything a post image or the film shows is a capture taken here: the real
# binary, run in a real terminal of a fixed size against the fleet that
# scripts/demo-fleet.sh builds, with real keys pressed. Nothing is mocked up
# and nothing comes from anybody's own repositories. The captures are HTML
# (text with its colours), so the studio pages can zoom into them and stay
# sharp.
#
#   scripts/studio/capture.sh        writes target/assets/captures/*.html
set -euo pipefail
cd "$(dirname "$0")/../.."

DEMO=/tmp/cohors-demo
OUT=target/assets/captures
BIN="${COHORS_BIN:-$(command -v cohors)}"
SIZE=150x50

./scripts/demo-fleet.sh "$DEMO" > /dev/null
HOME_DIR="$(cd "$DEMO/home" && pwd -P)"
rm -rf "$OUT" && mkdir -p "$OUT"

shoot() {  # shoot NAME "keys"
  HOME="$HOME_DIR" python3 scripts/term-shot.py --size "$SIZE" --out "$OUT" --name "$1" --keys "$2" -- "$BIN" > /dev/null 2>&1
}

# The dashboard, card by card, then the views that open from it.
shoot tour "wait:3500 shot:overview \
  2 wait:250 down wait:200 shot:risk \
  5 wait:250 down down down wait:200 shot:repos \
  enter wait:600 shot:detail esc \
  s wait:1800 shot:standup q \
  t wait:1000 shot:launchpad esc \
  ? wait:300 shot:help esc \
  : text:so tab text:r tab wait:250 shot:command esc q"

# The fleet as data before anything is pushed, for the cards that list every
# repository as it was found.
HOME="$HOME_DIR" "$BIN" overview --json > "$OUT/overview-before.json" 2>/dev/null || echo '{}' > "$OUT/overview-before.json"

# A real action: push everything that exists only on this machine, and the
# same screen afterwards. (The fleet is rebuilt on the next run.)
shoot act "wait:3500 shot:before-push P wait:3500 shot:after-push q"
rm -f "$OUT"/tour.* "$OUT"/act.*

# The same fleet as data, for the editor extension's preview.
for part in "overview --json" "launchpad --json" "standup --json" "standup --today --json"; do
  name="$(echo "$part" | tr -d ' -' | sed 's/json$//')"
  # shellcheck disable=SC2086
  HOME="$HOME_DIR" "$BIN" $part > "$OUT/$name.json" 2>/dev/null || echo '{}' > "$OUT/$name.json"
done

ls "$OUT" | tr '\n' ' '; echo
