// cohors for VS Code: the extension host.
//
// Ships empty. Every number on screen comes from the `cohors` binary
// (src/cohors.js); agent runners and prompts come from `cohors agent`
// (src/agent.js). Nothing about any particular user appears in this file.
//
// The host's job is small: run cohors, hand the JSON to the webview, and turn
// the webview's messages into editor actions (open a folder, open a terminal,
// run a bulk git action where the user can watch it).

const vscode = require("vscode");
const cp = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const Cohors = require("./cohors");
const Agent = require("./agent");
const Local = require("./local");
const Gate = require("./gate");
const Devices = require("./devices");

const EXT_DIR = path.join(__dirname, "..");
const INSTALL_URL = "https://github.com/rushirb2001/cohors#install";

let ctx = null;
let panel = null, timer = null, tree = null;
let refreshing = false, lastRefresh = 0, lastData = null;
let pendingReveal = null;   // a page/repo to show once the webview is ready

const cfg = () => vscode.workspace.getConfiguration("cohors");
const cli = Cohors.create(() => ({
  binaryPath: cfg().get("binaryPath", ""),
  roots: cfg().get("roots", []),
}));
const agent = Agent.create(cli);

const workspaceFolders = () => (vscode.workspace.workspaceFolders || []).map((f) => f.uri.fsPath);
const post = (m) => { if (panel) panel.webview.postMessage(m); };
const fail = (e) => vscode.window.showErrorMessage(`cohors: ${(e && e.message) || e}`);

/* ───────── data ───────── */

// Runners and prompts change only when something is installed, so they are
// asked for once and again on "Detect again" or a settings change.
let agentInfo = null;
async function loadAgentInfo() {
  if (agentInfo) return agentInfo;
  try {
    const [runners, prompts] = await Promise.all([agent.runners(), agent.prompts()]);
    agentInfo = { runners, prompts, error: null };
  } catch (e) {
    const info = {
      runners: [], prompts: [],
      error: e.kind === "unsupported"
        ? "This version of cohors has no `cohors agent` command, so agent setup is unavailable. Update cohors, then choose Detect again."
        : `cohors could not list agent runners: ${e.message}`,
    };
    if (e.kind === "missing") return info;   // do not cache: the binary may appear
    agentInfo = info;
  }
  return agentInfo;
}

let versionCache = null;
async function binaryVersion(binPath) {
  if (versionCache && versionCache.path === binPath) return versionCache.version;
  versionCache = { path: binPath, version: await cli.version() };
  return versionCache.version;
}

async function collect() {
  const base = {
    generatedAt: Math.floor(Date.now() / 1000),
    home: os.homedir(),
    here: workspaceFolders(),
    trusted: vscode.workspace.isTrusted,
    rootsSetting: cfg().get("roots", []),
  };

  const bin = await cli.locate();
  if (!bin.path) {
    lastData = { ...base, status: "missing", problem: { message: bin.reason, configured: !!bin.configured } };
    return lastData;
  }

  let overview, launchpad;
  try {
    [overview, launchpad] = await Promise.all([cli.overview(), cli.launchpad()]);
  } catch (e) {
    lastData = {
      ...base,
      status: e.kind === "missing" ? "missing" : "error",
      binary: { path: bin.path },
      problem: { message: e.message, detail: e.stderr || "", kind: e.kind || "failed", configured: !!bin.configured },
    };
    return lastData;
  }

  const [info, version] = await Promise.all([loadAgentInfo(), binaryVersion(bin.path)]);

  // Machine-local extras, only for repositories that opted in with a
  // launchpad file (see src/local.js for why the gate is at the read).
  const pathOf = new Map(overview.repos.map((r) => [r.id, r.path]));
  const threads = {}, sessions = {};
  for (const entry of launchpad.repos) {
    const p = pathOf.get(entry.id);
    if (!entry.configured || !p || !path.isAbsolute(p)) continue;
    const t = Local.readThreads(p);
    if (Object.keys(t).length) threads[entry.id] = t;
    const s = Local.claudeSessions(p, 5);
    if (s.length) sessions[entry.id] = s;
  }

  lastData = {
    ...base, status: "ok",
    binary: { path: bin.path, version },
    overview, launchpad,
    agent: info,
    local: { threads, sessions },
  };
  return lastData;
}

