// Discovery: where the launchpad's content comes from.
//
// Deliberately NOT a filesystem scan. The editor already knows which projects
// this user actually opens, and in what order. That list is personal, ranked by
// real use, and costs nothing to read, so it is the seed for everything else.
// Nothing here is hardcoded to any user.

const vscode = require("vscode");
const fs = require("fs");
const path = require("path");
const os = require("os");
const cp = require("child_process");

const HOME = os.homedir();
const PRINCIPIA_HOME = path.join(HOME, ".principia");

const exists = (p) => { try { fs.accessSync(p); return true; } catch { return false; } };

function run(cmd, args, opts = {}) {
  return new Promise((resolve) => cp.execFile(cmd, args,
    { timeout: opts.timeout || 4000, cwd: opts.cwd, encoding: "utf8", maxBuffer: 1 << 20 },
    (err, stdout) => resolve(err ? "" : stdout)));
}

/* ───────── 1. what the editor already knows ───────── */

// `_workbench.getRecentlyOpened` is undocumented but stable across VS Code and
// its forks (Cursor, Windsurf). Reading globalStorage/storage.json is the
// fallback for builds that drop it.
async function recentlyOpened() {
  try {
    const r = await vscode.commands.executeCommand("_workbench.getRecentlyOpened");
    if (r && Array.isArray(r.workspaces)) {
      return r.workspaces.map((w) => {
        if (w.folderUri) return { uri: w.folderUri.toString(), kind: "folder" };
        if (w.workspace && w.workspace.configPath) return { uri: w.workspace.configPath.toString(), kind: "workspace" };
        return null;
      }).filter(Boolean);
    }
  } catch { /* fall through */ }
  return recentsFromStorage();
}

function recentsFromStorage() {
  // Same data, read directly. Path differs per product (Code, Cursor, ...), so
  // derive it from the running app rather than assuming "Code".
  const candidates = [];
  try {
    const base = vscode.env.appRoot || "";
    const product = path.basename(path.dirname(path.dirname(base))) || "Code";
    candidates.push(path.join(HOME, "Library/Application Support", product, "User/globalStorage/storage.json"));
  } catch { /* ignore */ }
  candidates.push(path.join(HOME, "Library/Application Support/Code/User/globalStorage/storage.json"));
  candidates.push(path.join(HOME, ".config/Code/User/globalStorage/storage.json"));

  for (const p of candidates) {
    if (!exists(p)) continue;
    try {
      const d = JSON.parse(fs.readFileSync(p, "utf8"));
      const entries = ((d["history.recentlyOpenedPathsList"] || {}).entries) || [];
      return entries.map((e) => {
        const uri = e.folderUri || (e.workspace && e.workspace.configPath);
        if (!uri || typeof uri !== "string") return null;
        return { uri, kind: e.workspace ? "workspace" : "folder" };
      }).filter(Boolean);
    } catch { /* try next */ }
  }
  return [];
}

function uriToPath(uri) {
  if (!uri.startsWith("file://")) return null;
  try { return decodeURIComponent(uri.slice(7)); } catch { return null; }
}

/* ───────── 2. auto-detection: useful before any config exists ───────── */

function readJson(p) { try { return JSON.parse(fs.readFileSync(p, "utf8")); } catch { return null; } }

function packageManager(root) {
  if (exists(path.join(root, "pnpm-lock.yaml"))) return "pnpm";
  if (exists(path.join(root, "bun.lockb")) || exists(path.join(root, "bun.lock"))) return "bun";
  if (exists(path.join(root, "yarn.lock"))) return "yarn";
  return "npm";
}

// Scripts worth surfacing. A repo with 30 npm scripts should not produce 30
// buttons; the long tail belongs in the terminal, not the dashboard.
const INTERESTING = ["dev", "start", "serve", "build", "test", "lint", "typecheck", "check"];
const SERVERY = /^(dev|start|serve|watch)$/;

