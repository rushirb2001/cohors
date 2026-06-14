#!/usr/bin/env bash
#
# Generate a set of throwaway *sample* git repos that exercise every dashboard
# state, for demos (the vhs tape) and manual testing.
#
# Why this exists: demos must NEVER point cohors at your real repositories — that
# would bake personal data (repo names, commit messages) into a committed GIF.
# This builds fake, reproducible data instead. See docs/DECISIONS.md (ADR-015).
#
# Dates are relative to *now*, so "aging unpushed" (> 2 days) and "stale stash"
# (> 1 week) are always correct whenever you render.
#
# Usage:  scripts/gen-demo-repos.sh [TARGET_DIR]      (default: /tmp/cohors-demo)
# Then:   cohors --root TARGET_DIR
set -euo pipefail

TARGET="${1:-/tmp/cohors-demo}"
# Bare "remotes" and helper clones live outside TARGET so cohors doesn't list them.
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/cohors-demo-scratch.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT

now=$(date +%s)
DAY=86400
recent=$((now - 2 * 3600)) # 2 hours ago
aging=$((now - 5 * DAY))   # 5 days ago  → "aging" unpushed work
stale=$((now - 14 * DAY))  # 2 weeks ago → "stale" stash

# Commit author. The standup view (Tab) lists commits by the *viewer's*
# `git config user.email`, so by default we author as the running user — that
# way the standup actually populates in a demo. Override both for a reproducible
# release GIF; fall back to a fake identity when no git user is configured.
AUTHOR_NAME="${COHORS_DEMO_AUTHOR_NAME:-$(git config user.name 2>/dev/null || true)}"
AUTHOR_EMAIL="${COHORS_DEMO_AUTHOR_EMAIL:-$(git config user.email 2>/dev/null || true)}"
AUTHOR_NAME="${AUTHOR_NAME:-Ada Lovelace}"
AUTHOR_EMAIL="${AUTHOR_EMAIL:-ada@demo.invalid}"

rm -rf "$TARGET"
mkdir -p "$TARGET"

# git with the resolved identity, no global/system config, and an explicit date.
gi() {
  local dir="$1" date="$2"
  shift 2
  GIT_AUTHOR_NAME="$AUTHOR_NAME" GIT_AUTHOR_EMAIL="$AUTHOR_EMAIL" \
    GIT_COMMITTER_NAME="$AUTHOR_NAME" GIT_COMMITTER_EMAIL="$AUTHOR_EMAIL" \
    GIT_AUTHOR_DATE="@$date +0000" GIT_COMMITTER_DATE="@$date +0000" \
    GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null \
    git -C "$dir" "$@"
}

newrepo() {
  mkdir -p "$TARGET/$1"
  gi "$TARGET/$1" "$now" -c init.defaultBranch=main init -q
}

# commit_in <dir> <date> <message> — appends the message to history.log so every
# commit is a real change (no "nothing to commit").
commit_in() {
  local dir="$1" date="$2" msg="$3"
  printf '%s\n' "$msg" >>"$dir/history.log"
  gi "$dir" "$date" add -A
  gi "$dir" "$date" commit -q -m "$msg"
}

mkbare() {
  local p="$SCRATCH/$1.git"
  git -c init.defaultBranch=main init -q --bare "$p" >/dev/null
  printf '%s' "$p"
}

# 1. payments-api — unpushed AND aging (last commit 5 days old, not pushed).
newrepo payments-api
commit_in "$TARGET/payments-api" "$aging" "feat: idempotent charge endpoint"
remote=$(mkbare payments-api)
gi "$TARGET/payments-api" "$aging" remote add origin "$remote"
gi "$TARGET/payments-api" "$aging" push -q -u origin main
commit_in "$TARGET/payments-api" "$aging" "fix: retry on 5xx"

# 2. web-app — behind the remote by 3 (needs a pull).
newrepo web-app
commit_in "$TARGET/web-app" "$recent" "init: scaffold app"
remote=$(mkbare web-app)
gi "$TARGET/web-app" "$recent" remote add origin "$remote"
gi "$TARGET/web-app" "$recent" push -q -u origin main
clone="$SCRATCH/web-app-clone"
git clone -q "$remote" "$clone"
commit_in "$clone" "$recent" "feat: cart drawer"
commit_in "$clone" "$recent" "feat: promo codes"
commit_in "$clone" "$recent" "fix: checkout total"
gi "$clone" "$recent" push -q origin main
gi "$TARGET/web-app" "$recent" fetch -q

# 3. mobile-app — diverged (1 ahead, 2 behind).
newrepo mobile-app
commit_in "$TARGET/mobile-app" "$recent" "init: app shell"
remote=$(mkbare mobile-app)
gi "$TARGET/mobile-app" "$recent" remote add origin "$remote"
gi "$TARGET/mobile-app" "$recent" push -q -u origin main
clone="$SCRATCH/mobile-app-clone"
git clone -q "$remote" "$clone"
commit_in "$clone" "$recent" "feat: push notifications"
commit_in "$clone" "$recent" "feat: deep links"
gi "$clone" "$recent" push -q origin main
gi "$TARGET/mobile-app" "$recent" fetch -q
commit_in "$TARGET/mobile-app" "$recent" "wip: offline mode"

# 4. auth-service — clean and in sync.
newrepo auth-service
commit_in "$TARGET/auth-service" "$recent" "chore: bump deps"
remote=$(mkbare auth-service)
gi "$TARGET/auth-service" "$recent" remote add origin "$remote"
gi "$TARGET/auth-service" "$recent" push -q -u origin main

# 5. billing — dirty working tree (staged + modified + untracked), no remote.
newrepo billing
commit_in "$TARGET/billing" "$recent" "feat: invoices"
printf 'tax()\n' >>"$TARGET/billing/history.log"        # modified (tracked, unstaged)
printf 'staged change\n' >"$TARGET/billing/CHANGELOG.md"
gi "$TARGET/billing" "$recent" add CHANGELOG.md          # staged
printf 'scratch\n' >"$TARGET/billing/scratch.txt"        # untracked

# 6. infra — detached HEAD plus an untracked file.
newrepo infra
commit_in "$TARGET/infra" "$recent" "ci: pin runner image"
commit_in "$TARGET/infra" "$recent" "ci: cache deps"
gi "$TARGET/infra" "$recent" checkout -q --detach HEAD
printf 'TODO\n' >"$TARGET/infra/TODO.txt" # untracked

# 7. data-pipeline — a stale stash (2 weeks old).
newrepo data-pipeline
commit_in "$TARGET/data-pipeline" "$recent" "feat: nightly ETL"
printf 'wip backfill\n' >>"$TARGET/data-pipeline/history.log" # modified (tracked)
gi "$TARGET/data-pipeline" "$stale" stash push -q -m "wip: backfill"

# 8. docs-site — a fresh stash (today).
newrepo docs-site
commit_in "$TARGET/docs-site" "$recent" "docs: api reference"
printf 'draft\n' >>"$TARGET/docs-site/history.log" # modified (tracked)
gi "$TARGET/docs-site" "$recent" stash push -q -m "wip: tutorial"

# 9. legacy-billing — unreadable (a .git file pointing nowhere).
mkdir -p "$TARGET/legacy-billing"
printf 'gitdir: /nonexistent/legacy-billing.git\n' >"$TARGET/legacy-billing/.git"

count=$(find "$TARGET" -maxdepth 2 -name .git | wc -l | tr -d ' ')
echo "Generated $count sample repos in $TARGET"
echo "Try:  cohors --root $TARGET"