async function refresh(force) {
  if (refreshing) return;
  if (!force && Date.now() - lastRefresh < 3000) return;
  refreshing = true;
  post({ type: "busy", on: true });
  try {
    const state = await collect();
    post({ type: "data", state });
  } catch (e) {
    lastData = {
      generatedAt: Math.floor(Date.now() / 1000), home: os.homedir(), here: workspaceFolders(),
      trusted: vscode.workspace.isTrusted, rootsSetting: cfg().get("roots", []),
      status: "error", problem: { message: String((e && e.message) || e), detail: "", kind: "failed" },
    };
    post({ type: "data", state: lastData });
  } finally {
    refreshing = false;
    lastRefresh = Date.now();
    post({ type: "busy", on: false });
    syncSidebar();
  }
}

function armTimer() {
  clearInterval(timer);
  const s = cfg().get("refreshSeconds", 30);
  // `active`, not `visible`: a tab in a split you are not looking at costs nothing.
  if (s > 0) timer = setInterval(() => { if (panel && panel.active) refresh(); }, Math.max(5, s) * 1000);
}

// One write can fire several watcher events, and a git operation touches HEAD
// and the index together. Coalesce them into one refresh.
let nudgeTimer = null;
function nudge(delay) {
  clearTimeout(nudgeTimer);
  nudgeTimer = setTimeout(() => refresh(true), delay || 300);
}

const repoById = (id) => (lastData && lastData.overview && lastData.overview.repos.find((r) => r.id === id)) || null;
const entryById = (id) => (lastData && lastData.launchpad && lastData.launchpad.repos.find((e) => e.id === id)) || null;

// A repository the webview named, with a real directory behind it. The
// webview only ever sends ids, so no message can point the host at an
// arbitrary path.
function repoWithPath(id) {
  const repo = repoById(id);
  if (!repo || !repo.path || !path.isAbsolute(repo.path)) return null;
  return repo;
}

/* ───────── terminals ───────── */

// A plain shell terminal: used for agent sessions (the command string comes
// from `cohors agent command`) and for a repository's own flows.
function terminal(cwd, text, name) {
  const t = vscode.window.createTerminal({ name: name || "cohors", cwd });
  if (text) t.sendText(text);
  t.show(false);
  return t;
}

// How a command line is SHOWN. The process itself is started with the
// argument array, so this quoting is for the reader only.
function shown(bin, args) {
  const q = (a) => (/^[A-Za-z0-9_\-./:=@~]+$/.test(a) ? a : `'${String(a).replace(/'/g, "'\\''")}'`);
  return [path.basename(bin), ...args].map(q).join(" ");
}

// Run cohors where the user can watch it: a terminal whose "shell" is the
// cohors process itself. No shell is involved, so nothing is quoted or
// expanded, and the first line shows exactly what is running.
function runVisible(name, bin, args) {
  const write = new vscode.EventEmitter();
  const close = new vscode.EventEmitter();
  let child = null, done = false;
  const out = (chunk) => write.fire(String(chunk).replace(/\r?\n/g, "\r\n"));
  const dim = (s) => `\x1b[2m${s}\x1b[0m`;

  const pty = {
    onDidWrite: write.event,
    onDidClose: close.event,
    open() {
      out(`${dim("$ " + shown(bin, args))}\n\n`);
      try {
        child = cp.spawn(bin, args, { windowsHide: true });
      } catch (e) {
        done = true;
        out(`Could not start cohors: ${e.message}\n${dim("Press any key to close this terminal.")}\n`);
        return;
      }
      child.stdout.setEncoding("utf8");
      child.stderr.setEncoding("utf8");
      child.stdout.on("data", out);
      child.stderr.on("data", out);
      child.on("error", (e) => {
        done = true;
        out(`\nCould not start cohors: ${e.message}\n${dim("Press any key to close this terminal.")}\n`);
      });
      child.on("close", (code, signal) => {
        if (done) return;
        done = true;
        const how = signal ? `cohors was stopped (${signal}).` : code === 0 ? "Done." : `cohors exited with code ${code}.`;
        out(`\n${dim(how + " Press any key to close this terminal.")}\n`);
        nudge(500);   // the fleet just changed
      });
    },
    close() { if (child && !done) child.kill(); },
    handleInput(data) {
      if (done) return void close.fire(0);
      if (data === "\x03" && child) child.kill("SIGINT");
    },
  };
  const t = vscode.window.createTerminal({ name, pty });
  t.show(false);
  return t;
}

