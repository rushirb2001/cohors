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

const server = new McpServer({ name: "principia", version: "0.1.0" });

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
      "no absolute paths, secrets, or machine-specific values anywhere in this file — it is committed",
    ],
    note: "write_repo_contribution validates against exactly these rules before writing anything. A failed validation writes nothing.",
  })
);

server.registerTool(
  "write_repo_contribution",
  {
    title: "Write a repo's .principia/repo.json",
    description:
      "Validate and write this repository's contribution to the Principia launchpad: its flows, agents, and tasks. Rejects and writes nothing if the document is invalid; the errors array explains exactly what to fix. Also ensures .principia/local.json is gitignored.",
    inputSchema: {
      root: z.string().describe("Absolute path to the repository"),
      name: z.string(),
      summary: z.string().max(80).optional(),
      group: z.string().optional(),
      icon: z.string().optional(),
      flows: z.array(z.object({
        id: z.string(), label: z.string(), run: z.string(),
        cwd: z.string().optional(), kind: z.enum(["server", "task", "test", "build", "tool"]).optional(),
        port: z.number().int().optional(), background: z.boolean().optional(), primary: z.boolean().optional(),
      })).optional(),
      agents: z.array(z.object({
        id: z.string(), label: z.string(), file: z.string(),
        supports: z.array(z.enum(["claude-code", "codex", "gemini-cli", "agy"])).optional(),
      })).optional(),
      tasks: z.array(z.object({
        id: z.string(), title: z.string(),
        status: z.enum(["todo", "doing", "blocked", "done"]).optional(),
        priority: z.enum(["high", "normal", "low"]).optional(),
        notes: z.string().max(400).optional(),
      })).optional(),
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
