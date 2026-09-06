// Principia: the launchpad host.
//
// Ships empty. Everything on screen is discovered at runtime (discovery.js) or
// declared by a repository in its committed .principia/ directory. Nothing about
// any particular user appears in this file.

const vscode = require("vscode");
const fs = require("fs");
const os = require("os");
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
    here,
    contract: !!contractRoot(),
    counts: {
      repos: repos.length,
      configured: repos.filter((r) => r.configured).length,
      dirty: repos.filter((r) => r.dirty > 0).length,
      agents: repos.reduce((n, r) => n + r.agents.length, 0),
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
  const s = cfg().get("refreshSeconds", 120);
  // `active`, not `visible`: a tab in a split you are not looking at costs nothing.
  if (s > 0) timer = setInterval(() => { if (panel && panel.active) refresh(); }, s * 1000);
}

/* ───────── actions ───────── */

function terminal(cwd, cmd, name) {
  const t = vscode.window.createTerminal({ name: name || "principia", cwd });
  if (cmd) t.sendText(cmd);
  t.show(false);
  return t;
}

function openRepo(repo, newWindow) {
  const target = repo.ws && D.exists(repo.ws) ? repo.ws : repo.root;
  return vscode.commands.executeCommand("vscode.openFolder", vscode.Uri.file(target), { forceNewWindow: !!newWindow });
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

function substitute(body, repo) {
  return body
    .replace(/\{\{REPO_NAME\}\}/g, repo.name || "")
    .replace(/\{\{REPO_ROOT\}\}/g, repo.root || "")
    .replace(/\{\{BRANCH\}\}/g, repo.branch || "");
}

async function setupRepo(root, runnerId) {
  const repo = (lastData && lastData.repos.find((r) => r.root === root)) || { root, name: path.basename(root) };
  const cr = contractRoot();
  if (!cr) {
    vscode.window.showErrorMessage(
      "Principia: cannot find the contract. Set `principia.specPath` to a checkout of the Principia repo."
    );
    return;
  }
  const src = path.join(cr, "prompts", "setup-repo.md");
  if (!D.exists(src)) { vscode.window.showErrorMessage(`Principia: missing ${src}`); return; }

  const runner = pickRunner(runnerId);
  if (!runner) return;

  const body = [
    substitute(fs.readFileSync(src, "utf8"), repo),
    "",
    "---",
    "",
    `The Principia contract is checked out at: ${cr}`,
    `Read ${path.join(cr, "spec/v1/SPEC.md")} if you need the authoritative rules.`,
    `Validate with: node ${path.join(cr, "scripts/validate.js")} .`,
  ].join("\n");

  const file = promptFile(body, "setup");
  const cmd = R.commandFor(runner.id, file);
  R.recordLaunch({ runner: runner.id, repo: repo.root, branch: repo.branch, agent: "setup-repo" });
  terminal(repo.root, cmd, `${runner.label}: setup`);
}

async function runRepoAgent(root, agentId, runnerId) {
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

  const body = substitute(fs.readFileSync(file, "utf8"), repo);
  const p = promptFile(body, `agent-${agentId}`);
  R.recordLaunch({ runner: runner.id, repo: repo.root, branch: repo.branch, agent: agentId });
  terminal(repo.root, R.commandFor(runner.id, p), `${runner.label}: ${agent.label || agentId}`);
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
    case "agent":
      return runRepoAgent(m.root, m.id, m.runner);
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

  // A repo's contribution is a file in that repo; watch for it appearing.
  const watcher = vscode.workspace.createFileSystemWatcher("**/.principia/*.json");
  watcher.onDidCreate(() => refresh(true));
  watcher.onDidChange(() => refresh(true));
  watcher.onDidDelete(() => refresh(true));
  context.subscriptions.push(watcher);

  const mode = cfg().get("openOnStartup", "emptyWindow");
  const empty = !(vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders.length);
  if (mode === "always" || (mode === "emptyWindow" && empty)) show();
}

function deactivate() { clearInterval(timer); }

module.exports = { activate, deactivate };