/* ───────── bulk git actions ───────── */

const VERB_INFO = {
  fetch:  { title: "Fetch",  detail: "Runs git fetch. It never touches your working tree.", confirm: false },
  pull:   { title: "Pull",   detail: "Runs git pull --ff-only. It never merges and never loses work.", confirm: true },
  push:   { title: "Push",   detail: "Runs git push. It never force-pushes. A branch with no upstream is published.", confirm: true },
  commit: { title: "Commit", detail: "Stages everything (git add -A) and commits with one message in each repository.", confirm: true },
  stash:  { title: "Stash",  detail: "Runs git stash push for tracked changes.", confirm: true },
};

function describe(selector) {
  if (selector.all) return "every repository";
  if (selector.ids.length === 1) {
    const r = repoById(selector.ids[0]);
    return r ? r.name : "1 repository";
  }
  return `${selector.ids.length} repositories`;
}

async function runVerb(verb, selector, opts = {}) {
  const info = VERB_INFO[verb];
  if (!info) return;
  if (!selector.all && !(selector.ids || []).length) {
    return void vscode.window.showInformationMessage("cohors: select at least one repository first.");
  }
  const bin = await cli.locate();
  if (!bin.path) return void fail(new Error(bin.reason));

  const where = describe(selector);
  let dryRun = !!opts.dryRun;
  let message;

  if (verb === "commit") {
    message = await vscode.window.showInputBox({
      title: `Commit in ${where}`,
      prompt: "Commit message, used in every selected repository",
      validateInput: (v) => (v.trim() ? null : "A commit needs a message."),
    });
    if (!message) return;
  }
  if (info.confirm && !dryRun) {
    const go = `${info.title} ${selector.all ? "all" : selector.ids.length}`;
    const preview = "Preview (dry run)";
    const pick = await vscode.window.showWarningMessage(
      `${info.title} ${where}?`,
      { modal: true, detail: `${info.detail}\n\nIt runs in a terminal, so you can see exactly what happens. Preview lists the repositories without changing anything.` },
      go, preview);
    if (!pick) return;
    dryRun = pick === preview;
  }

  let args;
  try { args = cli.actionArgs(verb, selector, { dryRun, message }); }
  catch (e) { return void fail(e); }
  runVisible(`cohors ${verb}${dryRun ? " (dry run)" : ""}`, bin.path, args);
}

// "Act on selected": a native picker over the five verbs.
async function actOn(ids) {
  if (!ids.length) return void vscode.window.showInformationMessage("cohors: select at least one repository first.");
  const selector = { ids };
  const pick = await vscode.window.showQuickPick(
    Cohors.VERBS.map((v) => ({ label: VERB_INFO[v].title, detail: VERB_INFO[v].detail, verb: v })),
    { title: `Act on ${describe(selector)}`, placeHolder: "Choose what to run. Anything that changes a repository asks first." });
  if (pick) await runVerb(pick.verb, selector);
}

/* ───────── opening repositories ───────── */

// A repo's workspace file is the intended way in when it has one: it carries
// the folder layout and settings that opening the bare folder throws away.
function openTarget(repoPath) {
  try {
    const hit = fs.readdirSync(repoPath).find((n) => n.endsWith(".code-workspace"));
    return hit ? path.join(repoPath, hit) : repoPath;
  } catch { return repoPath; }
}

function openRepo(repoPath, newWindow) {
  return vscode.commands.executeCommand(
    "vscode.openFolder", vscode.Uri.file(openTarget(repoPath)), { forceNewWindow: !!newWindow });
}

/* ───────── agents ───────── */

// The full text of a prompt awaiting approval, as a read-only document. It
// is served from memory, so what is read is exactly the text that was
// resolved and will be approved, not whatever is on disk a moment later.
const PROMPT_SCHEME = "cohors-prompt";
const promptTexts = new Map();   // uri path -> text
let promptCounter = 0;
const promptProvider = { provideTextDocumentContent: (uri) => promptTexts.get(uri.path) || "" };

