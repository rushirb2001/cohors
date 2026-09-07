# Decide whether this repository should ship an agent

Runner-agnostic. Every runner follows this same file. It is a focused pass over
one part of the contract; the whole picture is in `prompts/setup-repo.md`, and
`{{SPEC}}` wins over both.

Your job: decide whether this repo has a recurring job worth committing as an
agent prompt, and if so write `.principia/agents/<id>.md` and reference it from
`.principia/repo.json`.

---

## The bar for adding one

A repo-shipped agent is a prompt that **needs this repository's context** and
that someone would run **more than once**. It is committed, so it travels to
everyone who clones the repo, under whichever runner they happen to have.

Good candidates:

- Triage the working tree and propose a commit split.
- Cut a release the way *this* project cuts releases.
- Review a diff against conventions this repo actually documents.
- Re-run a check whose rules live in this repo and drift easily.

Bad candidates, which belong in the user's own agent set and not in this repo:

- "Explain this code", "write tests", "fix the bug" — generic, no repo context.
- Anything that restates what a flow already runs.
- Anything you cannot describe in one concrete sentence.

**Writing nothing is a correct outcome.** Most repositories do not need one. If
that is the answer here, say so and stop; do not invent a plausible-sounding
agent to fill the slot.

## How to decide

1. Read `README.md`, `CONTRIBUTING`, `CLAUDE.md`/`AGENTS.md`, the CI config, and
   any `scripts/` — recurring jobs usually leave traces in exactly those places.
2. Look at the repo's real history: `git log --oneline -30`. Work that shows up
   again and again is a candidate; a one-off is not.
3. Pick at most one or two. This is not a place to be comprehensive.

## Writing it

Create `.principia/agents/<id>.md`:

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

<the actual instruction, specific to this repository>
```

The launchpad substitutes `{{REPO_NAME}}`, `{{REPO_ROOT}}`, `{{BRANCH}}` and
`{{TASK_TITLE}}` before invoking a runner.

Set `readOnly: true` unless the agent genuinely needs to write. Set
`isolation: worktree` for anything that rewrites files, so it cannot disturb a
dirty working tree. List only the runners you actually expect to work.

Then reference it from `.principia/repo.json`, **updating** the file rather than
replacing it:

```json
"agents": [
  { "id": "triage", "label": "Triage uncommitted work",
    "file": ".principia/agents/triage.md",
    "supports": ["claude-code", "codex", "gemini-cli", "agy"] }
]
```

Rules the validator enforces: ids are kebab-case and unique; `file` is relative,
contains no `..`, lives under `.principia/`, and must actually exist.

If a task in `tasks[]` is exactly what this agent is for, set that task's
`agent` to this agent's id so the task's action runs it directly.

Refresh `updated` and `generatedFrom`.

## Prefer the MCP server when it is connected

If the `principia` MCP server is available, check your tools for
`write_repo_contribution`. Use it instead of writing the JSON by hand: it
validates against the contract **before** writing and returns the exact list of
errors, so an invalid file never reaches disk. `repo_status` and `read_contract`
answer what is already configured.

Fall back to editing the file directly plus `{{VALIDATE}}` only when the MCP
server is not connected.

## Finish

1. Validate: `{{VALIDATE}}`
2. Fix every `ERROR`.
3. Report what you wrote, or **why this repo does not need an agent**. The
   second is a perfectly good report.

Do not commit unless the user asks.
