// The gate in front of anything a repository supplies and the editor runs.
//
// A flow is a command line. It may be declared in a repository's launchpad
// file or detected from its manifests (an npm script, a Makefile target, a
// gradle task); either way the repository's author wrote it. A declared agent
// is a prompt file the repository committed, and a task's title and notes
// become an agent's instructions. All of that is somebody else's code or text.
//
// Three conditions must hold before any of it runs:
//
//   1. The window is trusted (VS Code Workspace Trust).
//   2. The user has been shown WHAT ACTUALLY RUNS, and where, and said yes.
//      The dashboard lists repositories other than the folder open in this
//      window, so the window's trust says nothing about them.
//   3. What runs is byte for byte what was shown. Nothing is looked up again
//      between the question and the run.
//
// So the order is always: resolve, show, run.
//
//   - A flow's command line must be plain text (an allowlist: printable
//     ASCII plus letters and digits, one line, at most 400 characters). It
//     is shown JSON-quoted, and that same string is sent.
//   - For an agent or a task, cohors is asked for the launch first. Its
//     answer carries the exact text the runner will receive; that text is
//     what is shown, and the command from that same answer is what is sent.
//
// Approvals are remembered per repository root for this session only, keyed
// on the exact command or on a hash of the exact prompt text. A flow whose
// command changed, or an agent whose prompt changed, is new and asks again.
// Nothing is written to disk.
//
// cohors's own prompts (setup-repo, plan-day, …) and the bulk git verbs are
// cohors's text and commands, not a repository's, and do not pass through
// here.
//
// No `vscode` in this file: the host passes in how to read trust and how to
// ask, so these rules can be checked from plain Node (dev/check.js).

const crypto = require("crypto");
const path = require("path");

// A command line a person can read in full, so that what is displayed is
// what runs. This is an ALLOWLIST, the same rule as `is_plain_command` in
// cohors core: non-blank, at most 400 characters, and every character either
// printable ASCII (U+0020..U+007E) or a non-ASCII letter or digit. A list of
// "bad" characters always misses some (direction marks, soft hyphens, line
// separators, variation selectors, tag characters, no-break spaces), so
// nothing is accepted unless it is known to be plain. The four Hangul fillers
// are letters to Unicode but draw as nothing, so they are refused by name.
const MAX_RUN_LENGTH = 400;
const PLAIN = /^[\x20-\x7e\p{L}\p{N}]+$/u;
const HANGUL_FILLERS = /[\u115f\u1160\u3164\uffa0]/;

const isPlain = (text) => typeof text === "string" && PLAIN.test(text) && !HANGUL_FILLERS.test(text);

/** Why a command line may not be run, or null when it may. */
function unsafeRun(run) {
  if (typeof run !== "string" || run.trim() === "") return "it has no command";
  if ([...run].length > MAX_RUN_LENGTH) return `its command is longer than ${MAX_RUN_LENGTH} characters`;
  if (!isPlain(run)) return "its command contains something other than plain letters, digits and ASCII punctuation, such as a line break, a control character or an invisible character";
  return null;
}

// Text made safe to DISPLAY. A prompt is free text, so it cannot be held to
// the allowlist above; instead anything that draws as nothing, or changes
// how its neighbours draw, is spelled out (`\u202e`): control and format
// characters, private-use and unassigned code points, every space other than
// a plain one, variation selectors and the Hangul fillers. Line breaks and
// tabs are kept. This is never what is executed.
const HIDDEN = /(?![\n\t ])[\p{C}\p{Z}\ufe00-\ufe0f\u{e0100}-\u{e01ef}\u115f\u1160\u3164\uffa0]/gu;
function visible(text) {
  return String(text).replace(/\r\n?/g, "\n").replace(HIDDEN, (ch) => {
    const hex = ch.codePointAt(0).toString(16);
    return hex.length <= 4 ? "\\u" + hex.padStart(4, "0") : `\\u{${hex}}`;
  });
}