async function showPromptText(prompt) {
  const name = path.basename(prompt.file || "prompt.md").replace(/[^A-Za-z0-9._-]/g, "_");
  const uri = vscode.Uri.from({ scheme: PROMPT_SCHEME, path: `/${++promptCounter}/${name}` });
  promptTexts.set(uri.path, prompt.text);
  if (promptTexts.size > 20) promptTexts.delete(promptTexts.keys().next().value);
  const doc = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(doc, { preview: true });
}

// How the gate asks. A flow is a yes or no. For an agent or a task the
// dialog shows the start of the prompt and offers the whole of it; choosing
// that opens it read-only and asks again without covering it.
async function confirmWithUser({ message, detail, action, prompt }) {
  if (!prompt) return (await vscode.window.showWarningMessage(message, { modal: true, detail }, action)) === action;
  const SHOW = "Show full prompt";
  const pick = await vscode.window.showWarningMessage(message, { modal: true, detail }, action, SHOW);
  if (pick === action) return true;
  if (pick !== SHOW) return false;
  await showPromptText(prompt);
  return (await vscode.window.showInformationMessage(message, action, "Cancel")) === action;
}

// Anything a repository supplies (a flow's command line, a declared agent's
// prompt, a task's text) passes through this gate: the window must be
// trusted, and the user must have been shown what actually runs, and where.
// Approvals live in memory for this session only. See src/gate.js.
const gate = Gate.create({
  isTrusted: () => vscode.workspace.isTrusted,
  confirm: confirmWithUser,
});

// Report why the gate said no. A declined prompt needs no second message.
function refused(result) {
  if (result.reason === "declined") return;
  vscode.window.showWarningMessage(`cohors: ${result.message}`);
}

const runnerLabel = (id) => {
  const r = id && agentInfo && agentInfo.runners.find((x) => x.id === id);
  return r ? r.label : "";
};

// cohors's own launches: its shipped prompts (setup-repo, plan-day, …),
// starting work in a repository, resuming a session. Ask cohors for the
// command, open a terminal where it says, and type the command exactly as
// returned. Nothing a repository wrote is involved, so there is no gate; a
// repository's agents and tasks go through `launchDeclared` instead.
async function launch(kind, opts) {
  if (kind === "agent" || kind === "task") throw new Error("declared agents and tasks must go through the gate");
  try {
    const r = await agent.command(kind, opts);
    terminal(r.cwd, r.command, r.label);
    nudge(1500);   // the launch is recorded
  } catch (e) { fail(e); }
}

// A repository's declared agent or task: resolve, show, run (src/gate.js).
// cohors is asked for the launch first (that writes the prompt and starts
// nothing); the user is shown the text in that answer; and the command from
// that same answer is what is typed. Nothing is resolved twice.
async function launchDeclared(kind, opts) {
  try {
    const result = await gate.launch({
      repo: opts.repo,
      kind,
      resolve: () => agent.command(kind, opts),
      readFile: (file) => fs.readFileSync(file, "utf8"),
      runnerLabel,
      run: (plan) => {
        terminal(plan.cwd, plan.command, plan.label);
        nudge(1500);   // a task may now be linked to its conversation
      },
    });
    if (!result.ok) refused(result);
  } catch (e) { fail(e); }
}

const pendingDir = () => ctx.globalStorageUri.fsPath;

// Open the repository in its own window and start working there. A new
// window has its own extension host, so the intent is left on disk for it to
// claim (see src/local.js).
async function startIn(id, runner) {
  const repo = repoWithPath(id);
  if (!repo) return;
  const open = workspaceFolders().some((h) => h === repo.path || h.startsWith(repo.path + path.sep));
  if (open) return launch("start", { repo: repo.path, runner });
  try { Local.queuePending(pendingDir(), repo.path, { kind: "start", runner: runner || null }); }
  catch (e) { console.error("cohors: could not queue the pending action", e); }
  await openRepo(repo.path, true);
}

// Every flow is a command line the repository's author wrote, whether it was
// declared in the launchpad file or detected from a manifest. None runs
// without trust and without having been shown (src/gate.js).
async function runFlow(id, flowId) {
  const repo = repoWithPath(id), entry = entryById(id);
  if (!repo || !entry) return;
  const result = await gate.flow({ repo, entry, flowId, run: terminal });
  if (!result.ok) refused(result);
}

