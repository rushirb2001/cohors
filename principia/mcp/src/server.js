#!/usr/bin/env node
// Principia MCP server. Gives any MCP-capable agent (Claude Code, Cursor,
// or anything else that speaks MCP) the same discovery/write/validate
// capabilities the VS Code extension's buttons use, as real tool calls
// instead of "write a file and hope the shape is right".
//
// Transport: stdio. Registered per-agent (Claude Code via `claude mcp add`
// or the plugin's mcpServers block; other clients via their own MCP config).

const { McpServer } = require("@modelcontextprotocol/sdk/server/mcp.js");
const { StdioServerTransport } = require("@modelcontextprotocol/sdk/server/stdio.js");
const { z } = require("zod");
const path = require("path");
const L = require("./lib");

// Server-level orientation. An agent reaching Principia through MCP alone may
// never have seen prompts/ or the spec, so the workflow and the judgment calls
// that make a contribution good rather than merely valid are stated here.
const INSTRUCTIONS = `
Principia turns a repository into an entry on a launchpad. The repository
describes itself in a committed .principia/repo.json; the launchpad holds no
configuration of its own.

Typical order of work:
  1. list_repos            — what exists, and which repos are already configured
  2. read_contract         — the exact rules a contribution must satisfy
  3. repo_status           — live git state for the repo you are about to touch
  4. write_repo_contribution / write_board — validated writes

What makes a contribution good, not just valid:

- DETECTION FIRST. npm/pnpm/yarn/bun scripts, cargo, uv/pytest, Makefile
  targets and gradle tasks are already detected with zero config and shown
  under Scripts. Declare a flow only when it adds what detection cannot infer:
  a label a human recognises, a port, which single flow is primary, a
  composite command, or the right cwd in a monorepo. An empty flows array is a
  correct answer, and a better one than five flows restating package.json.
- SHIP AN AGENT ONLY FOR RECURRING, REPO-SPECIFIC WORK. "Explain this code" is
  not one. Most repositories need none.
- TASKS ARE SHORT-LIVED AND CONCRETE, drawn from evidence: uncommitted work,
  unpushed commits, TODOs that name a bounded change, failing tests. A real
  backlog belongs in an issue tracker.
- NEVER MARK ANYTHING done WITHOUT EVIDENCE: a commit, a merged PR, or the
  user saying so.
- NOTHING MACHINE-SPECIFIC. repo.json is committed, so no absolute paths, no
  secrets, no hostnames, no chosen ports, no session ids. Machine-local values
  belong in the gitignored .principia/local.json.
- WRITES REPLACE. write_repo_contribution writes the whole document, so read
  the current one from list_repos first and pass back everything you intend to
  keep — especially tasks the user may be part-way through.

Every write validates first and writes nothing on failure, returning the exact
errors to fix. Prefer these tools over editing the JSON by hand.
`.trim();

const server = new McpServer(
  { name: "principia", version: "0.1.0" },
  { instructions: INSTRUCTIONS }
);

const text = (obj) => ({ content: [{ type: "text", text: JSON.stringify(obj, null, 2) }] });
const errText = (msg) => ({ content: [{ type: "text", text: msg }], isError: true });

// A repo path arriving from a tool call is agent-supplied input, exactly like
// a value from an untrusted request body. Constrain it before touching disk:
// must be an absolute path to a directory that actually contains a .git dir.
function resolveRepoRoot(root) {
  if (typeof root !== "string" || !path.isAbsolute(root)) return null;
  const resolved = path.resolve(root);
  if (!L.exists(path.join(resolved, ".git"))) return null;
  return resolved;
}

server.registerTool(
  "list_repos",
  {
    title: "List repositories",
    description:
      "List git repositories from the editor's recently-opened history, each with live branch/dirty/ahead-behind status and its .principia/ contribution if one exists. This is the same discovery Launch pad's Repos tab uses.",
    inputSchema: { maxRepos: z.number().int().min(1).max(200).optional() },
  },
  async ({ maxRepos }) => {
    const repos = await L.listRepos({ maxRepos });
    return text({ count: repos.length, repos });
  }
);

server.registerTool(
  "repo_status",
  {
    title: "Get one repo's git status",
    description: "Branch, dirty file count, ahead/behind vs upstream, and the last commit for a single repository.",
    inputSchema: { root: z.string().describe("Absolute path to the repository") },
  },
  async ({ root }) => {
    const r = resolveRepoRoot(root);
    if (!r) return errText(`Not a git repository: ${root}`);
    return text(await L.gitInfo(r));
  }
);

server.registerTool(
  "read_contract",
  {
    title: "Read the Principia contract",
    description:
      "Returns the spec version this server implements and the validation rules a .principia/repo.json must satisfy, so an agent does not have to guess the schema.",
    inputSchema: {},
  },
  async () => text({
    specVersion: 1,
    rules: [
      "name is required; summary is at most 80 characters",
      "every id (flows/agents/tasks) is unique and kebab-case",
      "flows[].run and flows[].cwd must be relative, never absolute",
      "at most one flow may be primary",
      "agents[].file must live under .principia/ and must exist",
      "agents[].supports may only contain: claude-code, codex, gemini-cli, agy",
      "tasks[].status is one of: todo, doing, blocked, done",
      "tasks[].priority is one of: high, normal, low",
      "tasks[].agent, if set, is kebab-case and should name one of this repo's own agents[] ids",
      "no absolute paths, secrets, or machine-specific values anywhere in this file — it is committed",
    ],
    judgement: [
      "Detection already covers npm/cargo/uv/Makefile/gradle with zero config; declare a flow only when it adds a label, a port, an ordering, a composite command or a cwd that detection cannot infer",
      "An empty flows array is a correct answer, and better than flows that restate package.json",
      "Add an agent only for recurring, repo-specific work; most repositories need none",
      "Tasks come from evidence (uncommitted work, unpushed commits, real TODOs, failing tests), never from invention",
      "Never set status done without a commit, a merged PR, or the user saying so",
    ],
    machineLocal: {
      file: ".principia/local.json",
      gitignored: true,
      purpose: "Values that are true only on this machine, so they must never be committed. `threads` maps a task id to the agent session it is being worked in, so the launchpad can reopen that conversation instead of starting cold.",
    },
    note: "write_repo_contribution validates against exactly these rules before writing anything. A failed validation writes nothing, and replaces the whole document on success — read the current one first.",
  })
);

