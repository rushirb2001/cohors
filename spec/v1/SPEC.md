# The cohors launchpad contract, v1

This is the contract between a repository and the cohors launchpad. Any AI
agent, in any repo, can read this file and know exactly what to write.

A repository's part lives in `.cohors/` at its root; the user's part lives in
`~/.cohors/`. `cohors launchpad contract` prints the same rules as JSON, and
that output is authoritative if the two ever disagree.

It is deliberately agent-agnostic. Claude Code, Codex, agy, Gemini CLI, or a
human with an editor all produce the same artifact.

---

## 1. The model in one paragraph

The launchpad ships **empty**. cohors discovers the git repositories under the
user's configured roots, then asks each one what it offers. A repository
answers by committing a `.cohors/` directory. cohors aggregates every answer
into one dashboard, shown identically in the terminal, the browser, the editor
extension and over MCP. Nothing about the user is stored in cohors itself, so a
fresh install on a new machine behaves identically to a stranger's first run.

The unit of enrichment is **the repository**, not the dashboard.

---

## 2. Where things live

| Path | Scope | Committed? | Written by |
| --- | --- | --- | --- |
| `<repo>/.cohors/repo.json` | One repository | **Yes** | Agent |
| `<repo>/.cohors/agents/*.md` | One repository | **Yes** | Agent |
| `<repo>/.cohors/local.json` | One repo, one machine | **No** (gitignored) | cohors |
| `~/.cohors/board.json` | User, all repos | Optional | Agent |
| `~/.cohors/history/*.jsonl` | User, all repos | Optional | cohors + session hooks |

Committed files are the point: clone the repo on another machine and its
launchpad entry comes with it. Machine-local values (absolute paths, chosen
ports, device ids) never enter a committed file.

`~/.cohors/` is the launchpad home; set `$COHORS_HOME` to put it
somewhere else. There is one copy and it is authoritative. Agents reach it
through the cohors MCP tools (`read_board`, `write_board`, `record_activity`)
rather than by knowing the user's home layout.

---

