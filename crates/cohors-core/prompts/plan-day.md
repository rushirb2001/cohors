# Plan focus across repositories

Runner-agnostic. Writes the focus board, `~/.principia/board.json`, as the
contract (`cohors launchpad contract`) describes it.

## Gather evidence first

Do not plan from memory. Look at:

1. **Uncommitted and unpushed work.** `cohors overview --json` lists every
   repo with its state, the reasons it needs attention, and what to do next.
   (Without cohors on the PATH: `git -C <repo> status --porcelain` and
   `git -C <repo> rev-list --count @{upstream}..HEAD` per repo.)
2. **What was committed recently.** `cohors standup --json`.
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

If the `cohors` MCP server is available, use `overview` (what is at risk
and what to do next), `launchpad` (every repo's tasks) and `list_repos` to
gather state, and `write_board` to save the result: it validates the board
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