// The first lines of a prompt, for the dialog.
const EXCERPT_LINES = 12;
const EXCERPT_WIDTH = 160;
function excerpt(text) {
  const lines = visible(text).split("\n");
  const shown = lines.slice(0, EXCERPT_LINES).map((l) => (l.length > EXCERPT_WIDTH ? l.slice(0, EXCERPT_WIDTH) + "…" : l));
  const more = lines.length - shown.length;
  return shown.join("\n") + (more > 0 ? `\n… ${more} more line${more === 1 ? "" : "s"}` : "");
}

const sha256 = (text) => crypto.createHash("sha256").update(text, "utf8").digest("hex");

// A flow by id in a launchpad entry, and whether the repository declared it.
function findFlow(entry, flowId) {
  if (!entry) return null;
  const declared = ((entry.contribution || {}).flows || []).find((f) => f.id === flowId);
  const flow = declared || (entry.detected_flows || []).find((f) => f.id === flowId);
  return flow ? { flow, declared: !!declared } : null;
}

// Where a flow runs: the repository, or a directory inside it. `null` when
// the declared directory would leave the repository or cannot be shown
// faithfully.
function flowCwd(repoPath, flow) {
  if (flow.cwd == null || flow.cwd === "") return repoPath;
  if (!isPlain(flow.cwd) || [...flow.cwd].length > MAX_RUN_LENGTH) return null;   // same rule as the command
  const base = path.resolve(repoPath);
  const target = path.resolve(repoPath, flow.cwd);
  return target === base || target.startsWith(base + path.sep) ? target : null;
}

/* ───────── what the user is shown ───────── */

function flowRequest(repoPath, flow, declared, cwd) {
  return {
    repo: repoPath,
    kind: "flow",
    key: `flow\n${cwd}\n${flow.run}`,
    // JSON-quoted: every character of the command is visible and delimited.
    message: `Run ${JSON.stringify(flow.run)} in ${cwd}?`,
    detail: `This command comes from the repository, not from cohors: ${declared
      ? "it is declared in its .principia/repo.json."
      : "it was detected from the repository's own scripts (package.json, Makefile, Cargo and the like)."}`
      + " It runs in a terminal exactly as shown between the quotes. You are asked once for each command in each repository, until this window closes.",
    action: "Run",
  };
}

// `plan` is cohors's answer for this launch; `text` is the exact prompt the
// runner will receive (null when cohors resolved the launch to a command with
// no prompt, such as reopening a task's linked conversation).
function promptRequest(repoPath, kind, plan, text, runnerLabel) {
  const who = runnerLabel || plan.runner || "the agent runner";
  const what = kind === "task" ? "task" : "agent";
  if (text == null) {
    return {
      repo: repoPath,
      kind,
      key: sha256(`${repoPath}\ncommand\n${plan.command}`),
      message: `Run this ${what} in ${repoPath}?`,
      detail: `Runner: ${who}\nFolder: ${visible(plan.cwd)}\n\ncohors resolved this ${what} to the command below, with no new instructions from the repository:\n\n${JSON.stringify(plan.command)}`,
      action: kind === "task" ? "Start" : "Run",
    };
  }
  return {
    repo: repoPath,
    kind,
    key: sha256(`${repoPath}\n${text}`),
    message: `Hand these instructions to ${who} in ${repoPath}?`,
    detail: `They come from the repository, not from cohors: ${kind === "task" ? "a task in its .principia/repo.json" : "an agent prompt it committed"}.`
      + ` ${who} receives exactly this text and can read and change files in that folder.\n\n${excerpt(text)}\n\n`
      + `You are asked once for each distinct set of instructions in each repository, until this window closes.`,
    action: kind === "task" ? "Start" : "Run",
    // Lets the host offer the whole text, read-only, before the answer.
    prompt: { text, file: plan.prompt_file || null },
  };
}

/**
 * @param {{ isTrusted: () => boolean,
 *           confirm: (request: {message: string, detail: string, action: string,
 *                               prompt?: {text: string, file: string|null}}) => Promise<boolean> }} host
 */
