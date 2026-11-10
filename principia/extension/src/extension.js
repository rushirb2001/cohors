// Principia: the launchpad host.
//
// Ships empty. Everything on screen is discovered at runtime (discovery.js) or
// declared by a repository in its committed .principia/ directory. Nothing about
// any particular user appears in this file.

const vscode = require("vscode");
const fs = require("fs");
const os = require("os");
const cp = require("child_process");
const crypto = require("crypto");
const path = require("path");

const D = require("./discovery");
const R = require("./runners");

const EXT_DIR = path.join(__dirname, "..");
const REPO_ROOT = path.join(EXT_DIR, "..");   // the principia checkout, when running from source

let panel = null, timer = null;
let refreshing = false, lastRefresh = 0, lastData = null;

const cfg = () => vscode.workspace.getConfiguration("principia");

/* ───────── contract location ───────── */

// Agents need the prompt files and the validator. Prefer a local checkout the
// user points at; otherwise use the copy shipped inside the extension.
function contractRoot() {
  const configured = cfg().get("specPath", "");
  if (configured && D.exists(path.join(configured, "spec/v1/SPEC.md"))) return configured;
  if (D.exists(path.join(REPO_ROOT, "spec/v1/SPEC.md"))) return REPO_ROOT;
  if (D.exists(path.join(EXT_DIR, "contract/spec/v1/SPEC.md"))) return path.join(EXT_DIR, "contract");
  return null;
}

/* ───────── data ───────── */

async function collect() {
  const max = cfg().get("maxRepos", 40);
  const repos = await D.collectRepos(max);
  const board = D.readBoard();
  const history = D.readHistory();
  const runners = R.detect();
  const dev = await D.devices();

  // Group repos by their declared group; ungrouped fall into one bucket at the end.
  const groups = [];
  const index = new Map();
  for (const r of repos) {
    const key = r.group || "";
    if (!index.has(key)) { index.set(key, { name: key, repos: [] }); groups.push(index.get(key)); }
    index.get(key).repos.push(r);
  }
  groups.sort((a, b) => (a.name ? 0 : 1) - (b.name ? 0 : 1) || a.name.localeCompare(b.name));

  const here = (vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0])
    ? vscode.workspace.workspaceFolders[0].uri.fsPath : null;

  lastData = {
    generatedAt: Date.now(),
    repos, groups, runners,
    board: board.focus, boardPresent: board.present, boardUpdated: board.updated,
    history,
    ports: dev.ports, android: dev.android, ios: dev.ios,
    recents: repos.map((r) => ({ name: r.name, root: r.root, ws: r.ws, rank: r.rank, branch: r.branch, dirty: r.dirty })),
    here,
    contract: !!contractRoot(),
    machine: {
      free: Math.round((os.freemem() / 1e9) * 10) / 10,
      total: Math.round(os.totalmem() / 1e9),
      load: os.loadavg()[0].toFixed(1),
    },
    counts: {
      repos: repos.length,
      configured: repos.filter((r) => r.configured).length,
      dirty: repos.filter((r) => r.dirty > 0).length,
      agents: repos.reduce((n, r) => n + r.agents.length, 0),
      tasks: repos.reduce((n, r) => n + r.tasks.length, 0) + board.focus.length,
      flows: repos.reduce((n, r) => n + r.flows.length, 0),
      devices: (dev.android.booted ? 1 : 0) + dev.ios.booted.length + dev.ports.length,
    },
  };
  return lastData;
}

async function refresh(force) {
  if (!panel) return;
  if (refreshing) return;
  if (!force && Date.now() - lastRefresh < 3000) return;
  refreshing = true;
  try { panel.webview.postMessage({ type: "data", ...(await collect()) }); }
  catch (e) { panel.webview.postMessage({ type: "error", message: String((e && e.message) || e) }); }
  finally { refreshing = false; lastRefresh = Date.now(); }
}

