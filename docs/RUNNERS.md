# Runner support

Principia is runner-agnostic by design. The instructions live once, in
`prompts/`, and each runner is a thin invocation over the same file.

| Runner | Detection | Invocation |
| --- | --- | --- |
| `claude-code` | `claude` on PATH | plugin skill `/principia:principia-setup`, or `claude --append-system-prompt "$(cat prompts/setup-repo.md)"` |
| `codex` | `codex` on PATH | `codex --prompt-file prompts/setup-repo.md` |
| `gemini-cli` | `gemini` on PATH | `gemini -p "$(cat prompts/setup-repo.md)"` |
| `agy` | `agy` on PATH | `agy run --prompt prompts/setup-repo.md` |

## Rules

1. **A missing runner is shown, disabled, with a reason.** Never hide an action
   because a binary is absent; the user should not have to guess why a button
   vanished.
2. **No runner is privileged in the contract.** Claude Code gets a plugin because
   the format exists, not because the spec favours it. A repo that lists
   `supports: [codex]` is exactly as valid.
3. **Flags change; the prompt does not.** When a runner changes its CLI, only the
   table above changes. `prompts/` is the stable surface.

## Adding a runner

1. Add its id to the `supports` enum in `spec/v1/repo.schema.json`.
2. Add its id to `RUNNERS` in `scripts/validate.js`.
3. Add a row above with its detection binary and invocation.
4. Add the mapping to the extension's runner registry.

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
