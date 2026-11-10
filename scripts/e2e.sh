#!/usr/bin/env bash
# End-to-end check of cohors against a sandbox fleet of real git repositories.
#
# Unit tests prove the pieces; this proves the product: it builds a small fleet
# with real remotes, in every state cohors is meant to notice, runs the real
# `cohors` binary against it (commands, then the dashboard driven by real key
# presses in a pseudo-terminal), and checks each result against git itself.
#
# Everything lives under one temporary directory with its own HOME, so nothing
# of yours is read or written: not your repos, your config, your cache, or
# your launchpad.
#
#   scripts/e2e.sh                 # uses `cohors` from your PATH
#   COHORS=target/debug/cohors scripts/e2e.sh
#
# Exits non-zero on the first failed check.
set -euo pipefail

COHORS="${COHORS:-cohors}"
HERE="$(cd "$(dirname "$0")" && pwd)"
BOX="$(mktemp -d "${TMPDIR:-/tmp}/cohors-e2e.XXXXXX")"
BOX="$(cd "$BOX" && pwd -P)" # the real path, as cohors and git will report it
trap 'rm -rf "$BOX"' EXIT

export HOME="$BOX/home"
export XDG_CONFIG_HOME="$HOME/.config" XDG_CACHE_HOME="$HOME/.cache"
export GIT_CONFIG_GLOBAL="$HOME/.gitconfig" GIT_CONFIG_NOSYSTEM=1
mkdir -p "$HOME" "$BOX/remotes" "$BOX/fleet" "$BOX/scratch"
git config --global user.name "E2E"
git config --global user.email "e2e@example.com"
git config --global init.defaultBranch main
git config --global advice.detachedHead false

