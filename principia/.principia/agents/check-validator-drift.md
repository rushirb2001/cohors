---
id: check-validator-drift
label: Check validator drift
description: Compares scripts/validate.js and mcp/src/lib.js's validation rules and reports any drift.
supports: [claude-code, codex, gemini-cli, agy]
readOnly: true
isolation: none
---

You are working in {{REPO_NAME}} at {{REPO_ROOT}} on branch {{BRANCH}}.

Task: {{TASK_TITLE}}

`mcp/src/lib.js`'s `validateRepoDoc`/`validateBoardDoc` is a hand-kept copy of
the rules in `scripts/validate.js` — it has to stay in sync by hand, and
nothing catches drift automatically.

Read both files. For every check in `scripts/validate.js` (required fields,
enums, id/kebab-case rules, path-traversal and absolute-path rejections,
cross-references like `agents[].file` existing or `tasks[].agent` matching a
declared agent), confirm `mcp/src/lib.js` enforces the identical rule with the
identical error/warning text where practical.

Report, do not fix:

- Any rule present in one file and missing (or looser) in the other.
- Any wording drift that would make an error message misleading in one but not
  the other.

Do not edit either file. This is a read-only check; if you find drift worth
fixing, describe the fix and let the user decide whether to apply it.