function armTimer() {
  clearInterval(timer);
  const s = cfg().get("refreshSeconds", 15);
  // `active`, not `visible`: a tab in a split you are not looking at costs nothing.
  if (s > 0) timer = setInterval(() => { if (panel && panel.active) refresh(); }, s * 1000);
}

// A single write can fire several watcher events, and a git operation touches
// HEAD and the index together. Coalesce them into one refresh instead of one
// per event.
let nudgeTimer = null;
function nudge() {
  clearTimeout(nudgeTimer);
  nudgeTimer = setTimeout(() => refresh(true), 250);
}

/* ───────── actions ───────── */

function terminal(cwd, cmd, name) {
  const t = vscode.window.createTerminal({ name: name || "principia", cwd });
  if (cmd) t.sendText(cmd);
  t.show(false);
  return t;
}

// A repo's workspace file is the intended way in when it has one: it carries
// the folder layout and settings that opening the bare folder throws away.
function openTarget(repo) {
  return repo.ws && D.exists(repo.ws) ? repo.ws : repo.root;
}

function openRepo(repo, newWindow) {
  return vscode.commands.executeCommand(
    "vscode.openFolder", vscode.Uri.file(openTarget(repo)), { forceNewWindow: !!newWindow });
}

/* ───────── "open it and start working" ─────────
 *
 * Opening a folder creates a WINDOW, and that window gets its own extension
 * host — this one cannot reach into it to open a terminal afterwards. So the
 * intent is left on disk and the next window to activate in that folder claims
 * it. One-shot and short-lived: a stale intent must never fire days later.
 */
const PENDING = () => path.join(D.PRINCIPIA_HOME, "pending.json");
const PENDING_TTL_MS = 120000;

function queuePending(root, action) {
  try {
    fs.mkdirSync(D.PRINCIPIA_HOME, { recursive: true });
    fs.writeFileSync(PENDING(), JSON.stringify({ root, action, at: Date.now() }, null, 2) + "\n");
  } catch (e) { console.error("principia: could not queue the pending action", e); }
}

// Claim it exactly once, whether or not it turns out to be ours.
function takePending(here) {
  let p;
  try { p = JSON.parse(fs.readFileSync(PENDING(), "utf8")); } catch { return null; }
  try { fs.unlinkSync(PENDING()); } catch { /* already gone */ }
  if (!p || !p.root || !here) return null;
  if (Date.now() - (p.at || 0) > PENDING_TTL_MS) return null;
  const same = here === p.root || here.startsWith(p.root + path.sep) || p.root.startsWith(here + path.sep);
  return same ? p.action : null;
}

// Terminal in the repo, then a session — resuming the most recent one rather
// than starting cold, which is the whole point of coming back to a project.
function startWorking(repo, runnerId) {
  const runner = pickRunner(runnerId);
  if (!runner) return;
  const latest = (repo.sessions && repo.sessions[0]) || null;
  const resume = latest ? R.resumeCommand(runner.id, latest.id) : null;
  R.recordLaunch({ runner: runner.id, repo: repo.root, branch: repo.branch, agent: resume ? "resume-latest" : "new-session" });
  terminal(repo.root, resume || runner.bin, `${runner.label}: ${repo.name || path.basename(repo.root)}`);
  if (latest && !resume) {
    vscode.window.showInformationMessage(
      `Principia: ${runner.label} cannot reopen a session by id, so this is a fresh one.`);
  }
}

async function launchRepo(root, runnerId) {
  const repo = (lastData && lastData.repos.find((r) => r.root === root))
    || { root, name: path.basename(root) };
  const here = (vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0])
    ? vscode.workspace.workspaceFolders[0].uri.fsPath : null;

  // Already open here: skip straight to working, no pointless second window.
  if (here && (here === repo.root || here.startsWith(repo.root + path.sep))) {
    return void startWorking(repo, runnerId);
  }
  queuePending(repo.root, { kind: "start-working", runner: runnerId || null });
  await openRepo(repo, true);
}

