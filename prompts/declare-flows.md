# Declare this repository's workflows

Runner-agnostic. Claude Code, Codex, Gemini CLI, agy and anything else follow
this same file. It is a focused pass over one part of the contract; the whole
picture is in `prompts/setup-repo.md`, and `{{SPEC}}` wins over both.

Your job: decide which flows this repo should **declare**, and write only those
into `.principia/repo.json` under `flows`.

---

## First, understand what you do not need to write

The launchpad already auto-detects, with zero config: npm/pnpm/yarn/bun scripts
(`dev`, `start`, `build`, `test`, `lint`, `typecheck`, `check`), cargo commands,
`uv sync` / pytest, Makefile targets, and `./gradlew assembleDebug`.

Those already appear under **Scripts**. A declared flow that only restates one of
them adds nothing and creates a file that goes stale. Do not write it.

## Write a flow only when it adds what detection cannot infer

- **A label a human recognises.** `"Storefront (SSR)"` beats `"dev"`.
- **A port**, so the launchpad can offer to open a browser.
- **Ordering**: which single flow is `primary` — the one you start first.
- **A composite command** detection would never guess, e.g.
  `docker compose up -d && npm run dev`.
- **The right `cwd`** in a monorepo, relative to the repo root.
- **`background: true`** for anything long-running.

## How to decide

1. Read what actually exists: `README.md`, `package.json`, `Cargo.toml`,
   `pyproject.toml`, `Makefile`, `docker-compose.yml`, CI workflows, and any
   `CONTRIBUTING`/`CLAUDE.md` that describes how people run this project.
2. Ask: *how does someone actually start this thing on a fresh clone?* If the
   real answer is two commands in order, that is a composite flow worth
   declaring. If the real answer is `npm run dev`, detection already has it.
3. Prefer three good flows over ten mechanical ones.

## Writing it

You are **updating** `.principia/repo.json`, not replacing it. Preserve
`agents`, `tasks`, and any key you do not understand. If the file does not
exist, follow `prompts/setup-repo.md` first.

Rules the validator enforces:

- Every `id` is kebab-case and unique within `flows`.
- **No absolute paths.** `run` and `cwd` are relative to the repo root.
- At most one flow may be `"primary": true`.
- `kind` is one of `server`, `task`, `test`, `build`, `tool`.
- No secrets, tokens, hostnames or machine-specific values. This file is
  committed; machine-local values belong in the gitignored
  `.principia/local.json`, which you should not write.

Refresh `updated` to now (ISO-8601 UTC) and `generatedFrom` to the current short
HEAD SHA.

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
2. Fix every `ERROR`. Warnings are advisory.
3. Report what you declared, and **what you deliberately left to detection and
   why**. An empty `flows` array with a reason is a better answer than five
   flows that restate `package.json`.

Do not commit unless the user asks.