function create(host) {
  const approved = new Map();   // repository root -> Set of approved keys

  const untrusted = (kind) => ({
    ok: false, reason: "untrusted",
    message: `This workspace is not trusted, so ${kind === "flow" ? "flows" : kind === "agent" ? "agents declared by a repository" : "tasks declared by a repository"} are disabled.`,
  });

  // Ask once per repository and key; remember a yes for the session.
  async function permit(request) {
    if (!host.isTrusted()) return untrusted(request.kind);
    const seen = approved.get(request.repo);
    if (seen && seen.has(request.key)) return { ok: true, asked: false };
    const yes = await host.confirm({ message: request.message, detail: request.detail, action: request.action || "Run", prompt: request.prompt });
    if (!yes) return { ok: false, reason: "declined" };
    // Trust may have been withdrawn while the question was on screen.
    if (!host.isTrusted()) return untrusted(request.kind);
    if (!approved.has(request.repo)) approved.set(request.repo, new Set());
    approved.get(request.repo).add(request.key);
    return { ok: true, asked: true };
  }

  /**
   * Run one of a repository's flows, declared or detected.
   * `run(cwd, command, name)` is called only when the window is trusted and
   * this exact command line has been shown and approved for this repository;
   * `command` is the very string that was shown.
   *
   * @param {{repo: {path: string, name: string}, entry: object, flowId: string,
   *          run: (cwd: string, command: string, name: string) => void}} o
   */
  async function flow(o) {
    if (!host.isTrusted()) return untrusted("flow");
    const found = findFlow(o.entry, o.flowId);
    if (!found) return { ok: false, reason: "unknown", message: "That flow is no longer listed for this repository." };
    const label = visible(found.flow.label || found.flow.id || "flow").replace(/\n/g, " ").slice(0, 80);
    const command = found.flow.run;   // read once: this is what is checked, shown and sent
    const bad = unsafeRun(command);
    if (bad) return { ok: false, reason: "unsafe", message: `Refusing to run the flow "${label}" from ${o.repo.name}: ${bad}.` };
    const cwd = flowCwd(o.repo.path, found.flow);
    if (!cwd) return { ok: false, reason: "outside", message: `Refusing to run the flow "${label}" from ${o.repo.name}: its folder is outside the repository or is not a plain path.` };
    const allowed = await permit(flowRequest(o.repo.path, { run: command }, found.declared, cwd));
    if (!allowed.ok) return allowed;
    o.run(cwd, command, `${o.repo.name}: ${label}`);
    return { ok: true, asked: allowed.asked, cwd, command };
  }

  /**
   * Start a repository's declared agent or one of its tasks.
   * Resolve first: `resolve()` asks cohors for the launch, which writes the
   * prompt file and starts nothing. Show second: the question is built from
   * the text in that answer. Run third: `run(plan)` gets that same answer.
   *
   * @param {{repo: string, kind: "agent"|"task",
   *          resolve: () => Promise<{command: string, cwd: string, runner: string, label: string,
   *                                  prompt_file: string|null, prompt?: string|null}>,
   *          readFile: (file: string) => string,
   *          runnerLabel?: (id: string) => string,
   *          run: (plan: object) => void}} o
   */
  async function launch(o) {
    if (!host.isTrusted()) return untrusted(o.kind);   // before cohors is even asked
    const plan = await o.resolve();
    // Newer cohors returns the text itself; older ones only the file it wrote.
    let text = null;
    if (typeof plan.prompt === "string") text = plan.prompt;
    else if (plan.prompt_file) {
      try { text = o.readFile(plan.prompt_file); }
      catch { return { ok: false, reason: "unreadable", message: "cohors wrote a prompt that could not be read back, so nothing was started." }; }
    }
    const label = o.runnerLabel ? o.runnerLabel(plan.runner) : "";
    const allowed = await permit(promptRequest(o.repo, o.kind, plan, text, label));
    if (!allowed.ok) return allowed;
    o.run(plan);
    return { ok: true, asked: allowed.asked, plan };
  }

  return { permit, flow, launch, approvedCount: () => [...approved.values()].reduce((n, s) => n + s.size, 0) };
}

module.exports = { create, findFlow, flowCwd, flowRequest, promptRequest, unsafeRun, isPlain, visible, excerpt, sha256, MAX_RUN_LENGTH };
