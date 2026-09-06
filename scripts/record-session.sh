#!/usr/bin/env sh
# Append one session event to the Principia history log.
# Invoked by hooks/hooks.json on SessionStart and SessionEnd.
# Must never fail loudly: a broken hook must not break the user's session.
set -u
PHASE="${1:-start}"
HOME_DIR="${HOME:-/tmp}"
DIR="$HOME_DIR/.principia/history"
mkdir -p "$DIR" 2>/dev/null || exit 0

REPO="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
BRANCH="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '')"
STAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
DAY="$(date -u +%Y-%m-%d)"

printf '{"ts":"%s","phase":"%s","runner":"claude-code","repo":"%s","branch":"%s","session":"%s"}\n' \
  "$STAMP" "$PHASE" "$REPO" "$BRANCH" "${CLAUDE_SESSION_ID:-}" \
  >> "$DIR/$DAY.jsonl" 2>/dev/null || true
exit 0