function detectFlows(root, kind) {
  const flows = [];
  const add = (o) => flows.push({ ...o, source: "detected" });

  const pkg = readJson(path.join(root, "package.json"));
  if (pkg && pkg.scripts) {
    const pm = packageManager(root);
    for (const name of INTERESTING) {
      if (!pkg.scripts[name]) continue;
      add({
        id: `npm-${name}`, label: name, run: `${pm} run ${name}`,
        kind: SERVERY.test(name) ? "server" : name === "test" ? "test" : "build",
        background: SERVERY.test(name),
      });
    }
  }
  if (kind === "rust") {
    add({ id: "cargo-check", label: "cargo check", run: "cargo check", kind: "build" });
    add({ id: "cargo-test", label: "cargo test", run: "cargo test", kind: "test" });
    add({ id: "cargo-run", label: "cargo run", run: "cargo run", kind: "server", background: true });
  }
  if (kind === "py") {
    const uv = exists(path.join(root, "uv.lock"));
    add({ id: "py-sync", label: uv ? "uv sync" : "pip install -e .", run: uv ? "uv sync" : "pip install -e .", kind: "task" });
    if (exists(path.join(root, "tests")) || exists(path.join(root, "test")))
      add({ id: "py-test", label: "pytest", run: uv ? "uv run pytest" : "pytest", kind: "test" });
  }
  if (kind === "android") {
    add({ id: "gradle-assemble", label: "assembleDebug", run: "./gradlew assembleDebug", kind: "build" });
  }
  const mk = path.join(root, "Makefile");
  if (exists(mk)) {
    try {
      const body = fs.readFileSync(mk, "utf8");
      let n = 0;
      for (const m of body.matchAll(/^([a-zA-Z0-9_-]+):(?!=)/gm)) {
        if (m[1].startsWith(".") || n >= 5) continue;
        add({ id: `make-${m[1]}`, label: `make ${m[1]}`, run: `make ${m[1]}`, kind: "task" });
        n++;
      }
    } catch { /* unreadable Makefile: skip */ }
  }
  return flows;
}

// A repo may ship a .code-workspace that is the intended way to open it (folder
// layout, settings, multi-root). Recents only records `ws` when you happened to
// open the workspace file last time, so the file is found here too — otherwise
// opening a repo you last opened as a plain folder silently ignores it.
function workspaceFile(root) {
  try {
    const hit = fs.readdirSync(root).find((n) => n.endsWith(".code-workspace"));
    return hit ? path.join(root, hit) : null;
  } catch { return null; }
}

function repoKind(root) {
  if (exists(path.join(root, "Cargo.toml"))) return "rust";
  if (exists(path.join(root, "package.json"))) return "node";
  if (exists(path.join(root, "pyproject.toml")) || exists(path.join(root, "requirements.txt"))) return "py";
  if (exists(path.join(root, "settings.gradle.kts")) || exists(path.join(root, "settings.gradle"))) return "android";
  try {
    if (fs.readdirSync(root).some((n) => n.endsWith(".xcodeproj") || n === "Package.swift")) return "ios";
  } catch { /* unreadable dir */ }
  return "other";
}

const ICON_FOR = { node: "server-environment", rust: "gear", py: "beaker", android: "device-mobile", ios: "device-mobile", other: "repo" };

/* ───────── 3. the repo's own contribution ───────── */

function readContribution(root) {
  const file = path.join(root, ".principia", "repo.json");
  if (!exists(file)) return { configured: false };
  const doc = readJson(file);
  if (!doc) return { configured: false, invalid: "repo.json is not valid JSON" };
  if (doc.specVersion !== 1) return { configured: false, invalid: `unsupported specVersion ${doc.specVersion}` };
  return {
    configured: true,
    name: doc.name,
    summary: doc.summary || "",
    group: doc.group || "",
    icon: doc.icon || "",
    links: doc.links || [],
    flows: (doc.flows || []).map((f) => ({ ...f, source: "declared" })),
    agents: doc.agents || [],
    tasks: doc.tasks || [],
    updated: doc.updated || null,
    generatedFrom: doc.generatedFrom || null,
  };
}

