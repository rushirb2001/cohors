#!/usr/bin/env bash
# Build every marketing asset from scratch.
#
#   scripts/studio/build.sh            everything
#   scripts/studio/build.sh posts      only the post images
#   scripts/studio/build.sh film       only the film, the sting and the README loop
#
# It rebuilds the made-up fleet, captures cohors on it, and renders the studio
# pages. The README's pictures go to assets/ (committed); the post images and
# the film go to target/assets/ (large, and reproducible from here: upload
# them where they are shown). Needs Chrome, Node, ffmpeg, gifsicle and ImageMagick.
set -euo pipefail
cd "$(dirname "$0")/../.."
WHAT="${1:-all}"
RENDER="node scripts/studio/render.mjs"

if [ "$WHAT" != film-only ] && [ ! -f target/assets/captures/overview.html ] || [ "${RECAPTURE:-}" = 1 ]; then
  scripts/studio/capture.sh
fi

still() {  # still CARD WxH FILE [SCALE]
  $RENDER --page "assets/studio/post.html?card=$1" --size "$2" --scale "${4:-1}" --still "$3"
}

if [ "$WHAT" = all ] || [ "$WHAT" = posts ]; then
  # The post images are built on demand and not committed (about 10 MB of
  # pictures that this script reproduces exactly).
  S=target/assets/social
  mkdir -p "$S"
  # Where each one goes, at the size that place wants.
  still hero      1280x640  "$S/github-social-preview.png"        # GitHub repository settings
  still hero      1200x630  "$S/link-preview.png"                 # the picture a pasted link shows
  still hero      1270x760  "$S/producthunt-gallery-1.png"
  still next      1270x760  "$S/producthunt-gallery-2.png"
  still push      1270x760  "$S/producthunt-gallery-3.png"
  still tabs      1600x900  "$S/x-tabs.png"                       # X, 16:9
  still next      1600x900  "$S/x-do-next.png"
  still risk      1600x900  "$S/x-work-at-risk.png"
  still risk      1200x627  "$S/linkedin-work-at-risk.png"        # LinkedIn link image
  still tabs      1200x627  "$S/linkedin-tabs.png"
  still push      1080x1080 "$S/square-push.png"                  # square, any feed
  still standup   1080x1080 "$S/square-standup.png"
  still launchpad 1080x1080 "$S/square-launchpad.png"
  still hero      1280x720  "$S/youtube-thumbnail.png"
  # The carousel: seven portrait pages, in order.
  n=1
  for card in q-cover q1 q2 q3 q4 q5 q-end; do
    still "$card" 1080x1350 "$S/carousel-$n.png"; n=$((n + 1))
  done
  # The roll call: every repository by name. A post in three shapes, and a
  # poster large enough to print.
  still roll 1600x900  "$S/x-roll-call.png"
  still roll 1080x1350 "$S/portrait-roll-call.png"
  still roll 1200x1800 "$S/poster-roll-call.png" 2
  # The mark alone: the editor extension's icon and the plugin's.
  still icon 256x256 extension/media/icon.png
  still icon 512x512 "$S/icon-512.png"
  # The README's pictures, at twice the size they are shown.
  cp "$S/github-social-preview.png" assets/social-preview.png
  # The name at the top of the README, for GitHub's dark and light pages.
  mkdir -p assets/readme
  for tone in dark light; do
    $RENDER --page "assets/studio/post.html?card=lockup&tone=$tone" --size 760x200 --scale 2 --transparent 1 --still "target/assets/lockup-$tone.png"
    magick "target/assets/lockup-$tone.png" -trim +repage -bordercolor none -border 12 "assets/readme/cohors-$tone.png"
  done
  still hero 1280x640 assets/hero.png 2
  still window 1480x1066 assets/terminal.png 2
  # The editor extension and the browser dashboard, as they really render.
  # (`from` points the extension's preview at the captures, so it shows the
  # same made-up fleet as the terminal.)
  $RENDER --page "extension/dev/preview.html?bare=1&from=/target/assets/captures/" --size 1440x900 --scale 2 --wait 1500 --still assets/vscode.png
  $RENDER --page "extension/dev/preview.html?side=all&bare=1&from=/target/assets/captures/" --size 360x1180 --scale 2 --wait 1500 --still assets/vscode-sidebar.png
  HOME_DIR="$(cd /tmp/cohors-demo/home && pwd -P)"
  # The fake home is only for what cohors reads; the toolchain is yours.
  REAL_HOME="$HOME"
  HOME="$HOME_DIR" CARGO_HOME="${CARGO_HOME:-$REAL_HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$REAL_HOME/.rustup}" \
    cohors web --no-open --port 8791 > /dev/null 2>&1 &
  WEB=$!
  for _ in $(seq 1 40); do curl -s -o /dev/null http://127.0.0.1:8791/api/overview && break; sleep 0.5; done
  $RENDER --url http://127.0.0.1:8791/ --size 1440x900 --scale 2 --wait 2500 --still assets/web.png
  kill "$WEB" 2>/dev/null || true