server.registerTool(
  "write_repo_contribution",
  {
    title: "Write a repo's .principia/repo.json",
    description:
      "Validate and write this repository's contribution: its flows, agents and tasks. " +
      "REPLACES the whole document — read the current one from list_repos first and pass back " +
      "everything you mean to keep, especially tasks the user may be part-way through. " +
      "Rejects and writes nothing if invalid; the errors say exactly what to fix. " +
      "Declare a flow only when it adds what detection cannot infer (label, port, primary, " +
      "composite command, cwd) — an empty flows array is a correct answer. " +
      "Never set a task done without evidence. Nothing machine-specific: this file is committed. " +
      "Also ensures .principia/local.json is gitignored.",
    inputSchema: {
      root: z.string().describe("Absolute path to the repository"),
      name: z.string(),
      summary: z.string().max(80).optional().describe("One line, what this repo IS"),
      group: z.string().optional().describe("Free text; groups repos in the dashboard"),
      icon: z.string().optional().describe("codicon name; omit to auto-pick by stack"),
      generatedFrom: z.string().optional()
        .describe("Short SHA of the commit you read when writing this, so staleness can be shown"),
      links: z.array(z.object({
        label: z.string(), url: z.string(),
      })).optional().describe("Docs, dashboards, issue trackers"),
      flows: z.array(z.object({
        id: z.string(), label: z.string(), run: z.string(),
        cwd: z.string().optional(), kind: z.enum(["server", "task", "test", "build", "tool"]).optional(),
        port: z.number().int().optional(), background: z.boolean().optional(), primary: z.boolean().optional(),
      })).optional().describe("Only what detection cannot infer; at most one primary"),
      agents: z.array(z.object({
        id: z.string(), label: z.string(), file: z.string(),
        supports: z.array(z.enum(["claude-code", "codex", "gemini-cli", "agy"])).optional(),
      })).optional().describe("Prompt files under .principia/agents/ that must already exist"),
      tasks: z.array(z.object({
        id: z.string(), title: z.string(),
        status: z.enum(["todo", "doing", "blocked", "done"]).optional(),
        priority: z.enum(["high", "normal", "low"]).optional(),
        notes: z.string().max(400).optional(),
        agent: z.string().optional()
          .describe("id of one of this repo's agents[]; the task's action then runs that agent"),
      })).optional().describe("Short-lived, evidence-backed work; a real backlog belongs elsewhere"),
    },
  },
  async ({ root, ...doc }) => {
    const r = resolveRepoRoot(root);
    if (!r) return errText(`Not a git repository: ${root}`);
    const result = L.writeRepoContribution(r, doc);
    if (!result.written) return errText(`Not written. Fix these and retry:\n` + result.errors.map((e) => `  - ${e}`).join("\n"));
    return text(result);
  }
);

server.registerTool(
  "read_board",
  {
    title: "Read the cross-repo focus board",
    description: "Read ~/.principia/board.json: the user's current focus items across every repo.",
    inputSchema: {},
  },
  async () => text(L.readBoard())
);

server.registerTool(
  "write_board",
  {
    title: "Write the cross-repo focus board",
    description:
      "Validate and write ~/.principia/board.json. Match `repo` by name (as it appears in list_repos), never by absolute path, so the board stays valid across machines. Never set status \"done\" without evidence.",
    inputSchema: {
      focus: z.array(z.object({
        id: z.string(), title: z.string(),
        repo: z.string().optional(), ref: z.string().optional(),
        status: z.enum(["todo", "doing", "blocked", "done"]).optional(),
        agent: z.string().optional(), thread: z.string().optional(),
      })),
    },
  },
  async ({ focus }) => {
    const result = L.writeBoard({ focus });
    if (!result.written) return errText(`Not written. Fix these and retry:\n` + result.errors.map((e) => `  - ${e}`).join("\n"));
    return text(result);
  }
);

server.registerTool(
  "record_activity",
  {
    title: "Record an agent activity event",
    description:
      "Append one event to ~/.principia/history/. Use this when working through this MCP server so Launch pad's Activity tab and cross-runner timeline see it, the same as Claude Code's own SessionStart/SessionEnd hook does automatically.",
    inputSchema: {
      runner: z.enum(["claude-code", "codex", "gemini-cli", "agy"]),
      phase: z.enum(["start", "end", "launch", "note"]),
      repo: z.string().optional().describe("Absolute path to the repository"),
      branch: z.string().optional(),
      agent: z.string().optional().describe("Which repo-declared or plugin agent, if any"),
    },
  },
  async (args) => text(L.recordActivity(args))
);

async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
}

main().catch((err) => {
  console.error("principia-mcp failed to start:", err);
  process.exit(1);
});
