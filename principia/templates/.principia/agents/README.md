# Repo agents

One markdown file per agent, with YAML frontmatter. See `spec/v1/SPEC.md` §4.

These are prompts, not code. They are runner-agnostic: the launchpad maps them
onto whichever of Claude Code, Codex, Gemini CLI or agy the user has installed.

Placeholders substituted at launch: `{{REPO_NAME}}`, `{{REPO_ROOT}}`,
`{{BRANCH}}`, `{{TASK_TITLE}}`.