// Writes the prompt to a temp file so every runner can be invoked the same way,
// regardless of how it takes input.
const ID_RE = /^[a-z0-9]+(-[a-z0-9]+)*$/;   // the spec's kebab-case rule

function promptFile(body, tag) {
  const dir = path.join(os.tmpdir(), "principia");
  fs.mkdirSync(dir, { recursive: true });
  // Belt and braces: runners shell-quote, but repo-supplied text must never
  // reach a filename in the first place.
  const safe = String(tag).replace(/[^a-zA-Z0-9_-]/g, "").slice(0, 60) || "prompt";
  const p = path.join(dir, `${safe}-${Date.now()}.md`);
  fs.writeFileSync(p, body);
  return p;
}

// An agent file is declared by a repo we may not have written. Keep it inside
// that repo's .principia/ directory: `..` segments must not escape.
function resolveAgentFile(root, rel) {
  if (typeof rel !== "string" || !rel) return null;
  const base = path.resolve(root, ".principia");
  const target = path.resolve(root, rel);
  if (target !== base && !target.startsWith(base + path.sep)) return null;
  return target;
}

// Committed prompt files must not contain machine-specific paths, so they carry
// placeholders and the real values are only ever resolved into the throwaway
// temp copy handed to the runner.
function substitute(body, repo, taskTitle) {
  const cr = contractRoot();
  return body
    .replace(/\{\{REPO_NAME\}\}/g, (repo && repo.name) || "")
    .replace(/\{\{REPO_ROOT\}\}/g, (repo && repo.root) || "")
    .replace(/\{\{BRANCH\}\}/g, (repo && repo.branch) || "")
    .replace(/\{\{TASK_TITLE\}\}/g, taskTitle || "")
    .replace(/\{\{CONTRACT_ROOT\}\}/g, cr || "the Principia checkout")
    .replace(/\{\{SPEC\}\}/g, cr ? path.join(cr, "spec/v1/SPEC.md") : "spec/v1/SPEC.md")
    .replace(/\{\{VALIDATE\}\}/g, cr ? `node ${path.join(cr, "scripts/validate.js")} .` : "node scripts/validate.js .");
}

// The prompts the launchpad itself can run, keyed by id. This is an allowlist,
// not a path: the webview names an id and never a file, so no message can point
// the runner at an arbitrary file on disk.
const PROMPTS = {
  "setup-repo":    { file: "setup-repo.md",    label: "setup",      scope: "repo" },
  "declare-flows": { file: "declare-flows.md", label: "flows",      scope: "repo" },
  "add-agent":     { file: "add-agent.md",     label: "agent",      scope: "repo" },
  "track-tasks":   { file: "track-tasks.md",   label: "tasks",      scope: "repo" },
  "plan-day":      { file: "plan-day.md",      label: "plan focus", scope: "board" },
};

function noContract() {
  vscode.window.showErrorMessage(
    "Principia: cannot find the contract. Set `principia.specPath` to a checkout of the Principia repo."
  );
}

// One path for every launchpad-run prompt. setupRepo and planBoard were
// near-identical copies of this before, which is how the two drifted apart on
// whether they substituted placeholders at all.
async function runPrompt(id, root, runnerId) {
  const spec = PROMPTS[id];
  if (!spec) { vscode.window.showErrorMessage(`Principia: unknown prompt "${id}".`); return; }

  const cr = contractRoot();
  if (!cr) return void noContract();
  const src = path.join(cr, "prompts", spec.file);
  if (!D.exists(src)) { vscode.window.showErrorMessage(`Principia: missing ${src}`); return; }

  const runner = pickRunner(runnerId);
  if (!runner) return;

  const here = (vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0])
    ? vscode.workspace.workspaceFolders[0].uri.fsPath
    : null;

  const repo = spec.scope === "repo"
    ? ((lastData && lastData.repos.find((r) => r.root === root)) || (root ? { root, name: path.basename(root) } : null))
    : null;

  const cwd = (repo && repo.root) || here || os.homedir();
  const tail = spec.scope === "board"
    ? [
        "Repositories Principia currently knows about:",
        ((lastData && lastData.repos) || []).map((r) => `${r.name}  (${r.root})`).join("\n") || "(none discovered yet)",
        "",
        `Write the board to: ${path.join(os.homedir(), ".principia", "board.json")}`,
      ]
    : [`You are working in the repository at: ${cwd}`];

  const body = [
    substitute(fs.readFileSync(src, "utf8"), repo || {}),
    "",
    "---",
    "",
    ...tail,
    "",
    `The Principia contract is checked out at: ${cr}`,
  ].join("\n");

  const file = promptFile(body, id);
  R.recordLaunch({ runner: runner.id, repo: cwd, branch: (repo && repo.branch) || "", agent: id });
  terminal(cwd, R.commandFor(runner.id, file), `${runner.label}: ${spec.label}`);
}

