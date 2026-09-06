<div align="center">

# principia

**The launchpad your repositories write for themselves.**

A VS Code surface that ships empty and fills up as each repository describes what
it offers: its flows, its agents, its work. Nothing to configure by hand, because
an AI agent sitting inside each repo writes the contribution for it.

<sub><i>principia</i> was the headquarters at the centre of a Roman fort: where the
standards were kept and the orders went out.</sub>

</div>

---

## The problem

Every dashboard you have tried wanted you to configure it. So you configured it
once, it went stale, and you stopped opening it.

Principia inverts that. The dashboard holds no config. **Each repository owns its
own entry**, committed alongside its code, so it travels with the clone and stays
next to the thing it describes. And you do not write it: you ask an agent, in
that repo, and it reads the code and writes the entry.

## How it works

**1. It is useful before you configure anything.** On install it reads your
editor's own recently-opened list, keeps the entries that are git repositories,
and auto-detects what it can run: npm scripts, cargo commands, Makefile targets,
gradle tasks, uv and pytest. Unconfigured repos are shown, never hidden, each
with a one-click setup.

**2. A repo opts in by committing `.principia/`.**

```
your-repo/
└── .principia/
    ├── repo.json          # flows, agents, tasks this repo offers
    ├── agents/triage.md   # prompts this repo ships
    └── local.json         # machine-local, gitignored
```

**3. An agent writes it.** In that repo, ask any supported runner:

> Set up this repo for the Principia launchpad.

It reads the code and writes the file. The launchpad picks it up on save.

**4. Any runner, not just one.** The instructions live in `prompts/`, which is
runner-agnostic. Claude Code gets a plugin with a skill and an agent; Codex,
Gemini CLI and agy read the same prompt files. A runner you have not installed is
shown disabled with a reason, never silently hidden.

## The contract

[`spec/v1/SPEC.md`](spec/v1/SPEC.md) is authoritative and is what agents read.
JSON Schemas sit beside it, and a zero-dependency validator checks any repo:

```sh
node scripts/validate.js .
```

It enforces the rules that matter: kebab-case stable ids, no absolute paths, no
secrets in a committed file, one primary flow, and referenced agent files that
actually exist.

## Design rules

- **Ships empty.** No user data in the extension. A fresh install on a new
  machine behaves exactly like a stranger's first run.
- **Committed, not hidden.** A repo's entry belongs in the repo, so a teammate or
  your second machine gets it for free. Machine-local values are quarantined in a
  gitignored file.
- **Never silently stale.** Every generated file carries `updated`, and
  `repo.json` records the commit it was written from. Cached values are shown
  with their age or not at all.
- **Detection first, config second.** An agent should not write down what the
  launchpad can already infer. An empty `flows` array is a valid answer.

## MCP server

`mcp/` is a real MCP server (stdio transport) giving any MCP-capable agent the
same discovery, validation, and write capabilities the VS Code extension's
buttons use: `list_repos`, `repo_status`, `read_contract`,
`write_repo_contribution`, `read_board`, `write_board`, `record_activity`.
Writes always validate against the contract first and reject with a specific
error list rather than writing an invalid file. It ships wired into the
Claude Code plugin (`.mcp.json`), so installing the plugin registers it
automatically; any other MCP client can point at
`node mcp/src/server.js` directly.

## Installing

```sh
cd cli && npm install -g .
principia status        # read-only: what would happen, and why not otherwise
principia init           # do it
```

`init` detects your agent runners, registers the Claude Code plugin
(marketplace + install, from this checkout), writes project-local skill files
as a fallback that works even without the plugin, points other runners
(Codex, Gemini CLI, agy) at the contract via `AGENTS.md`, sets up
`~/.principia/`, offers the VS Code/Cursor extension as a dev symlink, and
gitignores `.principia/local.json`. Every step checks its own prerequisites
first and is skipped (not fatal) if unmet; re-running is safe; nothing
outside `.principia/`, `.claude/`, `AGENTS.md`, and `.gitignore` in the target
repo, plus `~/.principia/` and the plugin/extension registration, is ever
touched.

## Status

Early. The contract, validator, plugin, MCP server, CLI, prompts and templates
are in place and tested end to end (including a real stdio MCP handshake and
a full `init` run against an isolated scratch environment). Not yet published
anywhere; install locally per above.

## Related

[cohors](https://github.com/rushirb2001/cohors) is the fleet control plane: every
repository's git state, in one place, for you and your agent. Principia is about
what you *do* next, not what the fleet's state *is*. When cohors is installed,
Principia can use it as a faster discovery engine; it is never required.

## License

MIT
