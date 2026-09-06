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
  const [branch, status, ab] = await Promise.all([
    run("git", ["-C", root, "rev-parse", "--abbrev-ref", "HEAD"]),
    run("git", ["-C", root, "status", "--porcelain"]),
    run("git", ["-C", root, "rev-list", "--left-right", "--count", "HEAD...@{upstream}"]),
  ]);
  const [ahead, behind] = ab.trim().split(/\s+/).map((n) => parseInt(n, 10) || 0);
  return {
    branch: branch.trim() || "",
    dirty: status ? status.split("\n").filter(Boolean).length : 0,
    ahead: ahead || 0, behind: behind || 0,
  };
}

// Staleness, per SPEC §7: a cached value is shown with its age or not at all.
async function stalenessOf(root, contribution) {
  if (!contribution.configured || !contribution.generatedFrom) return null;
  const out = await run("git", ["-C", root, "rev-list", "--count", `${contribution.generatedFrom}..HEAD`]);
  const n = parseInt(out.trim(), 10);
  return Number.isFinite(n) && n > 0 ? n : null;
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
      ws,
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

module.exports = {
  PRINCIPIA_HOME, exists, run,
  collectRepos, readBoard, readHistory, recentlyOpened,
};