fi

if [ "$WHAT" = all ] || [ "$WHAT" = film ]; then
  F=target/assets
  mkdir -p "$F"
  # One film, framed three ways: the page takes the size it is given and
  # frames every shot to fit, so the square and upright cuts are renders of
  # their own, not crops that lose the captions.
  for cut in 1080p:1920x1080 square:1080x1080 upright:1080x1920; do
    name="${cut%%:*}" size="${cut#*:}"
    $RENDER --page assets/studio/film.html --size "$size" --fps 30 --frames "$F/frames/$name"
    ffmpeg -y -loglevel error -framerate 30 -i "$F/frames/$name/%06d.png" \
      -c:v libx264 -preset slow -crf 17 -pix_fmt yuv420p -movflags +faststart "$F/cohors-film-$name.mp4"
  done
  # The sting: the mark assembling and the name, four seconds, for the start
  # of somebody else's video or a post of its own.
  for cut in 1080p:1920x1080 square:1080x1080; do
    name="${cut%%:*}" size="${cut#*:}"
    $RENDER --page "assets/studio/film.html?sting=1" --size "$size" --fps 30 --frames "$F/frames/sting-$name"
    ffmpeg -y -loglevel error -framerate 30 -i "$F/frames/sting-$name/%06d.png" \
      -c:v libx264 -preset slow -crf 17 -pix_fmt yuv420p -movflags +faststart "$F/cohors-sting-$name.mp4"
  done
  # The README loop: the push, from the whole dashboard to the number
  # changing. It is rendered from the page itself, not cut from the video, so
  # the text is drawn sharp at its own size rather than scaled down from a
  # compressed frame. 50 frames a second is the fastest a GIF plays in a
  # browser (a frame shorter than 2/100 s is slowed to 1/10 s). One palette
  # of 256 colours for the whole loop, no dithering (it speckles text) and no
  # lossy pass: about 25 MB.
  $RENDER --page assets/studio/film.html --size 1280x720 --fps 50 --from 7000 --to 15800 --frames "$F/frames/gif"
  ffmpeg -y -loglevel error -framerate 50 -i "$F/frames/gif/%06d.png" \
    -vf "split[a][b];[a]palettegen=max_colors=256:stats_mode=full[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" \
    "$F/demo-raw.gif"
  gifsicle -O3 "$F/demo-raw.gif" -o assets/demo.gif 2>/dev/null
  rm -f "$F/demo-raw.gif"
  # The same stretch as a video, 3840x2160 at 120 frames a second, for places
  # that play video (X, YouTube, a GitHub upload).
  $RENDER --page assets/studio/film.html --size 1920x1080 --scale 2 --fps 120 --from 7000 --to 15800 --frames "$F/frames/clip4k"
  ffmpeg -y -loglevel error -framerate 120 -i "$F/frames/clip4k/%06d.png" \
    -c:v libx264 -preset slow -crf 16 -profile:v high -level:v 5.2 -pix_fmt yuv420p -movflags +faststart "$F/cohors-push-4k120.mp4"
  rm -rf "$F/frames"
  ls -la "$F"/*.mp4 assets/demo.gif | awk '{printf "%.1f MB  %s\n", $5 / 1048576, $NF}'
fi
