---
name: fleet
description: >
  Answer questions about ALL of the user's git repositories at once using the
  cohors tools. Use when the user asks about "my repos", "my projects", "the
  fleet", what is dirty / unpushed / behind / failing CI / needs attention,
  where a repo lives, or wants to find or change something across several
  repos.
---

# Work across the whole fleet

The `cohors` MCP server already knows every repository under the user's
configured roots. Use it instead of `find`, `cd`, or per-repo `git`, and do not
ask the user where their repos are.

## Pick the cheapest tool that answers the question

| The user wants | Call |
| --- | --- |
| "Is anything on fire?" | `fleet_summary` |
| Status of many repos | `list_repos` (narrow with a selector, shrink with `fields`) |
| One repo in depth | `get_repo` |
| What is uncommitted, and the diff | `changes` |
| Code or repos matching something | `search` |
| Open PRs / red CI | `list_prs`, `ci_status` |
| Where a repo is on disk | `repo_path` |
| What to work on, how to run things | `launchpad`, `read_board` |

## Selectors

Every tool that targets repos takes the same selector object. Fields AND
together: `{"dirty": true}`, `{"behind": true}`, `{"name": "pay*"}`,
`{"ci": "failing"}`, `{"any_of": [{"dirty": true}, {"ahead": true}]}`.

The empty selector `{}` matches **nothing** on purpose. Pass `{"all": true}`
when you really mean every repo.

## Acting across repos

`fetch`, `pull`, `push`, `commit`, `stash` and `run` are off unless the user
started the server with `--allow-writes` / `--allow-run`. When one is refused,
tell the user which flag enables it; do not fall back to a shell loop.

Always follow **find → target → preview → act**:

1. Find the repos (`search`, or `list_repos` with a selector).
2. Call the action with `dry_run: true` and show the user the target list.
3. Only after they agree, call it again with `confirm: true`.

`pull` is fast-forward only and `push` never forces, so neither can lose work.

## Reading results honestly

Every read carries a `meta` block: the roots searched, the config in effect,
and whether a GitHub token is present. If `total` is 0 or a `note` is present,
report that rather than saying everything is clean.
