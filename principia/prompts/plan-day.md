# Plan focus across repositories

Runner-agnostic. Writes `~/.principia/board.json` per `{{SPEC}}` §6.

## Gather evidence first

Do not plan from memory. Look at:

1. **Uncommitted work.** For each repo the launchpad knows about:
   `git -C <repo> status --porcelain` and how long it has been dirty.
2. **Unpushed work.** `git -C <repo> rev-list --count @{upstream}..HEAD`.
3. **Repo-declared tasks.** `.principia/repo.json` → `tasks[]` with status
   `todo`, `doing` or `blocked`.
4. **Recent sessions.** `~/.principia/history/*.jsonl`, newest first, to see what
   was actually being worked on and by which runner.

## Then write the board

- Carry forward anything still `doing`. Do not silently drop in-flight work.
- Remove focus entries whose underlying task is now `done`.
- Add entries only for work you can point at evidence for.
- Where a repo ships a relevant agent, set `agent` so the item is one click from
  running it.
- Where a prior session exists for the same work, set `thread` so the item
  resumes rather than starting cold.

## Prefer the MCP server when it is connected

If the `principia` MCP server is available, use `list_repos` and `repo_status`
to gather state and `write_board` to save the result: it validates the board
before writing. `read_board` gives you the current one so you can report what
changed. Fall back to reading and writing the file directly only when it is not
connected.

## Rules

- **Never invent progress.** Only `done` with evidence: a commit, a merged PR, or
  the user saying so.
- Do not cap the list artificially, but order it so the top item is genuinely the
  next thing to do.
- Set `updated` to now, in ISO-8601 UTC.
- Report what changed since the last board, not just the new state.
