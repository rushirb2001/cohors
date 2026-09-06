# Principia contract, v1

This is the contract between a repository and the Principia launchpad. Any AI
agent, in any repo, can read this file and know exactly what to write.

It is deliberately agent-agnostic. Claude Code, Codex, agy, Gemini CLI, or a
human with an editor all produce the same artifact.

---

## 1. The model in one paragraph

The launchpad ships **empty**. It discovers candidate repositories from the
editor's own recently-opened list, then asks each one what it offers. A
repository answers by committing a `.principia/` directory. The launchpad
aggregates every answer into one dashboard. Nothing about the user is stored in
the extension itself, so a fresh install on a new machine behaves identically to
a stranger's first run.

The unit of enrichment is **the repository**, not the dashboard.

---

## 2. Where things live

| Path | Scope | Committed? | Written by |
| --- | --- | --- | --- |
| `<repo>/.principia/repo.json` | One repository | **Yes** | Agent |
| `<repo>/.principia/agents/*.md` | One repository | **Yes** | Agent |
| `<repo>/.principia/local.json` | One repo, one machine | **No** (gitignored) | Extension |
| `~/.principia/board.json` | User, all repos | Optional | Agent |
| `~/.principia/history/*.jsonl` | User, all repos | Optional | Extension + hooks |

Committed files are the point: clone the repo on another machine and its
launchpad entry comes with it. Machine-local values (absolute paths, chosen
ports, device ids) never enter a committed file.

`~/.principia/` is the portable home. When the Claude Code plugin is installed,
it mirrors the same files into `${CLAUDE_PLUGIN_DATA}` so plugin-side agents can
read them without knowing the user's home layout. The extension treats
`~/.principia/` as authoritative and keeps the mirror fresh; neither copy is
allowed to go stale silently (see §7).

---

## 3. `.principia/repo.json`

The only required file. Minimal valid example:

```json
{
  "specVersion": 1,
  "name": "my-service",
  "summary": "One line, shown under the repo name."
}
```

Full shape:

```jsonc
{
  "specVersion": 1,                  // required, integer, currently 1
  "name": "sushrutalgs-bff",         // required, display name
  "summary": "Backend for frontend", // one line, <= 80 chars
  "group": "Platform",               // free-text; groups repos in the dashboard
  "icon": "server",                  // codicon name; omit to auto-pick by stack
  "links": [
    { "label": "Docs", "url": "https://..." }
  ],

  // Things the user can start. Each becomes a launchable item.
  "flows": [
    {
      "id": "dev",                   // required, unique within the repo
      "label": "Dev server",         // required
      "run": "npm run dev",          // shell command, run at repo root
      "cwd": ".",                    // optional, relative to repo root
      "kind": "server",              // server | task | test | build | tool
      "port": 3000,                  // optional; enables the "open in browser" action
      "background": true,            // long-running?
      "primary": true                // at most one; the default action for this repo
    }
  ],

  // Agents this repo ships. See §4.
  "agents": [
    {
      "id": "triage",
      "label": "Triage uncommitted work",
      "file": ".principia/agents/triage.md",
      "supports": ["claude-code", "codex", "gemini-cli", "agy"]
    }
  ],

  // Work the repo knows about. The dashboard shows these as focus candidates.
  // Keep short-lived; real backlogs belong in an issue tracker.
  "tasks": [
    {
      "id": "billing-retries",
      "title": "Wire billing webhook retries",
      "status": "todo",              // todo | doing | blocked | done
      "priority": "normal",          // high | normal | low
      "notes": "Optional context."
    }
  ]
}
```

**Rules**

- Every `id` is stable and kebab-case. The dashboard keys off it; renaming one
  orphans its state.
- No absolute paths. Ever. `run` and `cwd` are relative to the repo root.
- No secrets, tokens, or machine-specific values. This file is committed.
- Unknown keys are preserved, not dropped, so a newer extension can round-trip an
  older file.

---

## 4. Repo-declared agents

`.principia/agents/<id>.md` is a prompt with YAML frontmatter. It is
**agent-runner agnostic**: the frontmatter declares intent, and each runner maps
it to its own flags.