/* ───────── 4. git, only for repos we are actually showing ───────── */

async function gitInfo(root) {
  const [branch, status, ab, log] = await Promise.all([
    run("git", ["-C", root, "rev-parse", "--abbrev-ref", "HEAD"]),
    run("git", ["-C", root, "status", "--porcelain"]),
    run("git", ["-C", root, "rev-list", "--left-right", "--count", "HEAD...@{upstream}"]),
    run("git", ["-C", root, "log", "-1", "--format=%cr%n%s"]),
  ]);
  const [when, subject] = log.split("\n");
  const [ahead, behind] = ab.trim().split(/\s+/).map((n) => parseInt(n, 10) || 0);
  return {
    branch: branch.trim() || "",
    dirty: status ? status.split("\n").filter(Boolean).length : 0,
    ahead: ahead || 0, behind: behind || 0,
    lastCommit: (when || "").trim(), lastSubject: (subject || "").trim(),
  };
}

// Staleness, per SPEC §7: a cached value is shown with its age or not at all.
async function stalenessOf(root, contribution) {
  if (!contribution.configured || !contribution.generatedFrom) return null;
  const out = await run("git", ["-C", root, "rev-list", "--count", `${contribution.generatedFrom}..HEAD`]);
  const n = parseInt(out.trim(), 10);
  return Number.isFinite(n) && n > 0 ? n : null;
}

/* ───────── 4a. machine-local state: .principia/local.json ─────────
 *
 * A session id names a transcript in ~/.claude/projects on THIS machine. Put it
 * in the committed repo.json and a teammate clones a pointer to a conversation
 * that does not exist for them, which is exactly what SPEC §2 forbids. So the
 * task->conversation links live here, in the gitignored local file the spec
 * already reserved for machine-local values.
 *
 *   { "specVersion": 1, "threads": { "<task-id>": "<session-id>" } }
 */

function localPath(root) { return path.join(root, ".principia", "local.json"); }

function readLocal(root) {
  const d = readJson(localPath(root));
  if (!d || d.specVersion !== 1) return { threads: {} };
  return { threads: d.threads || {} };
}

function writeThread(root, taskId, sessionId) {
  const file = localPath(root);
  const current = readJson(file) || {};
  const next = {
    ...current,
    specVersion: 1,
    threads: { ...(current.threads || {}), [taskId]: sessionId },
    updated: new Date().toISOString(),
  };
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify(next, null, 2) + "\n");
  return next;
}

/* ───────── 4b. real Claude Code session history, gated by opt-in ─────────
 *
 * Claude Code writes every session's full transcript to
 * ~/.claude/projects/<cwd with "/" as "-">/<session-id>.jsonl, no hook
 * required. That is much richer than anything this extension could log
 * itself, but it is also real conversation content, not a structural fact
 * like a branch name or an npm script. So unlike everything else in this
 * file, it is never read for a repo that has not opted in by committing
 * .principia/repo.json: `configured` is the explicit, per-repo permission
 * gate, and the caller in collectRepos() enforces it before this is ever
 * invoked, not just before it is displayed.
 */

const CLAUDE_PROJECTS = path.join(HOME, ".claude", "projects");

function projectSessionsDir(root) {
  return path.join(CLAUDE_PROJECTS, root.replace(/[\\/]/g, "-"));
}

// Transcripts run to megabytes, so parsing one on every refresh is what forced
// the old two-minute poll. A session file is append-only: if its mtime has not
// moved, the parse cannot have changed, so keep the last result. This is what
// makes a short refresh interval affordable.
const sessionCache = new Map();   // path -> { mtime, parsed }