## 3. `.cohors/repo.json`

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
  "name": "billing-service",         // required, display name
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
      "file": ".cohors/agents/triage.md",
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
      "notes": "Optional context.",
      "agent": "triage"              // optional: id of one of this repo's agents[]
    }
  ]
}
```

**Task actions**

Every task the dashboard shows — repo-declared or from the cross-repo board —
gets a "work on this now" action, with no extra config required. If `agent`
names one of this repo's `agents[].id`, that agent's prompt runs with
`{{TASK_TITLE}}` filled in and the task's `notes` appended. Otherwise a
generic prompt runs, built from the task's `title` and `notes` alone. A task
never has to declare an agent to be actionable; declaring one is only for
tasks that recur often enough to warrant tailored instructions.

**Rules**

- Every `id` is stable and kebab-case. The dashboard keys off it; renaming one
  orphans its state.
- No absolute paths. Ever. `run` and `cwd` are relative to the repo root.
- A flow's `run` is one line a person can read in full: at most 400
  characters, each one either printable ASCII or a letter or digit of a
  left-to-right script. Control characters, invisible characters, line
  breaks and right-to-left text are refused, because they let a command
  display as one thing and run as another. A flow that breaks this rule is
  rejected by the validator and is not offered on any surface.
- No secrets, tokens, or machine-specific values. This file is committed.
- Unknown keys are preserved, not dropped, so a newer cohors can round-trip an
  older file.

---

## 4. Repo-declared agents

`.cohors/agents/<id>.md` is a prompt with YAML frontmatter. It is
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

| Placeholder | Becomes |
| --- | --- |
| `{{REPO_NAME}}` | the repo's display name |
| `{{REPO_ROOT}}` | absolute path of the repo the runner is launched in |
| `{{BRANCH}}` | current branch |
| `{{TASK_TITLE}}` | the task's title, when a task triggered the run |

These exist so that a **committed** prompt never contains a machine-specific
path. The placeholder is what lives in git; the resolved value only ever appears
in the throwaway copy handed to the runner.

### Runner mapping

Every invocation must do two things: **submit the prompt as the first user
turn**, and **leave the user in an interactive session**. A mapping that only
submits gives a headless run that prints and exits; a mapping that only opens a
session drops the user into an idle agent that was never asked anything.

| Runner | Invocation |
| --- | --- |
| `claude-code` | `claude "$(cat <file>)"` |
| `codex` | `codex "$(cat <file>)"` |
| `gemini-cli` | `gemini -i "$(cat <file>)"` |
| `agy` | `agy --prompt-interactive "$(cat <file>)"` |

The prompt is always passed as a positional/interactive prompt, never as a
system-prompt append and never through a headless flag (`claude -p`,
`gemini -p`, `agy --print`), which would defeat one half of the requirement.

A runner listed in `supports` but not installed is shown disabled with a reason,
never hidden. The user should never wonder why an action vanished.

`cohors agent run <what>` performs these invocations, and `cohors agent
command <what>` prints the one it would perform so an editor can type it into
its own terminal. Two safety rules apply to every runner:

- The prompt is one argument and must never be read as an option, so a
  prompt whose first character is `-` gets a fixed `Prompt:` line in front.
- Where the shell cannot pass a file's contents as exactly one argument
  (PowerShell), the prompt text is not put on a command line at all: cohors
  reads the file and starts the runner itself.

A front-end that starts a repo-declared agent or task must show the person the
exact text cohors resolved (the `prompt` field of `cohors agent command`)
before running it, and run nothing else.

---

## 5. What the launchpad does without any of this

Zero-config behaviour, so a fresh install is useful immediately:

1. Scans the user's configured roots (or the ones passed with `--root`).
2. Keeps every directory that is a git repository.
3. Reads branch, uncommitted work, stashes, and ahead/behind for each.
4. **Auto-detects flows** without any config:
   - `package.json` scripts (`dev`, `start`, `build`, `test`, `lint`)
   - `Cargo.toml` → `cargo check|build|test|run`
   - `pyproject.toml` / `uv.lock` → `uv sync`, `uv run pytest`
   - `Makefile` targets
   - `settings.gradle[.kts]` → `./gradlew assembleDebug`
5. Shows every discovered repo, including unconfigured ones, each with a
   **"Set up with an agent"** action.

Auto-detected flows are marked `"source": "detected"` in memory and are never
written to disk. Committing them is the agent's job, and only when it adds
something detection could not infer.

---

## 6. `~/.cohors/board.json`

Cross-repo focus. Optional; the dashboard works without it.

```jsonc
{
  "specVersion": 1,
  "updated": "2026-09-06T12:00:00Z",
  "focus": [
    {
      "id": "f-1",
      "title": "Ship billing retries",
      "repo": "billing-service",     // matches repo.json name, not a path
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

## 6b. `<repo>/.cohors/local.json`

Machine-local, gitignored, written by cohors. Never commit it.

```jsonc
{
  "specVersion": 1,
  "updated": "2026-09-07T12:00:00Z",
  "threads": {
    "billing-retries": "00000000-0000-0000-0000-000000000000"
  }
}
```

`threads` maps a task id to the agent session that task is being worked in, so
the task's action reopens that conversation instead of starting a cold one. It
lives here and **not** in `repo.json` for a specific reason: a session id names
a transcript on one machine. Committed, it would hand a teammate a pointer to a
conversation that does not exist for them.

The cross-repo board carries the same idea in its own `thread` field (§6);
that file is user-level and never committed, so it needs no separate home.

A launchpad links a thread by choosing the session id **before** launching,
where the runner allows it (`claude --session-id`, `gemini --session-id`). A
runner without that flag simply starts unlinked — the id is never guessed by
matching transcripts after the fact.

---

## 7. Staleness

Anything cached must be able to prove it is current, or say that it is not.

- Every generated file carries `"updated"` as an ISO-8601 UTC timestamp.
- `repo.json` also carries an optional `"generatedFrom"`: the commit SHA the
  agent read when writing it. The dashboard shows a "may be out of date" hint
  when `HEAD` has moved far past it.
- The dashboard never displays a cached value without also being able to show its
  age.

---

## 8. Validating

```bash
cohors launchpad validate <path-to-repo>
```

Checks `repo.json` against this contract (the JSON Schema beside this file
describes the same shape): every `agents[].file` exists, ids are unique and
kebab-case, no `run` or `cwd` is absolute, every `run` is plain text as §3
requires, and `specVersion` is supported. Every problem is reported at once.
The MCP tool `write_repo_contribution` runs the same checks before it writes
and writes nothing when one fails. Agents should run this
before reporting success.

---

## 9. Versioning

`specVersion` is an integer. cohors supports the current version and one
back. When it reads an older file it migrates in memory and writes the new shape
only when something else changes the file. It never rewrites a repo's committed
config just to bump a version.