const setupRepo = (root, runnerId) => runPrompt("setup-repo", root, runnerId);
const planBoard = (runnerId) => runPrompt("plan-day", null, runnerId);

async function runRepoAgent(root, agentId, runnerId, task) {
  const repo = lastData && lastData.repos.find((r) => r.root === root);
  if (!repo) return;
  if (!ID_RE.test(String(agentId || ""))) {
    vscode.window.showErrorMessage(`Principia: refusing agent id "${agentId}" (must be kebab-case).`);
    return;
  }
  const agent = repo.agents.find((a) => a.id === agentId);
  if (!agent) { vscode.window.showErrorMessage(`Principia: no agent "${agentId}" in ${repo.name}`); return; }

  const file = resolveAgentFile(root, agent.file);
  if (!file) {
    vscode.window.showErrorMessage(`Principia: refusing agent file "${agent.file}" (must stay inside .principia/).`);
    return;
  }
  if (!D.exists(file)) { vscode.window.showErrorMessage(`Principia: agent file missing: ${agent.file}`); return; }

  // Repo-declared commands are code from a repository. Honour Workspace Trust.
  if (!vscode.workspace.isTrusted) {
    vscode.window.showWarningMessage("Principia: this workspace is not trusted, so repo-declared agents are disabled.");
    return;
  }

  // Honour the repo's own declaration of which runners can execute this.
  const allowed = agent.supports && agent.supports.length ? agent.supports : null;
  const runner = pickRunner(runnerId, allowed);
  if (!runner) return;

  let body = substitute(fs.readFileSync(file, "utf8"), repo, task && task.title);
  if (task) body += `\n\n---\n\nSpecific task: ${task.title}${task.notes ? `\n\n${task.notes}` : ""}`;
  const tag = task ? `task-${task.id}` : `agent-${agentId}`;
  const p = promptFile(body, tag);
  R.recordLaunch({ runner: runner.id, repo: repo.root, branch: repo.branch, agent: task ? `task:${task.id}` : agentId });
  terminal(repo.root, R.commandFor(runner.id, p), `${runner.label}: ${task ? task.title : (agent.label || agentId)}`);
}

