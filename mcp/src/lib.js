// Pure logic shared by the MCP server. Deliberately has ZERO dependency on
// `vscode` (unlike extension/src/discovery.js, which needs it for the live
// _workbench.getRecentlyOpened command) so this runs standalone in any agent
// runner's process. Falls back to reading storage.json directly, the same
// file the VS Code extension itself reads as its own fallback path.

const fs = require("fs");
const os = require("os");
const path = require("path");
const cp = require("child_process");

const HOME = os.homedir();
const PRINCIPIA_HOME = path.join(HOME, ".principia");

const exists = (p) => { try { fs.accessSync(p); return true; } catch { return false; } };
const readJson = (p) => { try { return JSON.parse(fs.readFileSync(p, "utf8")); } catch { return null; } };

function run(cmd, args, opts = {}) {
  return new Promise((resolve) => cp.execFile(cmd, args,
    { timeout: opts.timeout || 4000, cwd: opts.cwd, encoding: "utf8", maxBuffer: 1 << 20 },
    (err, stdout) => resolve(err ? "" : stdout)));
}

/* ───────── recents, read directly (no vscode command available here) ───────── */

function recentsFromStorage() {
  const candidates = [
    path.join(HOME, "Library/Application Support/Code/User/globalStorage/storage.json"),
    path.join(HOME, "Library/Application Support/Cursor/User/globalStorage/storage.json"),
    path.join(HOME, ".config/Code/User/globalStorage/storage.json"),
  ];
  for (const p of candidates) {
    if (!exists(p)) continue;
    const d = readJson(p);
    if (!d) continue;
    const entries = ((d["history.recentlyOpenedPathsList"] || {}).entries) || [];
    const out = entries.map((e) => {
      const uri = e.folderUri || (e.workspace && e.workspace.configPath);
      if (!uri || typeof uri !== "string" || !uri.startsWith("file://")) return null;
      try { return decodeURIComponent(uri.slice(7)); } catch { return null; }
    }).filter(Boolean);
    if (out.length) return out;
  }
  return [];
}

async function gitInfo(root) {
  const [branch, status, ab, log] = await Promise.all([
    run("git", ["-C", root, "rev-parse", "--abbrev-ref", "HEAD"]),
    run("git", ["-C", root, "status", "--porcelain"]),
    run("git", ["-C", root, "rev-list", "--left-right", "--count", "HEAD...@{upstream}"]),
    run("git", ["-C", root, "log", "-1", "--format=%cr%n%s"]),
  ]);
  const [ahead, behind] = ab.trim().split(/\s+/).map((n) => parseInt(n, 10) || 0);
  const [when, subject] = log.split("\n");
  return {
    branch: branch.trim() || "", dirty: status ? status.split("\n").filter(Boolean).length : 0,
    ahead: ahead || 0, behind: behind || 0,
    lastCommit: (when || "").trim(), lastSubject: (subject || "").trim(),
  };
}

function readContribution(root) {
  const file = path.join(root, ".principia", "repo.json");
  if (!exists(file)) return { configured: false };
  const doc = readJson(file);
  if (!doc) return { configured: false, invalid: "repo.json is not valid JSON" };
  if (doc.specVersion !== 1) return { configured: false, invalid: `unsupported specVersion ${doc.specVersion}` };
  return { configured: true, ...doc };
}

async function listRepos({ maxRepos = 40 } = {}) {
  const seen = new Set();
  const roots = [];
  for (const p of recentsFromStorage()) {
    const root = exists(path.join(p, ".git")) ? p : null;
    if (!root || seen.has(root)) continue;
    seen.add(root);
    roots.push(root);
    if (roots.length >= maxRepos) break;
  }
  const gits = await Promise.all(roots.map(gitInfo));
  return roots.map((root, i) => {
    const c = readContribution(root);
    return {
      root, name: c.name || path.basename(root),
      configured: c.configured, invalid: c.invalid || null,
      summary: c.summary || "", group: c.group || "",
      flows: c.flows || [], agents: c.agents || [], tasks: c.tasks || [],
      ...gits[i],
    };
  });
}

/* ───────── validation, shared with scripts/validate.js's rules ───────── */

const ID_RE = /^[a-z0-9]+(-[a-z0-9]+)*$/;
const ABS_RE = /^(\/|~|[A-Za-z]:)/;
const RUNNERS = ["claude-code", "codex", "gemini-cli", "agy"];
const KINDS = ["server", "task", "test", "build", "tool"];
const STATUS = ["todo", "doing", "blocked", "done"];
const PRIORITY = ["high", "normal", "low"];

function uniqueIds(list, where, errors) {
  const seen = new Set();
  for (const item of list || []) {
    if (!item || typeof item.id !== "string") continue;
    if (!ID_RE.test(item.id)) errors.push(`${where}: id "${item.id}" must be kebab-case`);
    if (seen.has(item.id)) errors.push(`${where}: duplicate id "${item.id}"`);
    seen.add(item.id);
  }
}