function parseSession(file, mtime) {
  const hit = sessionCache.get(file);
  if (hit && hit.mtime === mtime) return hit.parsed;

  let firstMsg = "", customTitle = "", branch = "", turns = 0;
  try {
    const lines = fs.readFileSync(file, "utf8").split("\n");
    for (const line of lines) {
      if (!line) continue;
      let o;
      try { o = JSON.parse(line); } catch { continue; }
      if (o.type === "user" && !o.isSidechain) {
        turns++;
        if (!firstMsg) {
          const c = o.message && o.message.content;
          firstMsg = typeof c === "string" ? c
            : Array.isArray(c) ? ((c.find((x) => x.type === "text") || {}).text || "")
            : "";
        }
        if (!branch && o.gitBranch) branch = o.gitBranch;
      } else if (o.type === "custom-title" && o.customTitle) {
        customTitle = o.customTitle; // a later rename in the same file wins
      }
    }
  } catch { /* unreadable session: caller still has id and timestamp */ }

  const parsed = {
    title: (customTitle || firstMsg || "(untitled)").replace(/\s+/g, " ").trim().slice(0, 140),
    turns,
    branch,
  };
  sessionCache.set(file, { mtime, parsed });
  return parsed;
}

function claudeSessions(root, max = 5) {
  const dir = projectSessionsDir(root);
  if (!exists(dir)) return [];
  let files;
  try { files = fs.readdirSync(dir).filter((f) => f.endsWith(".jsonl")); } catch { return []; }

  const stated = files.map((f) => {
    const p = path.join(dir, f);
    let mtime = 0;
    try { mtime = fs.statSync(p).mtimeMs; } catch { /* skip */ }
    return { id: f.replace(/\.jsonl$/, ""), path: p, mtime };
  }).sort((a, b) => b.mtime - a.mtime).slice(0, max);

  return stated.map((s) => ({
    id: s.id,
    ...parseSession(s.path, s.mtime),
    ts: s.mtime ? new Date(s.mtime).toISOString() : null,
    path: s.path,
  }));
}

/* ───────── 5. put it together ───────── */

async function collectRepos(max) {
  const seen = new Set();
  const roots = [];
  for (const r of await recentlyOpened()) {
    const p = uriToPath(r.uri);
    if (!p) continue;
    const root = r.kind === "workspace" ? path.dirname(p) : p;
    if (seen.has(root) || !exists(root) || !exists(path.join(root, ".git"))) continue;
    seen.add(root);
    roots.push({ root, ws: r.kind === "workspace" ? p : null });
    if (roots.length >= max) break;
  }

  const gits = await Promise.all(roots.map((r) => gitInfo(r.root)));
  const out = [];
  for (let i = 0; i < roots.length; i++) {
    const { root, ws } = roots[i];
    const kind = repoKind(root);
    const contribution = readContribution(root);
    const detected = detectFlows(root, kind);
    out.push({
      root,
      // Prefer what recents recorded; fall back to a workspace file on disk.
      ws: ws || workspaceFile(root),
      dirName: path.basename(root),
      name: contribution.name || path.basename(root),
      summary: contribution.summary || "",
      group: contribution.group || "",
      icon: contribution.icon || ICON_FOR[kind] || "repo",
      kind,
      configured: contribution.configured,
      invalid: contribution.invalid || null,
      updated: contribution.updated || null,
      staleBy: await stalenessOf(root, contribution),
      // Declared flows win; detected ones fill the gap and are never persisted.
      flows: contribution.configured && contribution.flows.length ? contribution.flows : detected,
      detectedCount: detected.length,
      agents: contribution.agents || [],
      tasks: contribution.tasks || [],
      links: contribution.links || [],
      // Gated at the read, not just the render: an unconfigured repo's
      // transcripts are never touched, regardless of what the UI does.
      sessions: contribution.configured ? claudeSessions(root, 5) : [],
      // task id -> session id, machine-local (never committed)
      threads: readLocal(root).threads,
      rank: i,
      ...gits[i],
    });
  }
  return out;
}

/* ───────── 6. cross-repo board + history ───────── */

function readBoard() {
  const p = path.join(PRINCIPIA_HOME, "board.json");
  const d = readJson(p);
  if (!d || d.specVersion !== 1) return { focus: [], present: false };
  return { focus: d.focus || [], updated: d.updated || null, present: true };
}