// A task is either declared by the repo itself (`.principia/repo.json` tasks[])
// or a cross-repo focus item (`~/.principia/board.json`). Either way it now gets
// a real action: delegate to a declared agent when `task.agent` names one this
// repo actually ships, otherwise run a generic prompt built from the task itself.
async function runTask(root, taskId, runnerId) {
  if (!ID_RE.test(String(taskId || ""))) {
    vscode.window.showErrorMessage(`Principia: refusing task id "${taskId}" (must be kebab-case).`);
    return;
  }
  const repo = lastData && lastData.repos.find((r) => r.root === root);
  if (!repo) { vscode.window.showErrorMessage("Principia: cannot find this task's repo."); return; }

  const task = repo.tasks.find((t) => t.id === taskId) || ((lastData.board || []).find((f) => f.id === taskId));
  if (!task) { vscode.window.showErrorMessage(`Principia: no task "${taskId}" found.`); return; }

  if (task.agent && repo.agents.some((a) => a.id === task.agent)) {
    return runRepoAgent(root, task.agent, runnerId, task);
  }

  if (!vscode.workspace.isTrusted) {
    vscode.window.showWarningMessage("Principia: this workspace is not trusted, so repo-declared tasks are disabled.");
    return;
  }
  const runner = pickRunner(runnerId);
  if (!runner) return;

  // Already linked to a conversation? Go back to it instead of starting a cold
  // one that has none of the context the first run built up.
  const existing = threadFor(repo, task);
  if (existing) {
    const cmd = R.resumeCommand(runner.id, existing);
    if (cmd) {
      R.recordLaunch({ runner: runner.id, repo: repo.root, branch: repo.branch, agent: `task:${taskId}` });
      return void terminal(repo.root, cmd, `${runner.label}: ${task.title}`);
    }
    vscode.window.showInformationMessage(
      `Principia: ${runner.label} cannot reopen a session by id, so this starts a fresh one.`);
  }

  const cr = contractRoot();
  const body = [
    `You are working in ${repo.name} at ${repo.root}${repo.branch ? ` on branch ${repo.branch}` : ""}.`,
    "",
    `Task: ${task.title}`,
    task.notes ? `Notes: ${task.notes}` : "",
    "",
    `Work on this task. Only mark it "done" in .principia/repo.json with real evidence (a commit, a merged PR, or the user confirming) — never invent completion.`,
    cr ? `The Principia contract is checked out at: ${cr}.` : "",
    cr ? `Validate any repo.json edits with: node ${path.join(cr, "scripts/validate.js")} .` : "",
  ].filter(Boolean).join("\n");

  const file = promptFile(body, `task-${taskId}`);
  // Choose the session id up front where the runner allows it, so the task and
  // the conversation it starts are linked with certainty rather than by
  // guessing which transcript appeared afterwards.
  const sessionId = crypto.randomUUID();
  const started = R.startWithSession(runner.id, file, sessionId);
  if (started.linked) {
    try { D.writeThread(repo.root, taskId, sessionId); }
    catch (e) { console.error("principia: could not record thread", e); }
  }
  R.recordLaunch({ runner: runner.id, repo: repo.root, branch: repo.branch, agent: `task:${taskId}` });
  terminal(repo.root, started.cmd, `${runner.label}: ${task.title}`);
}

// A task's conversation: the board carries `thread` itself (it is user-level and
// never committed), while a repo task's link lives in the gitignored
// .principia/local.json, because a session id is machine-specific.
function threadFor(repo, task) {
  if (task.thread) return task.thread;
  return (repo.threads || {})[task.id] || null;
}

// Resuming reads the user's own local session history, not repo-declared
// content, so this needs no workspace-trust or repo-agent gating — only that
// the id looks like a session id at all.
function resumeSession(root, sessionId) {
  if (!/^[0-9a-fA-F-]{8,64}$/.test(String(sessionId || ""))) return;
  terminal(root, `claude --resume ${R.shq(sessionId)}`, "Claude Code: resume");
}

function newSession(root) {
  if (!root) return;
  terminal(root, "claude", "Claude Code: new");
}

function pickRunner(runnerId, allowed) {
  const detected = R.detect();
  let usable = detected.filter((r) => r.available);
  if (allowed) usable = usable.filter((r) => allowed.includes(r.id));
  if (runnerId) {
    const want = detected.find((r) => r.id === runnerId);
    if (want && want.available) return want;
    if (want) { vscode.window.showErrorMessage(`Principia: ${want.label} is unavailable. ${want.reason}`); return null; }
  }
  if (!usable.length) {
    const names = (allowed || detected.map((r) => r.id)).join(", ");
    vscode.window.showErrorMessage(`Principia: no supported agent runner is installed (looked for: ${names}).`);
    return null;
  }
  return usable[0];
}

/* ───────── webview ───────── */