// A repository's declared agent, through the gate.
function runAgent(id, agentId, runner) {
  const repo = repoWithPath(id);
  if (!repo) return undefined;
  return launchDeclared("agent", { repo: repo.path, agent: String(agentId || ""), runner });
}

// A repository's task, through the gate.
function runTask(repo, taskId, runner) {
  return launchDeclared("task", { repo: repo.path, task: String(taskId || ""), runner });
}

// For the command palette, where there is no picker on screen: choose a
// runner only when there is a real choice to make.
async function pickRunner() {
  const info = await loadAgentInfo();
  if (info.error) { fail(new Error(info.error)); return undefined; }
  const usable = info.runners.filter((r) => r.available);
  if (!usable.length) {
    fail(new Error(`no agent runner is installed. Looked for: ${info.runners.map((r) => r.bin).join(", ")}.`));
    return undefined;
  }
  if (usable.length === 1) return usable[0].id;
  const pick = await vscode.window.showQuickPick(info.runners.map((r) => ({
    label: `${r.available ? "$(pass)" : "$(circle-slash)"} ${r.label}`,
    description: r.available ? (r.path || r.bin) : (r.reason || "not installed"),
    runner: r,
  })), { title: "Run with which agent?" });
  if (!pick) return undefined;
  if (!pick.runner.available) {
    fail(new Error(`${pick.runner.label} is unavailable. ${pick.runner.reason || ""}`));
    return undefined;
  }
  return pick.runner.id;
}

/* ───────── webview ───────── */

function html(webview) {
  const nonce = [...Array(32)].map(() => Math.floor(Math.random() * 36).toString(36)).join("");
  const u = (f) => webview.asWebviewUri(vscode.Uri.file(path.join(EXT_DIR, "media", f)));
  return `<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource} 'unsafe-inline'; font-src ${webview.cspSource}; img-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>@font-face{font-family:"codicon";src:url("${u("codicon.ttf")}") format("truetype");}</style>
<link rel="stylesheet" href="${u("codicon.css")}">
<link rel="stylesheet" href="${u("main.css")}">
<title>cohors</title></head>
<body><div id="app"></div>
<script nonce="${nonce}" src="${u("view.js")}"></script>
<script nonce="${nonce}" src="${u("main.js")}"></script></body></html>`;
}

// `reveal` is optional: { page, repo } to land on.
function show(reveal) {
  if (reveal && typeof reveal === "object" && (reveal.page || reveal.repo)) pendingReveal = reveal;
  if (panel) {
    panel.reveal(vscode.ViewColumn.One);
    if (pendingReveal) { post({ type: "reveal", ...pendingReveal }); pendingReveal = null; }
    return void refresh(true);
  }
  panel = vscode.window.createWebviewPanel("cohors", "cohors", vscode.ViewColumn.One, {
    enableScripts: true,
    retainContextWhenHidden: true,
    localResourceRoots: [vscode.Uri.file(path.join(EXT_DIR, "media"))],
  });
  panel.iconPath = {
    light: vscode.Uri.file(path.join(EXT_DIR, "media", "cohors-light.svg")),
    dark: vscode.Uri.file(path.join(EXT_DIR, "media", "cohors-dark.svg")),
  };
  panel.webview.html = html(panel.webview);
  panel.onDidDispose(() => { panel = null; clearInterval(timer); });
  panel.onDidChangeViewState((e) => {
    if (e.webviewPanel.active && Date.now() - lastRefresh > 30000) refresh();
  });
  panel.webview.onDidReceiveMessage((m) => onMessage(m).catch(fail));
  armTimer();
}

