# Set up this repository for the Principia launchpad

This is the canonical, runner-agnostic instruction. Claude Code, Codex, Gemini
CLI, agy, and any future runner all follow this same file. The Claude Code skill
and agent in this plugin are thin wrappers that point here.

Your job: read the repository you are in, then write `.principia/repo.json` (and
optionally `.principia/agents/*.md`) so the launchpad can show it properly.

---

## Before you write anything

1. **Read the contract.** `spec/v1/SPEC.md` in the Principia repo is
   authoritative. If this file and the spec disagree, the spec wins.
2. **Look at the repo, do not guess.** Read in this order, whichever exist:
   `README.md`, `package.json`, `Cargo.toml`, `pyproject.toml`, `Makefile`,
   `settings.gradle[.kts]`, `*.xcodeproj`, `docker-compose.yml`, `CLAUDE.md`.
3. **Check what already exists.** If `.principia/repo.json` is present you are
   *updating*, not replacing. Preserve `tasks[]` the user may be mid-way through
   and preserve any keys you do not understand.

## What is worth writing down

The launchpad already auto-detects the obvious things with no config at all:
npm scripts, cargo commands, Makefile targets, gradle tasks, uv/pytest. **Do not
write a flow that only restates one of those.** A `repo.json` full of
`npm run dev` adds nothing and creates a file that will go stale.

Write a flow when it adds something detection cannot infer:

- a **label** a human would actually recognise (`"Storefront (SSR)"` beats `"dev"`)
- the **port**, so the launchpad can offer to open a browser
- **ordering**: which one is `primary`, the thing you start first
- a **composite** command that detection would never guess
  (`docker compose up -d && npm run dev`)
- the right **cwd** in a monorepo

If the repo genuinely has nothing beyond detection, write the minimal file with
`name` and `summary` and leave `flows` empty. That is a correct answer, and it
still gives the repo a name, a group and a one-line description in the dashboard.

## Writing it

Create `.principia/repo.json`:

```json
{
  "specVersion": 1,
  "name": "<repo display name>",
  "summary": "<one line, max 80 chars, what this repo IS>",
  "group": "<optional: the product or area this belongs to>",
  "updated": "<ISO-8601 UTC, now>",
  "generatedFrom": "<current git HEAD short SHA>",
  "flows": [],
  "agents": [],
  "tasks": []
}
```

Hard rules, all enforced by the validator:

- Every `id` is kebab-case and unique within its array.
- **No absolute paths.** `run` and `cwd` are relative to the repo root.
- **No secrets, tokens, hostnames, or machine-specific values.** This file is
  committed. Anything machine-local belongs in `.principia/local.json`, which is
  gitignored and which you should not write.
- At most one flow may be `"primary": true`.
- `summary` is 80 characters or fewer.

## Agents this repo should ship

Only add an agent if the repo has a recurring job that benefits from repo
context. Good candidates: triaging uncommitted work, cutting a release,
reviewing a diff against this repo's conventions. Bad candidates: anything
generic, which belongs in the user's own agent set, not in this repo.

Write `.principia/agents/<id>.md`:

```markdown
---
id: triage
label: Triage uncommitted work
description: One line the launchpad shows on the button.
supports: [claude-code, codex, gemini-cli, agy]
readOnly: true
isolation: none
---

You are working in {{REPO_NAME}} at {{REPO_ROOT}} on branch {{BRANCH}}.

<the actual instruction>
```

Set `readOnly: true` unless the agent genuinely needs to write. Set
`isolation: worktree` for anything that rewrites files, so it cannot disturb a
dirty working tree.

Then reference it from `repo.json`:

```json
"agents": [
  { "id": "triage", "label": "Triage uncommitted work",
    "file": ".principia/agents/triage.md",
    "supports": ["claude-code", "codex", "gemini-cli", "agy"] }
]
```

## Tasks

Only short-lived, repo-specific work that a person would want surfaced when they
open this repo. A real backlog belongs in an issue tracker, not here. If you
cannot name something concrete, leave `tasks` empty rather than inventing filler.

Never set `"status": "done"` without evidence: a commit, a merged PR, or the
user saying so.

## Finish

1. Ensure `.gitignore` contains `.principia/local.json`.
2. Validate:

   ```sh
   node <principia>/scripts/validate.js .
   ```

3. Fix every `ERROR`. Warnings are advisory.
4. Report to the user what you wrote and, specifically, **what you chose not to
   write and why**. An empty `flows` array with a reason is a better outcome
   than five flows that restate `package.json`.

Do not commit unless the user asks.