function html(webview) {
  const nonce = [...Array(24)].map(() => Math.random().toString(36)[2]).join("");
  const u = (f) => webview.asWebviewUri(vscode.Uri.file(path.join(EXT_DIR, "media", f)));
  return `<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource} 'unsafe-inline'; font-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>@font-face{font-family:"codicon";src:url("${u("codicon.ttf")}") format("truetype");}</style>
<link rel="stylesheet" href="${u("codicon.css")}">
<link rel="stylesheet" href="${u("main.css")}">
<title>Principia</title></head>
<body><div id="app" tabindex="0"></div>
<script nonce="${nonce}" src="${u("main.js")}"></script></body></html>`;
}

function show() {
  if (panel) { panel.reveal(vscode.ViewColumn.One); refresh(true); return; }
  panel = vscode.window.createWebviewPanel("principia", "Principia", vscode.ViewColumn.One, {
    enableScripts: true,
    retainContextWhenHidden: true,
    localResourceRoots: [vscode.Uri.file(path.join(EXT_DIR, "media"))],
  });
  panel.webview.html = html(panel.webview);
  panel.onDidDispose(() => { panel = null; clearInterval(timer); });
  panel.onDidChangeViewState((e) => {
    if (e.webviewPanel.active && Date.now() - lastRefresh > 30000) refresh();
  });
  panel.webview.onDidReceiveMessage(onMessage);
  armTimer();
  refresh(true);
}

async function onMessage(m) {
  switch (m.type) {
    case "ready":
    case "refresh":
      return refresh(true);
    case "open":
      return openRepo(lastData.repos.find((r) => r.root === m.root) || { root: m.root }, m.newWindow);
    case "flow": {
      const repo = lastData.repos.find((r) => r.root === m.root);
      if (!repo) return;
      const flow = repo.flows.find((f) => f.id === m.id);
      if (!flow) return;
      if (flow.source === "declared" && !vscode.workspace.isTrusted) {
        vscode.window.showWarningMessage("Principia: this workspace is not trusted, so repo-declared flows are disabled.");
        return;
      }
      const cwd = flow.cwd ? path.join(repo.root, flow.cwd) : repo.root;
      return void terminal(cwd, flow.run, `${repo.name}: ${flow.label}`);
    }
    case "terminal":
      return void terminal(m.root, m.cmd || "", m.name);
    case "setup":
      return setupRepo(m.root, m.runner);
    case "plan":
      return planBoard(m.runner);
    case "prompt":
      return runPrompt(m.id, m.root, m.runner);
    case "agent":
      return runRepoAgent(m.root, m.id, m.runner);
    case "task":
      return runTask(m.root, m.id, m.runner);
    case "resume":
      return void resumeSession(m.root, m.id);
    case "newSession":
      return void newSession(m.root);
    case "launch":
      return launchRepo(m.root, m.runner);
    case "android": {
      const st = await D.devices().then((d) => d.android);
      if (!st.available) return void vscode.window.showErrorMessage(`Launch pad: ${st.reason}`);
      if (m.action === "stop") { await D.run(D.ADB, ["emu", "kill"]); return void setTimeout(() => refresh(true), 2500); }
      if (!st.avds.length) return void vscode.window.showErrorMessage("Launch pad: no AVDs configured.");
      if (st.booted) return void vscode.window.showInformationMessage(`Launch pad: ${st.device} is already running.`);
      // Headless on purpose: the point is to see it inside the editor, not in a
      // second native window. Capped memory for laptops.
      cp.spawn(D.EMU, ["-avd", st.avds[0], "-no-window", "-no-audio", "-no-boot-anim", "-memory", "2048", "-no-snapshot-save"],
        { detached: true, stdio: "ignore" }).unref();
      vscode.window.showInformationMessage(`Launch pad: booting ${st.avds[0]} headless.`);
      return void setTimeout(() => refresh(true), 15000);
    }
    case "ios": {
      if (!/^[0-9A-Fa-f-]{8,64}$/.test(String(m.udid || ""))) return;
      if (m.action === "boot") { await D.run("xcrun", ["simctl", "boot", m.udid], { timeout: 20000 }); await D.run("open", ["-a", "Simulator"]); }
      if (m.action === "shutdown") await D.run("xcrun", ["simctl", "shutdown", m.udid], { timeout: 20000 });
      return void setTimeout(() => refresh(true), 3000);
    }
    case "browser":
      return vscode.commands.executeCommand("simpleBrowser.show", m.url);
    case "external":
      return vscode.env.openExternal(vscode.Uri.parse(m.url));
    case "reveal":
      return vscode.commands.executeCommand("revealFileInOS", vscode.Uri.file(m.path));
    case "openFile":
      return void vscode.window.showTextDocument(vscode.Uri.file(m.path), { preview: true });
    case "openSpec": {
      const cr = contractRoot();
      if (!cr) return void vscode.window.showErrorMessage("Principia: no contract checkout found.");
      return void vscode.window.showTextDocument(vscode.Uri.file(path.join(cr, "spec/v1/SPEC.md")));
    }
    case "copy":
      return vscode.env.clipboard.writeText(m.text);
    case "command":
      return void vscode.commands.executeCommand(m.id);
    case "redetect":
      R.resetDetection();
      return refresh(true);
    default:
      return;
  }
}

