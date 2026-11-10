---
name: launchpad-setup
description: >
  Teach a repository to describe itself to the cohors launchpad by writing its
  committed .cohors/repo.json (flows, agents, tasks). Use when the user asks
  to "set up this repo for the launchpad", "add this repo to the launchpad",
  "configure the launchpad here", or when `launchpad` reports the repo as
  configured: false.
---

# Set up a repo for the launchpad

A repository opts in by committing `.cohors/repo.json`. The file travels
with the clone, so a teammate or a second machine gets the entry for free.

## Before writing

1. Call `read_contract`. It is authoritative; do not guess the schema.
2. Call `launchpad` with a selector for this repo. If it is already
   `configured`, you are **updating**: the write replaces the whole document,
   so carry forward every flow, agent and task you are not deliberately
   changing, above all tasks that are `doing`.
3. Read the repo itself: `README.md`, the build manifest (`package.json`,
   `Cargo.toml`, `pyproject.toml`, `Makefile`, `settings.gradle`),
   `docker-compose.yml`, `CLAUDE.md`.

## What is worth writing down

`detected_flows` in the `launchpad` result shows what is already inferred with
zero config (npm/pnpm/yarn/bun scripts, cargo, uv/pytest, Makefile targets,
gradle). **Do not restate those.** Declare a flow only when it adds something
detection cannot know:

- a label a person recognises ("Storefront (SSR)" rather than "dev")
- the port
- which single flow is `primary`
- a composite command (`docker compose up -d && npm run dev`)
- the right `cwd` in a monorepo

An empty `flows` array is a correct answer. A file with just `name`, `summary`
and `group` still gives the repo a real entry.

**Agents:** add one only for recurring work that needs this repo's context
(cutting a release, checking a repo-specific invariant). Write the prompt to
`.cohors/agents/<id>.md` first, then reference it; the write is rejected if
the file does not exist. Most repos need none.

**Tasks:** short-lived and drawn from evidence — uncommitted work, unpushed
commits, a failing test, a TODO naming a bounded change. A backlog belongs in
an issue tracker. Never set `status: "done"` without a commit, a merged PR, or
the user saying so.

**Nothing machine-specific.** No absolute paths, hostnames, secrets, tokens or
session ids: the file is committed.

## Writing it

Call `write_repo_contribution` with `dry_run: true` first, fix every error it
lists, then call it for real. Set `generatedFrom` to the current short commit
SHA so staleness can be shown later.

The plugin starts the server **read-only**, so the real write is refused
until the user opts in. When that happens, do one of:

- Write `.cohors/repo.json` yourself with your normal file tools (the user
  approves the edit like any other), stamping `"specVersion": 1` and an
  ISO-8601 UTC `"updated"`, then check it:

  ```sh
  cohors launchpad validate .
  ```

- Or tell the user they can let the server write launchpad files directly by
  adding `--allow-launchpad` to the server's arguments.

Either way, `dry_run: true` always works and is the fastest way to find every
problem before writing.

## Finish

Report what you wrote and, specifically, what you chose **not** to write and
why. Do not commit unless the user asks.