```markdown
---
id: triage
label: Triage uncommitted work
description: Read the working tree and propose what to commit, split, or discard.
supports: [claude-code, codex, gemini-cli, agy]
model: sonnet          # optional hint; runners map or ignore
readOnly: true         # if true, runners are invoked without write permission
isolation: none        # none | worktree
---

You are working in {{REPO_NAME}} at {{REPO_ROOT}}.

Read `git status --porcelain` and `git diff`, then propose a commit plan...
```

Placeholders the launchpad substitutes before invoking a runner:
`{{REPO_NAME}}`, `{{REPO_ROOT}}`, `{{BRANCH}}`, `{{TASK_TITLE}}`.

### Runner mapping

| Runner | Invocation |
| --- | --- |
| `claude-code` | `claude --append-system-prompt "$(cat <file>)"` in the repo, or a plugin agent when Principia's plugin is installed |
| `codex` | `codex --prompt-file <file>` |
| `gemini-cli` | `gemini -p "$(cat <file>)"` |
| `agy` | `agy run --prompt <file>` |

A runner listed in `supports` but not installed is shown disabled with a reason,
never hidden. The user should never wonder why an action vanished.

---

## 5. What the launchpad does without any of this

Zero-config behaviour, so a fresh install is useful immediately:

1. Reads the editor's recently-opened list (`_workbench.getRecentlyOpened`).
2. Keeps entries that exist on disk and contain a `.git` directory.
3. Reads branch, dirty count, and ahead/behind for each.
4. **Auto-detects flows** without any config:
   - `package.json` scripts (`dev`, `start`, `build`, `test`, `lint`)
   - `Cargo.toml` → `cargo check|build|test|run`
   - `pyproject.toml` / `uv.lock` → `uv sync`, `uv run pytest`
   - `Makefile` targets
   - `settings.gradle[.kts]` → `./gradlew assembleDebug`
   - `*.xcodeproj` / `Package.swift` → iOS build
5. Shows every discovered repo, including unconfigured ones, each with a
   **"Set up with an agent"** action.

Auto-detected flows are marked `"source": "detected"` in memory and are never
written to disk. Committing them is the agent's job, and only when it adds
something detection could not infer.

---

## 6. `~/.principia/board.json`

Cross-repo focus. Optional; the dashboard works without it.

```jsonc
{
  "specVersion": 1,
  "updated": "2026-09-06T12:00:00Z",
  "focus": [
    {
      "id": "f-1",
      "title": "Ship billing retries",
      "repo": "sushrutalgs-bff",     // matches repo.json name, not a path
      "ref": "billing-retries",      // optional: a task id inside that repo
      "status": "doing",
      "agent": "triage",             // optional: repo agent to launch
      "thread": "abc123"             // optional: prior session to resume
    }
  ]
}
```

`repo` is matched by **name**, never by absolute path, so a board syncs across
machines where checkouts live in different places.

---

## 7. Staleness

Anything cached must be able to prove it is current, or say that it is not.

- Every generated file carries `"updated"` as an ISO-8601 UTC timestamp.
- `repo.json` also carries an optional `"generatedFrom"`: the commit SHA the
  agent read when writing it. The dashboard shows a "may be out of date" hint
  when `HEAD` has moved far past it.
- The `${CLAUDE_PLUGIN_DATA}` mirror stores the source mtime it copied. On
  mismatch the extension re-mirrors rather than serving the old copy.
- The dashboard never displays a cached value without also being able to show its
  age.

---

## 8. Validating

```bash
node scripts/validate.js <path-to-repo>
```

Checks `repo.json` against `spec/v1/repo.schema.json`, verifies every
`agents[].file` exists, that ids are unique and kebab-case, that no `run` or
`cwd` is absolute, and that `specVersion` is supported. Agents should run this
before reporting success.

---

## 9. Versioning

`specVersion` is an integer. The extension supports the current version and one
back. When it reads an older file it migrates in memory and writes the new shape
only when something else changes the file. It never rewrites a repo's committed
config just to bump a version.