/* ───────── lifecycle ───────── */

function activate(context) {
  context.subscriptions.push(
    vscode.commands.registerCommand("principia.show", show),
    vscode.commands.registerCommand("principia.refresh", () => refresh(true)),
    vscode.commands.registerCommand("principia.openSpec", () => onMessage({ type: "openSpec" })),
    vscode.commands.registerCommand("principia.setupRepo", async () => {
      const here = vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0];
      if (!here) return void vscode.window.showErrorMessage("Principia: open a folder first.");
      await setupRepo(here.uri.fsPath);
    }),
    vscode.workspace.onDidChangeConfiguration((e) => { if (e.affectsConfiguration("principia")) armTimer(); }),
  );

  // Watchers, so the panel reflects reality without waiting for a poll.
  //
  // A bare "**/…" glob only covers the OPEN workspace folders, which is why the
  // board never updated: ~/.principia/board.json lives outside every workspace,
  // so "plan my day" wrote it and nothing noticed. Anything outside the
  // workspace needs an absolute RelativePattern base.
  const watch = (pattern, label) => {
    try {
      const w = vscode.workspace.createFileSystemWatcher(pattern);
      w.onDidCreate(nudge);
      w.onDidChange(nudge);
      w.onDidDelete(nudge);
      context.subscriptions.push(w);
    } catch (e) {
      console.error(`principia: could not watch ${label}`, e);
    }
  };

  // A repo's contribution, in any open workspace folder.
  watch("**/.principia/*.json", "workspace .principia");
  // The cross-repo board and the machine-local files, which live in $HOME.
  watch(new vscode.RelativePattern(vscode.Uri.file(D.PRINCIPIA_HOME), "*.json"), "~/.principia");
  // Branch switches and commits move HEAD; staging and committing move the
  // index. Both change what every tab shows about a repo.
  for (const f of vscode.workspace.workspaceFolders || []) {
    watch(new vscode.RelativePattern(f, ".git/{HEAD,index}"), `${f.name} git state`);
  }

  // Did another window ask us to start working here? Claim it before anything
  // else can, then act once discovery knows about this repo.
  {
    const here = (vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0])
      ? vscode.workspace.workspaceFolders[0].uri.fsPath : null;
    const pending = takePending(here);
    if (pending && pending.kind === "start-working") {
      collect()
        .then(() => {
          const repo = (lastData.repos || []).find((r) => r.root === here)
            || { root: here, name: path.basename(here) };
          startWorking(repo, pending.runner);
        })
        .catch((e) => console.error("principia: pending action failed", e));
    }
  }

  const mode = cfg().get("openOnStartup", "emptyWindow");
  const empty = !(vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders.length);
  if (mode === "always" || (mode === "emptyWindow" && empty)) show();
}

function deactivate() { clearInterval(timer); }

module.exports = { activate, deactivate };