function readHistory(days = 7) {
  const dir = path.join(PRINCIPIA_HOME, "history");
  if (!exists(dir)) return [];
  const out = [];
  let files = [];
  try { files = fs.readdirSync(dir).filter((f) => f.endsWith(".jsonl")).sort().reverse().slice(0, days); }
  catch { return []; }
  for (const f of files) {
    let lines = [];
    try { lines = fs.readFileSync(path.join(dir, f), "utf8").split("\n"); } catch { continue; }
    for (const line of lines) {
      if (!line.startsWith("{")) continue;
      try { out.push(JSON.parse(line)); } catch { /* skip malformed line */ }
    }
  }
  return out.sort((a, b) => String(b.ts).localeCompare(String(a.ts))).slice(0, 200);
}


/* ───────── 7. devices and dev servers (all optional, all best-effort) ───────── */

const SDK = path.join(HOME, "Library/Android/sdk");
const ADB = path.join(SDK, "platform-tools/adb");
const EMU = path.join(SDK, "emulator/emulator");

async function listeningPorts() {
  const out = await run("lsof", ["-iTCP", "-sTCP:LISTEN", "-P", "-n", "-Fpcn"]);
  if (!out) return [];
  const found = [];
  let pid = null, cmd = null;
  for (const line of out.split("\n")) {
    if (line[0] === "p") pid = parseInt(line.slice(1), 10);
    else if (line[0] === "c") cmd = line.slice(1);
    else if (line[0] === "n") {
      const m = line.match(/:(\d+)$/);
      if (m) found.push({ port: parseInt(m[1], 10), pid, cmd });
    }
  }
  const seen = new Map();
  for (const r of found) if (r.port >= 1024 && !seen.has(r.port)) seen.set(r.port, r);
  return [...seen.values()].sort((a, b) => a.port - b.port).slice(0, 40);
}

async function androidState() {
  if (!exists(EMU)) return { available: false, reason: "Android SDK not found", booted: false, avds: [] };
  const [dev, avd] = await Promise.all([run(ADB, ["devices"]), run(EMU, ["-list-avds"])]);
  const m = dev.match(/(emulator-\d+)\s+device/);
  return {
    available: true, reason: null,
    booted: !!m, device: m ? m[1] : null,
    avds: avd.split("\n").map((x) => x.trim()).filter(Boolean),
  };
}

async function iosState() {
  const out = await run("xcrun", ["simctl", "list", "devices", "available", "-j"], { timeout: 6000 });
  if (!out) return { available: false, reason: "Xcode command line tools not found", booted: [], all: [] };
  let j; try { j = JSON.parse(out); } catch { return { available: false, reason: "could not read simctl output", booted: [], all: [] }; }
  const all = [];
  for (const [rt, devs] of Object.entries(j.devices || {})) {
    if (!/iOS/.test(rt)) continue;
    for (const d of devs) all.push({ name: d.name, udid: d.udid, state: d.state });
  }
  all.sort((a, b) => ((b.state === "Booted") - (a.state === "Booted")) || a.name.localeCompare(b.name));
  return { available: true, reason: null, booted: all.filter((d) => d.state === "Booted"), all: all.slice(0, 30) };
}

// Each of these shells out (lsof, adb, simctl). Simulators and listening ports
// do not change on a one-second timescale, so a short TTL keeps a fast refresh
// interval from turning into three subprocess spawns per tick.
let deviceCache = { at: 0, value: null };
const DEVICE_TTL_MS = 10000;

async function devices(force) {
  if (!force && deviceCache.value && Date.now() - deviceCache.at < DEVICE_TTL_MS) {
    return deviceCache.value;
  }
  const [ports, android, ios] = await Promise.all([listeningPorts(), androidState(), iosState()]);
  deviceCache = { at: Date.now(), value: { ports, android, ios } };
  return deviceCache.value;
}

function resetDeviceCache() { deviceCache = { at: 0, value: null }; }

module.exports = {
  PRINCIPIA_HOME, exists, run,
  collectRepos, readBoard, readHistory, recentlyOpened, devices, ADB, EMU,
  readLocal, writeThread, resetDeviceCache,
};
