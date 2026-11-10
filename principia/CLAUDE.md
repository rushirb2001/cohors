# CLAUDE.md — working agreement for principia

## What this is

A VS Code launchpad that ships empty and is populated by repositories describing
themselves. Two halves in one repo:

- **The contract** (`spec/`, `prompts/`, `scripts/validate.js`, `templates/`) —
  runner-agnostic, MIT, the part other people's agents read.
- **The extension** (`extension/`) — the VS Code surface.

Plus a Claude Code plugin (`.claude-plugin/`, `skills/`, `agents/`, `hooks/`).

## Read before coding

1. `spec/v1/SPEC.md` — authoritative. If code and spec disagree, the spec wins.
2. `prompts/setup-repo.md` — what agents actually follow.
3. `docs/RUNNERS.md` — how runners are invoked and added.

## Hard rules

- **Ship empty.** No personal paths, repo names, or user data anywhere in the
  extension or the plugin. This is distributed software. The maintainer's own
  repos must be discovered at runtime like anyone else's.
- **Detection before configuration.** If the launchpad can infer it, do not ask an
  agent to write it down. Auto-detected flows are marked `source: "detected"` and
  never persisted.
- **Committed files carry nothing machine-specific.** No absolute paths, no
  hostnames, no ports the user chose, no device ids. Those go in the gitignored
  `.principia/local.json`.
- **Nothing is shown without its age.** If a value is cached, the UI can state how
  old it is, or it does not display it.
- **No runner is privileged.** Claude Code has a plugin because the format exists.
  The spec treats all runners equally.
- **The webview never rebuilds wholesale.** Render the shell once and patch. A
  full `innerHTML` swap on a refresh destroys scroll, focus and caret. This was a
  real bug; do not reintroduce it.

## Quality gates

```sh
node --check scripts/validate.js
node scripts/validate.js .              # self-check
node scripts/validate.js <some-repo>    # against a real repo
node scripts/check-runners.js           # every runner list still agrees
```

Every JSON file must parse. Every schema must be valid JSON Schema 2020-12.

## Performance budget

Target: a 16 GB laptop with a dozen editors open. Discovery reads the editor's
recents list, not the filesystem. Git is only run for repos actually shown.
Nothing polls while the tab is unfocused.

## Commit style

Single subject line, imperative, no body. No AI attribution trailers.