// Mirrors scripts/validate.js exactly (kept in sync by hand; both read the
// same spec/v1/repo.schema.json rules). Returns { errors, warnings }, never
// throws, so a tool call always gets a structured answer back.
function validateRepoDoc(doc, root) {
  const errors = [], warnings = [];
  if (doc.specVersion !== 1) errors.push(`specVersion must be 1, got ${JSON.stringify(doc.specVersion)}`);
  if (typeof doc.name !== "string" || !doc.name.trim()) errors.push("name is required");
  if (doc.summary && doc.summary.length > 80) errors.push(`summary is ${doc.summary.length} chars, max 80`);

  const flows = doc.flows || [];
  uniqueIds(flows, "flows", errors);
  let primaries = 0;
  for (const f of flows) {
    const at = `flows[${f && f.id ? f.id : "?"}]`;
    if (!f || typeof f.run !== "string" || !f.run.trim()) errors.push(`${at}: run is required`);
    else if (ABS_RE.test(f.run.trim())) errors.push(`${at}: run must not start with an absolute path`);
    if (!f || typeof f.label !== "string") errors.push(`${at}: label is required`);
    if (f && f.cwd && ABS_RE.test(f.cwd)) errors.push(`${at}: cwd must be relative to the repo root`);
    if (f && f.kind && !KINDS.includes(f.kind)) errors.push(`${at}: kind must be one of ${KINDS.join(", ")}`);
    if (f && f.primary) primaries++;
  }
  if (primaries > 1) errors.push(`only one flow may be primary, found ${primaries}`);

  const agents = doc.agents || [];
  uniqueIds(agents, "agents", errors);
  for (const a of agents) {
    const at = `agents[${a && a.id ? a.id : "?"}]`;
    for (const r of (a && a.supports) || []) {
      if (!RUNNERS.includes(r)) errors.push(`${at}: unknown runner "${r}"`);
    }
    if (!a || typeof a.file !== "string") { errors.push(`${at}: file is required`); continue; }
    if (ABS_RE.test(a.file)) { errors.push(`${at}: file must be relative to the repo root`); continue; }
    if (a.file.split(/[\\/]/).includes("..")) { errors.push(`${at}: file must not contain ".."`); continue; }
    if (!/^\.principia[\\/]/.test(a.file)) { errors.push(`${at}: file must live under .principia/`); continue; }
    if (root && !exists(path.join(root, a.file))) errors.push(`${at}: file not found: ${a.file}`);
  }

  const tasks = doc.tasks || [];
  uniqueIds(tasks, "tasks", errors);
  for (const t of tasks) {
    const at = `tasks[${t && t.id ? t.id : "?"}]`;
    if (!t || typeof t.title !== "string" || !t.title.trim()) errors.push(`${at}: title is required`);
    if (t && t.status && !STATUS.includes(t.status)) errors.push(`${at}: status must be one of ${STATUS.join(", ")}`);
    if (t && t.priority && !PRIORITY.includes(t.priority)) errors.push(`${at}: priority must be one of ${PRIORITY.join(", ")}`);
  }

  if (!doc.updated) warnings.push("no updated timestamp");
  return { errors, warnings };
}

function validateBoardDoc(doc) {
  const errors = [];
  if (doc.specVersion !== 1) errors.push(`specVersion must be 1, got ${JSON.stringify(doc.specVersion)}`);
  uniqueIds(doc.focus || [], "focus", errors);
  for (const f of doc.focus || []) {
    if (!f || typeof f.title !== "string" || !f.title.trim()) errors.push(`focus[${f && f.id}]: title is required`);
    if (f && f.status && !STATUS.includes(f.status)) errors.push(`focus[${f.id}]: invalid status`);
  }
  return { errors, warnings: [] };
}

/* ───────── writes: always validate first, never write on error ───────── */

function writeRepoContribution(root, doc) {
  doc = { specVersion: 1, ...doc, updated: new Date().toISOString() };
  const { errors, warnings } = validateRepoDoc(doc, root);
  if (errors.length) return { written: false, errors, warnings };
  const dir = path.join(root, ".principia");
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "repo.json"), JSON.stringify(doc, null, 2) + "\n");
  const gi = path.join(root, ".gitignore");
  const line = ".principia/local.json";
  if (exists(gi)) {
    const body = fs.readFileSync(gi, "utf8");
    if (!body.includes(line)) fs.appendFileSync(gi, (body.endsWith("\n") ? "" : "\n") + line + "\n");
  } else {
    fs.writeFileSync(gi, line + "\n");
  }
  return { written: true, errors: [], warnings, path: path.join(dir, "repo.json") };
}

function readBoard() {
  const p = path.join(PRINCIPIA_HOME, "board.json");
  const doc = readJson(p);
  if (!doc) return { specVersion: 1, focus: [] };
  return doc;
}

function writeBoard(doc) {
  doc = { specVersion: 1, ...doc, updated: new Date().toISOString() };
  const { errors, warnings } = validateBoardDoc(doc);
  if (errors.length) return { written: false, errors, warnings };
  fs.mkdirSync(PRINCIPIA_HOME, { recursive: true });
  fs.writeFileSync(path.join(PRINCIPIA_HOME, "board.json"), JSON.stringify(doc, null, 2) + "\n");
  return { written: true, errors: [], warnings, path: path.join(PRINCIPIA_HOME, "board.json") };
}

function recordActivity({ runner, repo, branch, agent, phase }) {
  const dir = path.join(PRINCIPIA_HOME, "history");
  fs.mkdirSync(dir, { recursive: true });
  const day = new Date().toISOString().slice(0, 10);
  const rec = {
    ts: new Date().toISOString(), phase: phase || "note",
    runner: runner || "unknown", repo: repo || "", branch: branch || "",
    agent: agent || "", source: "principia-mcp",
  };
  fs.appendFileSync(path.join(dir, `${day}.jsonl`), JSON.stringify(rec) + "\n");
  return rec;
}

module.exports = {
  HOME, PRINCIPIA_HOME, exists, readJson,
  listRepos, gitInfo, readContribution,
  validateRepoDoc, validateBoardDoc,
  writeRepoContribution, readBoard, writeBoard, recordActivity,
};
