// Agent runners and prompts: every call to `cohors agent …` lives here.
//
// The runner table (Claude Code, Codex, Gemini CLI, agy), the prompt texts and
// the exact command line for each runner belong to the cohors binary. The
// extension only asks for them and types the returned command into a terminal.
// If the contract changes, this file is the one to adjust:
//
//   cohors agent runners --json
//     {"runners":[{"id","label","bin","available","path","reason","can_link","can_resume"}]}
//   cohors agent prompts --json
//     {"prompts":[{"id","title","summary","needs_repo"}]}
//   cohors agent command <kind> [--runner <id>] [--repo <path>] [--agent <id>]
//                               [--task <id>] [--session <id>]
//     {"command","cwd","runner","label","prompt_file","linked","session"}
//     Non-zero exit with a one-line reason on stderr when it cannot.
//
// `cohors agent command` writes the prompt file and records the launch itself.

const { CohorsError } = require("./cohors");

// The five launchpad prompts, then the kinds that act on a repository.
const PROMPT_KINDS = ["setup-repo", "declare-flows", "add-agent", "track-tasks", "plan-day"];
const KINDS = [...PROMPT_KINDS, "agent", "task", "start", "resume"];

// The launchpad contract's kebab-case rule for agent and task ids. Checked
// here as well as in cohors: these ids come from a repository's committed
// file, which on a cloned repo is somebody else's text.
const ID_RE = /^[a-z0-9]+(-[a-z0-9]+)*$/;
const SESSION_RE = /^[0-9A-Za-z][0-9A-Za-z._-]{5,127}$/;

const isObj = (v) => !!v && typeof v === "object" && !Array.isArray(v);
const str = (v) => (typeof v === "string" ? v : "");

/** @param {{json: (args: string[], opts?: object) => Promise<any>}} cli */
function create(cli) {
  async function runners() {
    const r = await cli.json(["agent", "runners", "--json"], { timeout: 15000 });
    if (!isObj(r) || !Array.isArray(r.runners)) throw new CohorsError("parse", "cohors agent runners printed an unexpected shape.");
    return r.runners.filter(isObj).map((x) => ({
      id: str(x.id),
      label: str(x.label) || str(x.id),
      bin: str(x.bin),
      available: x.available === true,
      path: str(x.path) || null,
      reason: str(x.reason) || null,
      can_link: x.can_link === true,
      can_resume: x.can_resume === true,
    })).filter((x) => x.id);
  }

  async function prompts() {
    const r = await cli.json(["agent", "prompts", "--json"], { timeout: 15000 });
    if (!isObj(r) || !Array.isArray(r.prompts)) throw new CohorsError("parse", "cohors agent prompts printed an unexpected shape.");
    return r.prompts.filter(isObj).map((x) => ({
      id: str(x.id),
      title: str(x.title) || str(x.id),
      summary: str(x.summary),
      needs_repo: x.needs_repo === true,
    })).filter((x) => x.id);
  }

  // The argument array for `cohors agent command`. Separate from `command`
  // so it can be checked without running anything.
  function commandArgs(kind, opts = {}) {
    if (!KINDS.includes(kind)) throw new CohorsError("failed", `Unknown agent command "${kind}".`);
    const needs = (name) => {
      if (!opts[name]) throw new CohorsError("failed", `cohors agent command ${kind} needs --${name}.`);
    };
    if (kind === "agent") { needs("repo"); needs("agent"); }
    if (kind === "task") { needs("repo"); needs("task"); }
    if (kind === "start") needs("repo");
    if (kind === "resume") { needs("repo"); needs("session"); }
    if (opts.agent && !ID_RE.test(opts.agent)) throw new CohorsError("failed", `Refusing agent id "${opts.agent}": it must be kebab-case.`);
    if (opts.task && !ID_RE.test(opts.task)) throw new CohorsError("failed", `Refusing task id "${opts.task}": it must be kebab-case.`);
    if (opts.session && !SESSION_RE.test(opts.session)) throw new CohorsError("failed", "That does not look like a session id.");

    const args = ["agent", "command", kind];
    for (const flag of ["runner", "repo", "agent", "task", "session"]) {
      if (opts[flag]) args.push(`--${flag}`, String(opts[flag]));
    }
    return args;
  }

  /**
   * Ask cohors for the command that starts (or resumes) an agent session.
   * Returns {command, cwd, runner, label, prompt_file, linked, session}.
   */
  async function command(kind, opts = {}) {
    const r = await cli.json(commandArgs(kind, opts), { timeout: 30000 });
    if (!isObj(r) || !str(r.command) || !str(r.cwd)) {
      throw new CohorsError("parse", "cohors agent command printed an unexpected shape.");
    }
    return {
      command: r.command,
      cwd: r.cwd,
      runner: str(r.runner),
      label: str(r.label) || "cohors agent",
      prompt_file: str(r.prompt_file) || null,
      linked: r.linked === true,
      session: str(r.session) || null,
    };
  }

  return { runners, prompts, command, commandArgs };
}

module.exports = { create, KINDS, PROMPT_KINDS, ID_RE, SESSION_RE };
