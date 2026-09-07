# Record this repository's current work as tasks

Runner-agnostic. Every runner follows this same file. It is a focused pass over
one part of the contract; the whole picture is in `prompts/setup-repo.md`, and
`{{SPEC}}` wins over both.

Your job: find the work this repo actually has in flight, and record it in
`.principia/repo.json` under `tasks`.

---

## What belongs here

Short-lived, repo-specific work a person would want surfaced when they open this
repository. Each task gets a "work on this" action in the launchpad that hands it
straight to an agent, so a task should be something an agent could pick up.

**A real backlog belongs in an issue tracker, not here.** Five honest tasks beat
thirty; if you cannot name something concrete, leave `tasks` empty rather than
inventing filler.

## Where to find real work — evidence, not guesses

- `git status --porcelain` — uncommitted work that is clearly mid-something.
- `git log --oneline -20` and unpushed commits — a thread someone left open.
- `TODO`/`FIXME`/`XXX` comments that name a real, bounded change.
- Failing or skipped tests, and anything the README calls "not yet".
- A `docs/` note or session log that states known limitations explicitly.

Do not turn every TODO into a task. Pick the ones a maintainer would actually
agree are live work.

## Writing it

**Update** `.principia/repo.json`; do not replace it. Preserve `flows`,
`agents`, and any key you do not understand. Critically: **preserve existing
tasks the user may be part-way through** — match on `id` and leave their
`status` alone unless you have evidence it changed.

```json
"tasks": [
  {
    "id": "billing-retries",
    "title": "Wire billing webhook retries",
    "status": "todo",
    "priority": "normal",
    "notes": "Optional context, max 400 chars.",
    "agent": "triage"
  }
]
```

- `id` is kebab-case, unique, and **stable** — the launchpad keys off it, so
  renaming one orphans its state.
- `title` is at most 120 characters. Say the change, not the symptom.
- `status` is `todo`, `doing`, `blocked` or `done`.
- `priority` is `high`, `normal` or `low`. Not everything is high.
- `agent` is optional: the id of one of this repo's own `agents[]`. Set it when
  a specific shipped agent is the right way to do this task, and the task's
  action will run that agent with the task as its context.

**Never set `"status": "done"` without evidence**: a commit, a merged PR, or the
user saying so. A task you merely believe is finished is not finished.

No secrets, absolute paths, or machine-specific values. This file is committed.

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
3. Report the tasks you added, any you deliberately left out, and anything you
   found that looks like work but that you could not confirm.

Do not commit unless the user asks.
