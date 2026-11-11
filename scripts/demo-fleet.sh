#!/usr/bin/env bash
# Build a made-up fleet of real git repositories, for screenshots, recordings
# and demos, in a home folder of its own.
#
# Why not `cohors demo`? That mode is sample data with stubbed actions: it
# says "(demo)" where the roots go and nothing it shows is real. This is the
# opposite: twelve real repositories under a fake home's ~/code, each with a
# local remote, in the states a working developer's repos get into (unpushed
# commits, a branch never published, stashes, a diverged branch, repos behind
# their remote). cohors reads them like any others, and fetch, pull and push
# genuinely work, so nothing on camera is staged. None of it touches, reads
# or names anything in your real home folder.
#
#   scripts/demo-fleet.sh [DIR]        (default: /tmp/cohors-demo)
#
# Then run anything against it with the fake home:
#
#   HOME=/tmp/cohors-demo/home cohors
#
# Re-running rebuilds it from scratch, so every take starts identical.

set -euo pipefail

DEMO="${1:-/tmp/cohors-demo}"
case "$DEMO" in
  /tmp/*|/private/tmp/*|"${TMPDIR:-/nonexistent}"*) ;;
  *) echo "refusing to build a demo fleet outside a temp folder: $DEMO" >&2; exit 1 ;;
esac

rm -rf "$DEMO"
mkdir -p "$DEMO/home/code" "$DEMO/remotes" "$DEMO/home/.config/cohors" "$DEMO/home/.cohors"
# The real path (on macOS /tmp is a link to /private/tmp), so the paths git
# reports sit under this HOME and cohors can show them as ~/code/...
DEMO="$(cd "$DEMO" && pwd -P)"
export HOME="$DEMO/home"
export GIT_CONFIG_NOSYSTEM=1
CODE="$HOME/code"
NOW=$(date +%s)

git config --global user.name "Sam Rivera"
git config --global user.email "sam@example.com"
git config --global init.defaultBranch main
git config --global advice.detachedHead false

# A commit dated `$1` days ago in the current repo, touching one file so the
# history has real content.
commit_ago() {
  local days="$1" message="$2" file="${3:-notes.md}"
  local when=$((NOW - days * 86400 - RANDOM % 30000))
  echo "$message" >> "$file"
  git add -A
  GIT_AUTHOR_DATE="@$when" GIT_COMMITTER_DATE="@$when" git commit -q -m "$message"
}

# A new repository with a few months of history, and (unless told otherwise)
# a local remote it is level with.
#   new_repo NAME BRANCH "day:message" ...
new_repo() {
  local name="$1" branch="$2"; shift 2
  mkdir -p "$CODE/$name" && cd "$CODE/$name"
  git init -q -b main
  printf '# %s\n' "$name" > README.md
  local entry
  for entry in "$@"; do commit_ago "${entry%%:*}" "${entry#*:}"; done
  git clone -q --bare . "$DEMO/remotes/$name.git"
  git remote add origin "$DEMO/remotes/$name.git"
  git fetch -q origin
  git branch -q --set-upstream-to=origin/main main
  if [ "$branch" != main ]; then git switch -q -c "$branch"; fi
}

# Move the remote ahead by N commits, as a teammate pushing would.
remote_ahead() {
  local name="$1" branch="$2" count="$3" work="$DEMO/tmp-clone"
  rm -rf "$work"
  git clone -q "$DEMO/remotes/$name.git" "$work"
  ( cd "$work"
    git switch -q "$branch" 2>/dev/null || git switch -q -c "$branch"
    for i in $(seq 1 "$count"); do
      echo "teammate change $i" >> teammate.md; git add -A
      git -c user.name="Priya Nair" -c user.email="priya@example.com" commit -q -m "fix: review feedback, part $i"
    done
    git push -q origin "$branch" )
  rm -rf "$work"
  ( cd "$CODE/$name" && git fetch -q origin )
}

# The launchpad file a repository commits about itself.
launchpad() {
  mkdir -p .cohors
  cat > .cohors/repo.json
  git add -A && git commit -q -m "chore: describe this repo for the launchpad"
}

# ── payments: two commits not pushed, three files changed, tasks ───────────
new_repo payments main "80:feat: charge intents" "66:feat: refunds" "52:fix: currency rounding" \
  "41:feat: webhook signatures" "33:test: replay protection" "24:fix: duplicate charge on retry" \
  "17:feat: idempotency keys" "9:refactor: split the processor client" "6:fix: retry charge on 5xx from the processor"
launchpad <<'JSON'
{
  "specVersion": 1,
  "name": "payments",
  "summary": "Charges, refunds and webhooks for every product",
  "group": "backend",
  "flows": [
    { "id": "dev", "label": "Run locally", "kind": "server", "run": "docker compose up -d && cargo run", "primary": true },
    { "id": "test", "label": "Test", "kind": "test", "run": "cargo test" }
  ],
  "agents": [],
  "tasks": [
    { "id": "idempotency-keys", "title": "Idempotency keys on charge retries", "status": "doing", "priority": "high" },
    { "id": "webhook-backoff", "title": "Back off webhook redelivery", "status": "todo", "priority": "normal" }
  ]
}
JSON
git push -q origin main
commit_ago 1 "feat: idempotency keys on retries" src_retry.rs
commit_ago 0 "fix: back off when the processor rate-limits" src_retry.rs
echo "wip" >> src_retry.rs; echo "wip" >> README.md; echo "notes" > scratch.md

# ── web-app: on a feature branch, five behind, work in progress ────────────
new_repo web-app main "84:feat: app shell" "70:feat: product pages" "58:feat: search" "44:fix: hydration warning" \
  "30:feat: cart" "21:perf: lazy-load the gallery" "12:feat: checkout form" "5:fix: tax line on mobile"
launchpad <<'JSON'
{
  "specVersion": 1,
  "name": "web-app",
  "summary": "The storefront customers use",
  "group": "frontend",
  "flows": [ { "id": "dev", "label": "Dev server", "kind": "server", "run": "pnpm dev", "port": 3000, "primary": true } ],
  "agents": [],
  "tasks": [
    { "id": "cart-drawer", "title": "Cart drawer animation", "status": "todo", "priority": "high" },
    { "id": "a11y-checkout", "title": "Keyboard order in the checkout form", "status": "blocked", "priority": "normal" }
  ]
}
JSON
git push -q origin main
git switch -q -c feat/checkout
commit_ago 2 "wip: cart drawer animation" cart.tsx
git push -q -u origin feat/checkout
remote_ahead web-app feat/checkout 5
for f in cart.tsx drawer.tsx checkout.tsx tax.ts styles.css totals.ts; do echo "wip" >> "$f"; done
git add cart.tsx drawer.tsx 2>/dev/null || true; git reset -q
echo "draft" > NOTES.md

# ── mobile-app: diverged (four ahead, one behind), staged work ─────────────
new_repo mobile-app main "76:feat: onboarding" "61:feat: push notifications" "47:fix: crash on cold start" \
  "35:feat: offline cart" "22:feat: face unlock" "8:chore: bump the SDK"
git switch -q -c release/2.0 && git push -q -u origin release/2.0
remote_ahead mobile-app release/2.0 1
for m in "feat: biometric unlock" "fix: token refresh race" "test: unlock flows" "chore: release notes"; do commit_ago 1 "$m" unlock.kt; done
echo "x" >> unlock.kt; echo "y" > biometrics.kt; echo "z" > keystore.kt; git add unlock.kt biometrics.kt keystore.kt
echo "more" >> README.md

# ── design-system: one commit not pushed, one stash ────────────────────────
new_repo design-system main "88:feat: tokens" "63:feat: button" "50:feat: dialog" "37:feat: data table" \
  "26:fix: focus ring contrast" "14:feat: dark-mode tokens"
echo "experiment" >> tokens.css && git stash -q
commit_ago 0 "feat: density tokens" tokens.css

# ── analytics: an old unpushed commit and two stashes ──────────────────────
new_repo analytics main "82:feat: event schema" "69:feat: funnels" "55:feat: retention" "42:perf: columnar rollups" \
  "28:fix: timezone bucketing" "15:perf: precompute funnel rollups"
echo "a" >> rollup.sql && git stash -q
echo "b" >> rollup.sql && git stash -q
commit_ago 4 "perf: incremental funnel rollups" rollup.sql

# ── data-pipeline: three behind, one task ──────────────────────────────────
new_repo data-pipeline main "79:feat: ingest" "64:feat: backfill" "49:fix: late events" "31:feat: partition pruning" \
  "19:fix: backfill job ordering" "7:chore: tune batch size"
launchpad <<'JSON'
{
  "specVersion": 1,
  "name": "data-pipeline",
  "summary": "Ingest, backfill and rollups",
  "group": "backend",
  "flows": [],
  "agents": [],
  "tasks": [ { "id": "late-events", "title": "Reprocess late events without a full backfill", "status": "todo", "priority": "high" } ]
}
JSON
git push -q origin main
remote_ahead data-pipeline main 3

# ── docs-site: a branch that was never published ───────────────────────────
new_repo docs-site main "74:feat: site" "57:docs: getting started" "39:docs: API reference" "20:feat: versioned docs"
git switch -q -c feat/search
commit_ago 3 "feat: search index" search.ts
commit_ago 1 "feat: search box" search.ts

# ── four that need nothing, or only a look ─────────────────────────────────
new_repo auth-service main "86:feat: sessions" "71:feat: passkeys" "53:fix: clock skew" "34:feat: device list" \
  "18:chore: bump jsonwebtoken" "10:fix: logout everywhere" "3:test: passkey recovery"
new_repo marketing-site main "90:feat: site" "75:content: pricing" "60:content: customers" "45:content: Q2 launch"
touch -t "$(date -v-40d +%Y%m%d%H%M)" "$CODE/marketing-site/.git/FETCH_HEAD" 2>/dev/null || true
new_repo cli-tools main "73:feat: deploy" "56:feat: logs" "38:feat: completions" "13:docs: add shell-completion install notes"
echo "todo" > IDEAS.md
new_repo notifications main "68:feat: email" "51:feat: push" "36:feat: digests" "23:fix: unsubscribe link" "4:feat: quiet hours"

# ── two with no remote at all ──────────────────────────────────────────────
mkdir -p "$CODE/experiments" && cd "$CODE/experiments" && git init -q -b main
commit_ago 65 "spike: try wgpu for the charts"; commit_ago 40 "spike: tile cache"
for f in a.rs b.rs c.rs d.rs; do echo "x" > "$f"; done
mkdir -p "$CODE/infra" && cd "$CODE/infra" && git init -q -b main
commit_ago 59 "feat: base network"; commit_ago 27 "build: pin terraform"; commit_ago 11 "feat: staging cluster"
git checkout -q --detach HEAD
echo "plan" >> main.tf; echo "vars" > staging.tfvars

# ── the user's own files: config, and the focus board ──────────────────────
cat > "$HOME/.config/cohors/config.toml" <<'TOML'
roots = ["~/code"]
theme = "dark"
TOML
cat > "$HOME/.cohors/board.json" <<JSON
{
  "specVersion": 1,
  "updated": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "focus": [
    { "id": "idempotency-keys", "title": "Idempotency keys on charge retries", "repo": "payments", "status": "doing" },
    { "id": "cart-drawer", "title": "Cart drawer animation", "repo": "web-app", "status": "todo" },
    { "id": "late-events", "title": "Reprocess late events without a full backfill", "repo": "data-pipeline", "status": "todo" }
  ]
}
JSON

echo "demo fleet ready: $DEMO"
echo "  HOME=$HOME cohors"