FLEET="$BOX/fleet"
pass=0
ok()   { pass=$((pass + 1)); printf '  ok    %s\n' "$1"; }
fail() { printf '  FAIL  %s\n        %s\n' "$1" "${2:-}"; exit 1; }
# check "what it proves" <command that must succeed>
check() { local name="$1"; shift; if "$@" >/dev/null 2>&1; then ok "$name"; else fail "$name" "$*"; fi; }
eq()    { if [ "$2" = "$3" ]; then ok "$1"; else fail "$1" "got '$2', wanted '$3'"; fi; }
co()    { "$COHORS" --root "$FLEET" "$@"; }
json()  { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

# ── build the fleet ──────────────────────────────────────────────────────────
# A repo with a bare remote and one pushed commit.
make() {
  local name="$1"
  git init -q --bare "$BOX/remotes/$name.git"
  git clone -q "$BOX/remotes/$name.git" "$FLEET/$name" 2>/dev/null
  ( cd "$FLEET/$name" && echo "$name" > README.md && git add . && git commit -qm "init $name" && git push -q -u origin main )
}
commit() { ( cd "$1" && echo "$2" >> notes.txt && git add . && git commit -qm "$2" ); }

build_fleet() {
  rm -rf "$BOX/remotes" "$FLEET" "$BOX/scratch"; mkdir -p "$BOX/remotes" "$FLEET" "$BOX/scratch"
  make clean
  make ahead;       commit "$FLEET/ahead" "local one"; commit "$FLEET/ahead" "local two"
  make behind
  git clone -q "$BOX/remotes/behind.git" "$BOX/scratch/behind"
  commit "$BOX/scratch/behind" "upstream change"; ( cd "$BOX/scratch/behind" && git push -q )
  ( cd "$FLEET/behind" && git fetch -q )
  make unpublished; ( cd "$FLEET/unpublished" && git checkout -q -b feature ); commit "$FLEET/unpublished" "feature work"
  make dirty;       ( cd "$FLEET/dirty" && echo change >> README.md && echo new > untracked.txt )
  make stashed;     ( cd "$FLEET/stashed" && echo wip >> README.md && git stash -q )
  git init -q "$FLEET/local-only"; commit "$FLEET/local-only" "no remote here"
}

remote_head() { git -C "$BOX/remotes/$1.git" rev-parse --verify -q "refs/heads/${2:-main}" || echo none; }
local_head()  { git -C "$FLEET/$1" rev-parse HEAD; }
action()      { co overview --json | json "next((a for a in d['actions'] if a['kind']=='$1'), None) is not None"; }
targets()     { co overview --json | json "sorted(r.split('/')[-1] for a in d['actions'] if a['kind']=='$1' for r in a['repos'])"; }

echo "cohors end-to-end, in $BOX"
echo "binary: $(command -v "$COHORS" || echo "$COHORS")"

# ── 1. it sees the truth ─────────────────────────────────────────────────────
echo; echo "reading the fleet"
build_fleet
eq "finds all seven repositories" "$(co overview --json | json "d['figures']['repos']")" "7"
# `clean`, and `local-only`: a repo with no remote is local on purpose, not a problem.
eq "counts the two repositories with nothing to report" "$(co overview --json | json "d['figures']['clean']")" "2"
eq "push step targets the repo with unpushed commits" "$(targets push)" "['ahead']"
eq "publish step targets the never-pushed branch, not the repo with no remote" "$(targets publish)" "['unpublished']"
eq "pull step targets the repo that is behind" "$(targets pull)" "['behind']"
eq "commit step targets the repo with uncommitted files" "$(targets commit)" "['dirty']"
eq "two commits are reported as not backed up" "$(co overview --json | json "d['figures']['unpushed_commits']")" "2"
check "the scan report names every repo with something to say" \
  bash -c "out=\$('$COHORS' --root '$FLEET' scan --json | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))'); [ \"\$out\" = 7 ]"

# ── 2. each suggested command does what its step says ────────────────────────
echo; echo "commands"
eq "dry run changes nothing" "$(co push --select 'ahead,!diverged' --dry-run >/dev/null; remote_head ahead)" "$(git -C "$FLEET/ahead" rev-parse 'HEAD~2')"
co push --select 'ahead,!diverged' >/dev/null
eq "push sends the unpushed commits" "$(remote_head ahead)" "$(local_head ahead)"

eq "the feature branch is not on the remote yet" "$(remote_head unpublished feature)" "none"
co push --select unpublished >/dev/null
eq "push publishes the never-pushed branch" "$(remote_head unpublished feature)" "$(local_head unpublished)"
eq "and the branch now tracks it" "$(git -C "$FLEET/unpublished" rev-parse --abbrev-ref 'feature@{upstream}')" "origin/feature"
check "the repo with no remote was left alone" bash -c "! git -C '$FLEET/local-only' remote | grep -q ."

co pull --select 'behind,!diverged' >/dev/null
eq "pull fast-forwards the repo that was behind" "$(local_head behind)" "$(remote_head behind)"

co commit -m "e2e: snapshot" --select dirty >/dev/null
eq "commit records the uncommitted files" "$(git -C "$FLEET/dirty" status --porcelain | wc -l | tr -d ' ')" "0"
eq "with the message given" "$(git -C "$FLEET/dirty" log -1 --format=%s)" "e2e: snapshot"

( cd "$FLEET/clean" && echo parked >> README.md )
co stash --select name:clean >/dev/null
eq "stash parks the change" "$(git -C "$FLEET/clean" stash list | wc -l | tr -d ' ')" "1"
eq "and leaves a clean tree" "$(git -C "$FLEET/clean" status --porcelain | wc -l | tr -d ' ')" "0"

check "fetch reaches every repo with a remote" co fetch --select all
eq "run executes in each selected repo" "$(co run 'git rev-parse --abbrev-ref HEAD' --select name:unpublished 2>/dev/null | grep -c feature)" "1"

# The commit above left one repo ahead; the overview says so, and a push clears it.
eq "the new commit shows up as something to push" "$(targets push)" "['dirty']"
co push --select 'ahead,!diverged' >/dev/null
eq "commit then push lands the change on the remote" "$(remote_head dirty)" "$(local_head dirty)"

eq "afterwards nothing is left to push, publish or pull" \
  "$(co overview --json | json "[a['kind'] for a in d['actions'] if a['kind'] in ('push','publish','pull')]")" "[]"

# ── 3. the launchpad and the agent commands ──────────────────────────────────
echo; echo "launchpad and agents"
mkdir -p "$FLEET/clean/.cohors"
cat > "$FLEET/clean/.cohors/repo.json" <<'JSON'
{"specVersion": 1, "name": "clean", "summary": "The clean one",
 "flows": [{"id": "test", "label": "Tests", "run": "echo ok"}],
 "tasks": [{"id": "ship-it", "title": "Ship it", "priority": "high"}]}
JSON
check "a valid launchpad file validates" co launchpad validate "$FLEET/clean"
eq "its task shows up in the overview" "$(co overview --json | json "d['figures']['open_tasks']")" "1"
printf '{"specVersion":1,"name":"x","flows":[{"id":"t","label":"T","run":"echo ok\\nrm -rf ~"}]}' > "$FLEET/dirty/.cohors.tmp"
mkdir -p "$FLEET/dirty/.cohors" && mv "$FLEET/dirty/.cohors.tmp" "$FLEET/dirty/.cohors/repo.json"
check "a flow hiding a second command is rejected" bash -c "! '$COHORS' launchpad validate '$FLEET/dirty'"
eq "and is never offered" "$(co launchpad --json | json "[len(r['contribution']['flows']) for r in d['repos'] if r['name']=='x']")" "[0]"

eq "every runner is listed, installed or not" "$(co agent runners --json | json "len(d['runners'])")" "4"
eq "five prompts ship" "$(co agent prompts --json | json "len(d['prompts'])")" "5"
# A stand-in runner on the PATH, so launches can be exercised without a real agent.
mkdir -p "$BOX/bin"; printf '#!/bin/sh\nprintf "%%s" "$1" > "%s/agent-got.txt"\n' "$BOX" > "$BOX/bin/claude"; chmod +x "$BOX/bin/claude"
PATH="$BOX/bin:$PATH" SHELL=/bin/sh co agent run setup-repo --repo "$FLEET/ahead" >/dev/null 2>&1
check "agent run hands the runner the setup prompt" grep -q "cohors launchpad" "$BOX/agent-got.txt"
check "and tells it which repository" grep -q "$FLEET/ahead" "$BOX/agent-got.txt"
plan="$(PATH="$BOX/bin:$PATH" SHELL=/bin/sh co agent command task --repo "$FLEET/clean" --task ship-it)"
eq "a task launch is linked to a conversation" "$(printf '%s' "$plan" | json "d['linked']")" "True"
session="$(printf '%s' "$plan" | json "d['session']")"
eq "the link is recorded, gitignored" "$(json "d['threads']['ship-it']" < "$FLEET/clean/.cohors/local.json")" "$session"
check "local.json is in .gitignore" grep -q ".cohors/local.json" "$FLEET/clean/.gitignore"
eq "the same task resumes that conversation next time" \
  "$(PATH="$BOX/bin:$PATH" SHELL=/bin/sh co agent command task --repo "$FLEET/clean" --task ship-it | json "d['command']")" "claude --resume '$session'"
eq "the launches are in the activity log" "$(co launchpad --json | json "len(d['recent_activity']) >= 3")" "True"

# ── 4. the dashboard, driven by real keys ────────────────────────────────────
echo; echo "dashboard (real key presses in a pseudo-terminal)"
build_fleet
SHOTS="$BOX/shots"
shoot() { python3 "$HERE/term-shot.py" --out "$SHOTS" --size 119x45 --settle 2500 "$@" -- "$COHORS" --root "$FLEET" --watch >/dev/null; }

shoot --keys "shot:start q"
check "it opens on the overview" grep -q "Do next" "$SHOTS/start.txt"
check "showing the real fleet" grep -q "7 repositories" "$SHOTS/start.txt"
check "and the publish step" grep -q "Publish 1 branch that has no upstream" "$SHOTS/start.txt"

# P on "Do next": push and publish. p: pull. Then quit.
shoot --keys "1 P wait:2500 p wait:2500 shot:after q"
eq "P pushed the unpushed commits" "$(remote_head ahead)" "$(local_head ahead)"
eq "P published the never-pushed branch" "$(remote_head unpublished feature)" "$(local_head unpublished)"
eq "p pulled the repo that was behind" "$(local_head behind)" "$(remote_head behind)"
check "the overview no longer lists them" bash -c "! grep -q 'Publish 1 branch' '$SHOTS/after.txt'"

# S asks first; n cancels, and nothing is stashed.
shoot --keys "5 / text:dirty enter S wait:300 shot:confirm n wait:300 q"
check "S asks before stashing" grep -q "Stash changes in 1 repo" "$SHOTS/confirm.txt"
eq "n cancels: the change is still there" "$(git -C "$FLEET/dirty" stash list | wc -l | tr -d ' ')" "0"
shoot --keys "5 / text:dirty enter S wait:300 y wait:2000 q"
eq "y stashes it" "$(git -C "$FLEET/dirty" stash list | wc -l | tr -d ' ')" "1"

# The detail view, the agent key (which asks first), and resizing.
shoot --keys "5 enter wait:500 shot:detail a wait:300 shot:agent n resize:80x24 shot:small resize:40x10 shot:tiny resize:119x45 shot:back q"
check "Enter opens the detail view" grep -q "signals" "$SHOTS/detail.txt"
check "a asks before starting an agent" grep -q "Start your coding agent in" "$SHOTS/agent.txt"
check "it redraws after a resize to 80x24" grep -q "Do next\|Repositories" "$SHOTS/small.txt"
check "it says so when the terminal is too small" grep -q "Terminal too small" "$SHOTS/tiny.txt"
check "and recovers when enlarged again" grep -q "Repositories" "$SHOTS/back.txt"
check "no screen is ever wider than the terminal" python3 - "$SHOTS" <<'PY'
import glob, sys
limits = {"small": 80, "tiny": 40}
for path in glob.glob(sys.argv[1] + "/*.txt"):
    name = path.rsplit("/", 1)[1][:-4]
    width = max((len(line) for line in open(path).read().split("\n")), default=0)
    assert width <= limits.get(name, 119), (name, width)
PY

echo; echo "$pass checks passed"