async function onMessage(m) {
  if (!m || typeof m.type !== "string") return;
  switch (m.type) {
    case "ready":
      if (lastData) post({ type: "data", state: lastData });
      if (pendingReveal) { post({ type: "reveal", ...pendingReveal }); pendingReveal = null; }
      return refresh(true);
    case "refresh":
      return refresh(true);
    case "redetect":
      cli.reset(); agentInfo = null; versionCache = null;
      return refresh(true);

    /* repositories */
    case "open": {
      const repo = repoWithPath(m.id);
      return repo ? openRepo(repo.path, m.newWindow) : undefined;
    }
    case "terminal": {
      const repo = repoWithPath(m.id);
      return repo ? void terminal(repo.path, "", repo.name) : undefined;
    }
    case "verb":
      return runVerb(String(m.verb), m.all ? { all: true } : { ids: (m.ids || []).map(String) });
    case "act":
      return actOn((m.ids || []).map(String));
    case "flow":
      return runFlow(m.id, m.flow);
    case "link": {
      // Only a link this repository actually declared, and only over http(s).
      const entry = entryById(m.id);
      const link = entry && ((entry.contribution || {}).links || []).find((l) => l.url === m.url);
      if (!link || !/^https?:\/\//i.test(link.url)) return;
      return vscode.env.openExternal(vscode.Uri.parse(link.url));
    }
    case "openAgentFile": {
      const repo = repoWithPath(m.id), entry = entryById(m.id);
      const decl = entry && ((entry.contribution || {}).agents || []).find((a) => a.id === m.agent);
      const file = repo && decl && Local.resolveInside(repo.path, decl.file, ".cohors");
      if (!file || !Local.exists(file)) return void fail(new Error("that agent's prompt file is missing or outside .cohors/."));
      return void vscode.window.showTextDocument(vscode.Uri.file(file), { preview: true });
    }

    /* agents */
    case "prompt": {
      // One of the five launchpad prompts. Everything but plan-day works in a repo.
      if (!Agent.PROMPT_KINDS.includes(m.kind)) return;
      if (m.kind === "plan-day") return launch("plan-day", { runner: m.runner });
      const repo = repoWithPath(m.id);
      return repo ? launch(m.kind, { repo: repo.path, runner: m.runner }) : undefined;
    }
    case "setupHere": {
      const here = workspaceFolders()[0];
      if (!here) return void fail(new Error("open a folder first."));
      return launch(Agent.PROMPT_KINDS.includes(m.kind) ? m.kind : "setup-repo", { repo: here, runner: m.runner });
    }
    case "agent":
      return runAgent(m.id, m.agent, m.runner);
    case "task": {
      const repo = repoWithPath(m.id);
      return repo ? runTask(repo, m.task, m.runner) : undefined;
    }
    case "focus": {
      // A focus-board item names its repository by name and its task by `ref`.
      const item = ((lastData && lastData.launchpad && lastData.launchpad.board.focus) || []).find((f) => f.id === m.item);
      const repo = item && lastData.overview.repos.find((r) => r.name === item.repo && r.path && path.isAbsolute(r.path));
      if (!item || !repo) return void fail(new Error("that focus item does not point at a repository cohors knows."));
      return runTask(repo, item.ref || item.id, m.runner);
    }
    case "resume": {
      const repo = repoWithPath(m.id);
      return repo ? launch("resume", { repo: repo.path, session: m.session, runner: m.runner }) : undefined;
    }
    case "start":
      return startIn(m.id, m.runner);

    /* standup */
    case "standup": {
      const today = m.window === "today";
      try { post({ type: "standup", window: today ? "today" : "week", data: await cli.standup(today) }); }
      catch (e) { post({ type: "standup", window: today ? "today" : "week", error: e.message }); }
      return;
    }
    case "copyStandup": {
      const md = await cli.standupMarkdown(m.window === "today");
      await vscode.env.clipboard.writeText(md);
      return void post({ type: "toast", text: "Standup copied as markdown" });
    }

    /* devices */
    case "devices":
      return void post({ type: "devices", data: await Devices.state(!!m.force) });
    case "android": {
      if (m.action === "stop") { await Devices.stopAndroid(); }
      else {
        const r = await Devices.bootAndroid();
        (r.ok ? vscode.window.showInformationMessage : vscode.window.showErrorMessage)(`cohors: ${r.message}`);
      }
      return void setTimeout(async () => post({ type: "devices", data: await Devices.state(true) }), m.action === "stop" ? 2500 : 15000);
    }
    case "ios":
      await Devices.ios(m.action, m.udid);
      return void post({ type: "devices", data: await Devices.state(true) });
    case "port": {
      const port = Number(m.port);
      if (!Number.isInteger(port) || port < 1 || port > 65535) return;
      const url = `http://localhost:${port}`;
      return m.external
        ? vscode.env.openExternal(vscode.Uri.parse(url))
        : vscode.commands.executeCommand("simpleBrowser.show", url);
    }

    /* settings and help */
    case "copy":
      await vscode.env.clipboard.writeText(String(m.text || ""));
      return void post({ type: "toast", text: "Copied" });
    case "settings":
      return vscode.commands.executeCommand("cohors.openSettings");
    case "useWorkspaceRoots":
      return vscode.commands.executeCommand("cohors.useWorkspaceRoots");
    case "installHelp":
      return vscode.commands.executeCommand("cohors.installHelp");
    case "openFolder":
      return vscode.commands.executeCommand("workbench.action.files.openFolder");
    default:
      return;
  }
}

/* ───────── sidebar: the activity-bar view ───────── */

// A small native tree: what to do next and where work is at risk. It is a
// way into the dashboard, not a second dashboard.
class FleetTree {
  constructor() {
    this.changed = new vscode.EventEmitter();
    this.onDidChangeTreeData = this.changed.event;
  }
  getTreeItem(node) { return node; }
  getChildren(node) {
    if (!lastData) { if (!refreshing) refresh(true); return []; }
    if (lastData.status !== "ok") return [];
    if (node) return node.children || [];
    const o = lastData.overview;

    // Shape carries the severity; the colour only reinforces it.
    const icon = (sev) => sev === "risk" ? new vscode.ThemeIcon("warning", new vscode.ThemeColor("list.errorForeground"))
      : sev === "warn" ? new vscode.ThemeIcon("debug-breakpoint-log", new vscode.ThemeColor("list.warningForeground"))
      : new vscode.ThemeIcon("circle-outline");
    const leaf = (label, description, tooltip, sev, reveal) => {
      const it = new vscode.TreeItem(label, vscode.TreeItemCollapsibleState.None);
      it.description = description;
      it.tooltip = tooltip;
      it.iconPath = icon(sev);
      it.command = { command: "cohors.show", title: "Open dashboard", arguments: [reveal] };
      return it;
    };
    const group = (label, children, emptyText) => {
      const g = new vscode.TreeItem(label, vscode.TreeItemCollapsibleState.Expanded);
      g.description = String(children.length);
      g.children = children.length ? children : [new vscode.TreeItem(emptyText)];
      return g;
    };

    const atRisk = o.repos.filter((r) => r.at_risk);
    return [
      group("Do next",
        o.actions.map((a) => leaf(a.title, "", a.why, a.severity, { page: "overview" })),
        "Nothing needs doing"),
      group("Work at risk",
        atRisk.map((r) => leaf(r.name, (r.reasons || [])[0] || "", (r.reasons || []).join(", "), r.severity, { page: "repos", repo: r.id })),
        "No local work is at risk"),
    ];
  }
}

// Which welcome text the sidebar shows when it has no tree to draw.
function syncSidebar() {
  const d = lastData;
  const state = !d ? "loading"
    : d.status !== "ok" ? d.status
    : d.overview.repos.length ? "ok" : "empty";
  vscode.commands.executeCommand("setContext", "cohors.state", state);
  if (tree) tree.changed.fire();
}

/* ───────── lifecycle ───────── */

// CSS hot-swaps in place, a webview script reloads the webview (its state
// survives via getState), host code restarts the extension host. Development
// only: this is the one place a wholesale reload is acceptable.
function devReload(context) {
  const w = vscode.workspace.createFileSystemWatcher(
    new vscode.RelativePattern(vscode.Uri.file(EXT_DIR), "{media,src}/*.{css,js}"));
  let t = null;
  const pending = new Set();
  const flush = () => {
    const files = [...pending]; pending.clear();
    if (files.some((f) => f.includes(`${path.sep}src${path.sep}`))) {
      // Every window runs this watcher; only the one you are looking at restarts.
      if (!vscode.window.state.focused) return;
      return vscode.commands.executeCommand("workbench.action.restartExtensionHost");
    }
    if (!panel) return;
    if (files.some((f) => f.endsWith(".js"))) { panel.webview.html = html(panel.webview); return; }
    files.forEach((f) => post({ type: "dev-css", file: path.basename(f) }));
  };
  w.onDidChange((u) => { pending.add(u.fsPath); clearTimeout(t); t = setTimeout(flush, 150); });
  context.subscriptions.push(w);
}

function activate(context) {
  ctx = context;
  if (context.extensionMode === vscode.ExtensionMode.Development) devReload(context);

  tree = new FleetTree();
  syncSidebar();

  context.subscriptions.push(
    vscode.window.registerTreeDataProvider("cohors.fleet", tree),
    vscode.workspace.registerTextDocumentContentProvider(PROMPT_SCHEME, promptProvider),
    vscode.commands.registerCommand("cohors.show", show),
    vscode.commands.registerCommand("cohors.refresh", () => refresh(true)),
    vscode.commands.registerCommand("cohors.fetchAll", () => runVerb("fetch", { all: true })),
    vscode.commands.registerCommand("cohors.setupRepo", async () => {
      const here = workspaceFolders()[0];
      if (!here) return void fail(new Error("open a folder first."));
      const runner = await pickRunner();
      if (runner) await launch("setup-repo", { repo: here, runner });
    }),
    vscode.commands.registerCommand("cohors.planDay", async () => {
      const runner = await pickRunner();
      if (runner) await launch("plan-day", { runner });
    }),
    vscode.commands.registerCommand("cohors.useWorkspaceRoots", async () => {
      const folders = workspaceFolders();
      if (!folders.length) return void fail(new Error("open a folder first."));
      await cfg().update("roots", folders, vscode.ConfigurationTarget.Workspace);
      vscode.window.showInformationMessage(
        `cohors now scans ${folders.length === 1 ? "this workspace's folder" : `this workspace's ${folders.length} folders`}. Clear cohors.roots in the workspace settings to go back to your cohors config.`);
    }),
    vscode.commands.registerCommand("cohors.showContract", async () => {
      try {
        const doc = await vscode.workspace.openTextDocument({ language: "json", content: await cli.contract() });
        await vscode.window.showTextDocument(doc, { preview: true });
      } catch (e) { fail(e); }
    }),
    vscode.commands.registerCommand("cohors.openSettings", () =>
      vscode.commands.executeCommand("workbench.action.openSettings", `@ext:${context.extension.id}`)),
    vscode.commands.registerCommand("cohors.installHelp", () =>
      vscode.env.openExternal(vscode.Uri.parse(INSTALL_URL))),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (!e.affectsConfiguration("cohors")) return;
      cli.reset(); agentInfo = null; versionCache = null;
      armTimer();
      refresh(true);
    }),
    vscode.workspace.onDidGrantWorkspaceTrust(() => refresh(true)),
  );

  // Watchers, so the dashboard reflects reality without waiting for a poll.
  // A bare "**/…" glob only covers the OPEN workspace folders; anything
  // outside them needs an absolute RelativePattern base.
  const watch = (pattern, label) => {
    try {
      const w = vscode.workspace.createFileSystemWatcher(pattern);
      const go = () => { if (panel) nudge(); };
      w.onDidCreate(go); w.onDidChange(go); w.onDidDelete(go);
      context.subscriptions.push(w);
    } catch (e) { console.error(`cohors: could not watch ${label}`, e); }
  };
  // A repository's launchpad file, in any open workspace folder.
  watch("**/.cohors/*.json", "workspace launchpad files");
  // The cross-repo focus board, which lives in the home directory (or where
  // COHORS_HOME says, the same override cohors itself honours).
  const boardHome = process.env.COHORS_HOME || path.join(os.homedir(), ".cohors");
  watch(new vscode.RelativePattern(vscode.Uri.file(boardHome), "*.json"), "the focus board");
  // Branch switches and commits move HEAD; staging and committing move the index.
  for (const f of vscode.workspace.workspaceFolders || []) {
    watch(new vscode.RelativePattern(f, ".git/{HEAD,index}"), `${f.name} git state`);
  }

  // Did another window ask this one to start working here?
  const here = workspaceFolders()[0] || null;
  let pending = null;
  try { pending = Local.takePending(pendingDir(), here); } catch { /* nothing queued */ }
  if (pending && pending.kind === "start") launch("start", { repo: here, runner: pending.runner || undefined });

  const mode = cfg().get("openOnStartup", "emptyWindow");
  if (mode === "always" || (mode === "emptyWindow" && !workspaceFolders().length)) show();
}

function deactivate() { clearInterval(timer); clearTimeout(nudgeTimer); }

module.exports = { activate, deactivate };
