// The gate in front of anything a repository supplies and the editor runs.
//
// A flow is a command line. It may be declared in a repository's launchpad
// file or detected from its manifests (an npm script, a Makefile target, a
// gradle task); either way the repository's author wrote it. A declared agent
// is a prompt file the repository committed, and a task's title and notes
// become an agent's prompt. All of that is somebody else's code or text.
//
// Two conditions must hold before any of it runs:
//
//   1. The window is trusted (VS Code Workspace Trust).
//   2. The user has seen exactly what will run, and where, and said yes.
//      The dashboard lists repositories other than the folder open in this
//      window, so the window's trust says nothing about them. Approval is
//      remembered per repository root and per command, for this session
//      only: a command that has not been shown is never run, and nothing is
//      written to disk.
//
// cohors's own prompts (setup-repo, plan-day, …) and the bulk git verbs are
// cohors's commands, not a repository's, and do not pass through here.
//
// No `vscode` in this file: the host passes in how to read trust and how to
// ask, so the rules can be checked from plain Node (dev/check.js).

const path = require("path");

/**
 * @param {{ isTrusted: () => boolean,
 *           confirm: (request: {message: string, detail: string, action: string}) => Promise<boolean> }} host
 */
function create(host) {
  const approved = new Map();   // repository root -> Set of approved keys

  /**
   * @param {{repo: string, key: string, kind: "flow"|"agent"|"task", message: string, detail: string, action?: string}} request
   * @returns {Promise<{ok: true, asked: boolean} | {ok: false, reason: "untrusted"|"declined", message?: string}>}
   */
  async function permit(request) {
    if (!host.isTrusted()) {
      const what = request.kind === "flow" ? "flows" : request.kind === "agent" ? "agents declared by a repository" : "tasks declared by a repository";
      return { ok: false, reason: "untrusted", message: `This workspace is not trusted, so ${what} are disabled.` };
    }
    const seen = approved.get(request.repo);
    if (seen && seen.has(request.key)) return { ok: true, asked: false };

    const yes = await host.confirm({ message: request.message, detail: request.detail, action: request.action || "Run" });
    if (!yes) return { ok: false, reason: "declined" };
    if (!approved.has(request.repo)) approved.set(request.repo, new Set());
    approved.get(request.repo).add(request.key);
    return { ok: true, asked: true };
  }

  /**
   * Run one of a repository's flows, declared or detected, through the gate.
   * `run(cwd, command, name)` is only called once the window is trusted and
   * this exact command has been shown and approved for this repository.
   *
   * @param {{repo: {path: string, name: string}, entry: object, flowId: string,
   *          run: (cwd: string, command: string, name: string) => void}} o
   */
  async function flow(o) {
    const found = findFlow(o.entry, o.flowId);
    if (!found) return { ok: false, reason: "unknown", message: "That flow is no longer listed for this repository." };
    const cwd = flowCwd(o.repo.path, found.flow);
    if (!cwd) return { ok: false, reason: "outside", message: `Refusing to run "${found.flow.label || found.flow.id}" outside its repository.` };
    const allowed = await permit(flowRequest(o.repo.path, found.flow, found.declared, cwd));
    if (!allowed.ok) return allowed;
    o.run(cwd, found.flow.run, `${o.repo.name}: ${found.flow.label || found.flow.id}`);
    return { ok: true, asked: allowed.asked, cwd, command: found.flow.run };
  }

  return { permit, flow, approvedCount: () => [...approved.values()].reduce((n, s) => n + s.size, 0) };
}

// A flow by id in a launchpad entry, and whether the repository declared it.
function findFlow(entry, flowId) {
  if (!entry) return null;
  const declared = ((entry.contribution || {}).flows || []).find((f) => f.id === flowId);
  const flow = declared || (entry.detected_flows || []).find((f) => f.id === flowId);
  if (!flow || typeof flow.run !== "string" || !flow.run.trim()) return null;
  return { flow, declared: !!declared };
}

// Where a flow runs: the repository, or a directory inside it. `null` when
// the declared directory would leave the repository.
function flowCwd(repoPath, flow) {
  if (!flow.cwd) return repoPath;
  const base = path.resolve(repoPath);
  const target = path.resolve(repoPath, String(flow.cwd));
  return target === base || target.startsWith(base + path.sep) ? target : null;
}

/* What the user is shown. The command and the directory are printed exactly. */

function flowRequest(repoPath, flow, declared, cwd) {
  return {
    repo: repoPath,
    kind: "flow",
    key: `flow\n${cwd}\n${flow.run}`,
    message: `Run \`${flow.run}\` in ${cwd}?`,
    detail: `This command comes from the repository, not from cohors: ${declared
      ? "it is declared in its .principia/repo.json."
      : "it was detected from the repository's own scripts (package.json, Makefile, Cargo and the like)."}`
      + " It runs in a terminal. You are asked once for each command in each repository, until this window closes.",
  };
}

function agentRequest(repoPath, decl, runnerLabel) {
  return {
    repo: repoPath,
    kind: "agent",
    key: `agent\n${decl.id}\n${decl.file}`,
    message: `Run the agent "${decl.label || decl.id}" in ${repoPath}?`,
    detail: `Its instructions are the prompt this repository committed at ${decl.file}. They are handed to ${runnerLabel || "the first installed agent runner"}, which can read and change files in that folder.`
      + " You are asked once for each agent in each repository, until this window closes.",
  };
}

function taskRequest(repoPath, task, runnerLabel) {
  return {
    repo: repoPath,
    kind: "task",
    key: `task\n${task.id}\n${task.title}\n${task.notes || ""}\n${task.agent || ""}`,
    message: `Start work on "${task.title}" in ${repoPath}?`,
    detail: `The task${task.notes ? " and its notes come" : " comes"} from this repository's .principia/repo.json${task.agent ? `, and it names the repository's "${task.agent}" agent` : ""}.`
      + ` It is handed to ${runnerLabel || "the first installed agent runner"} as its instructions, and that agent can read and change files in that folder.`
      + " You are asked once for each task in each repository, until this window closes.",
    action: "Start",
  };
}

module.exports = { create, findFlow, flowCwd, flowRequest, agentRequest, taskRequest };
