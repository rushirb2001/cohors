# Runner support

Principia is runner-agnostic by design. The instructions live once, in
`prompts/`, and each runner is a thin invocation over the same file.

## The prompts

Each is a focused pass the launchpad can hand to any runner. The extension
resolves them by **id** against an allowlist, so a webview message never names a
file path.

| Id | File | Asks the agent to |
| --- | --- | --- |
| `setup-repo` | `prompts/setup-repo.md` | read the repo and write its whole `.principia/` contribution |
| `declare-flows` | `prompts/declare-flows.md` | decide which flows are worth declaring over detection |
| `add-agent` | `prompts/add-agent.md` | decide whether the repo needs a committed agent, and write it |
| `track-tasks` | `prompts/track-tasks.md` | find real in-flight work and record it as tasks |
| `plan-day` | `prompts/plan-day.md` | plan focus across every repo and write the board |

A prompt is **committed**, so it must never contain a machine-specific path. Use
the placeholders in `spec/v1/SPEC.md` §4 — `{{CONTRACT_ROOT}}`, `{{SPEC}}`,
`{{VALIDATE}}`, `{{REPO_ROOT}}` — which are resolved into the throwaway copy
handed to the runner.

| Runner | Detection | Invocation |
| --- | --- | --- |
| `claude-code` | `claude` on PATH | plugin skill `/principia:principia-setup`, or `claude "$(cat prompts/setup-repo.md)"` |
| `codex` | `codex` on PATH | `codex "$(cat prompts/setup-repo.md)"` |
| `gemini-cli` | `gemini` on PATH | `gemini -i "$(cat prompts/setup-repo.md)"` |
| `agy` | `agy` on PATH | `agy --prompt-interactive "$(cat prompts/setup-repo.md)"` |

## Rules

1. **A missing runner is shown, disabled, with a reason.** Never hide an action
   because a binary is absent; the user should not have to guess why a button
   vanished.
2. **No runner is privileged in the contract.** Claude Code gets a plugin because
   the format exists, not because the spec favours it. A repo that lists
   `supports: [codex]` is exactly as valid.
3. **Flags change; the prompt does not.** When a runner changes its CLI, only the
   table above changes. `prompts/` is the stable surface.
4. **An invocation must submit the prompt *and* stay interactive.** Passing the
   text as a system prompt (`claude --append-system-prompt`) opens a session
   that was never asked anything, so the agent just waits. Passing it to a
   headless flag (`claude -p`, `gemini -p`, `agy --print`) runs once and exits,
   so the user cannot follow up. Use the positional prompt, or the runner's
   explicit "prompt then stay interactive" flag.

## Adding a runner

The implementation lives once, in `shared/runners.js`; `extension/src/runners.js`
and `cli/src/runners.js` are one-line re-exports of it. The other places below
name the runner set again on purpose — a JSON Schema cannot import code, and the
validator and MCP server are deliberately dependency-free.

1. Add the row to `RUNNERS` in `shared/runners.js`, with its `command`, and
   `startWith`/`resume` if the CLI genuinely supports them (`null` if not).
2. Add its id to the `supports` enum in `spec/v1/repo.schema.json`.
3. Add its id to `RUNNERS` in `scripts/validate.js` and in `mcp/src/lib.js`,
   and to the `z.enum` lists in `mcp/src/server.js`.
4. Add a row to the table above.
5. Run `node scripts/check-runners.js` — it fails if any of those lists
   disagree, which is the mistake this sequence invites.

No prompt changes. If a new runner needs different instructions, that is a signal
the prompt has drifted toward one runner's idioms and should be generalised.

## History

Every runner writes session events to `~/.principia/history/YYYY-MM-DD.jsonl`:

```json
{"ts":"2026-09-06T12:00:00Z","phase":"start","runner":"claude-code","repo":"/path","branch":"main","session":"abc"}
```

Claude Code does this automatically via `hooks/hooks.json`. Other runners are
wrapped by the extension, which writes the same record when it launches them.
That is what makes a unified cross-runner timeline possible: one shape, one log,
regardless of who did the work.
