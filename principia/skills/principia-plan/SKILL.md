---
description: >
  Plan focus across every repository Principia knows about by looking at
  uncommitted work, unpushed branches, and repo-declared tasks, then write
  ~/.principia/board.json. Use when the user asks to "plan my day", "plan my
  focus", "what should I work on", or when the launchpad's Today/Setup tab
  reports no board.
---

# Plan focus across repos

Follow `prompts/plan-day.md` in the Principia repository (or the copy bundled
with this plugin at `${CLAUDE_PLUGIN_ROOT}/prompts/plan-day.md`) exactly.

If the `principia` MCP server is connected, prefer its tools over raw file
edits:

- `list_repos` for the current state of every discovered repository
- `read_board` before writing, so you carry forward work still in progress
- `write_board` to save the result — it validates before writing and reports
  exactly what is wrong if it rejects your input, so fix and retry rather than
  editing the file directly
- `record_activity` when you finish, so the cross-runner timeline picks it up

Without the MCP server, read `~/.principia/board.json` and
`spec/v1/board.schema.json` directly, and validate by hand against the schema
before writing.

Never invent progress. Only mark something `"status": "done"` with evidence:
a commit, a merged PR, or the user saying so.
