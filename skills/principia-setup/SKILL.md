---
description: >
  Teach the current repository to describe itself to the Principia launchpad by
  writing .principia/repo.json and any repo-specific agent prompts. Use when the
  user asks to "set up this repo for the launchpad", "add this repo to
  Principia", "configure principia here", or when the launchpad reports this
  repo as unconfigured.
---

# Set up this repo for Principia

Follow `prompts/setup-repo.md` in the Principia repository. It is the canonical,
runner-agnostic instruction and this skill adds nothing to it.

If you cannot locate the Principia repo on disk, read the copy bundled with this
plugin at `${CLAUDE_PLUGIN_ROOT}/prompts/setup-repo.md`.

Two reminders that are worth repeating because they are the usual failure modes:

1. **Do not restate auto-detection.** The launchpad already finds npm scripts,
   cargo commands, Makefile targets, gradle tasks and pytest with zero config.
   Only write a flow that adds a real label, a port, an ordering, a composite
   command, or a monorepo `cwd`.
2. **An empty `flows` array is a valid, good answer** when the repo has nothing
   beyond detection. Say so in your report rather than padding the file.

Always finish by running the validator and fixing every error:

```sh
node <principia>/scripts/validate.js .
```
