// Machine-local extras cohors does not report yet.
//
// Three small things, all read from this machine and never sent anywhere:
//   1. task -> conversation links in a repo's gitignored .principia/local.json,
//      so a task's button can say "Resume" instead of "Start";
//   2. Claude Code session history for a repository that has opted in;
//   3. the one-shot "open this folder, then start working there" intent that
//      has to survive a new window being created.
//
// No `vscode` here, so it can be checked from plain Node.

const fs = require("fs");
const os = require("os");
const path = require("path");

const HOME = os.homedir();

function readJson(p) { try { return JSON.parse(fs.readFileSync(p, "utf8")); } catch { return null; } }
const exists = (p) => { try { fs.accessSync(p); return true; } catch { return false; } };

/* ───────── 1. task -> conversation links ─────────
 *
 * A session id names a transcript on THIS machine, so it lives in the
 * gitignored local file, never in the committed repo.json:
 *   { "specVersion": 1, "threads": { "<task-id>": "<session-id>" } }
 * cohors writes it when it starts a linked session; this only reads it.
 */
function readThreads(repoPath) {
  const d = readJson(path.join(repoPath, ".principia", "local.json"));
  if (!d || d.specVersion !== 1 || !d.threads || typeof d.threads !== "object") return {};
  const out = {};
  for (const [task, session] of Object.entries(d.threads)) {
    if (typeof session === "string" && session) out[task] = session;
  }
  return out;
}

/* ───────── 2. Claude Code session history, gated by opt-in ─────────
 *
 * Claude Code writes each session's transcript to
 * ~/.claude/projects/<cwd with "/" as "-">/<session-id>.jsonl. That is real
 * conversation content, not a structural fact like a branch name, so the
 * caller only asks for a repository that has opted in by committing a
 * launchpad file. The gate is at the read, not just at the render.
 */
const CLAUDE_PROJECTS = path.join(HOME, ".claude", "projects");

// A transcript is append-only: if its mtime has not moved, the parse cannot
// have changed. This is what makes a short refresh interval affordable.
const sessionCache = new Map();   // file -> { mtime, parsed }

function parseSession(file, mtime) {
  const hit = sessionCache.get(file);
  if (hit && hit.mtime === mtime) return hit.parsed;

  let firstMsg = "", customTitle = "", branch = "", turns = 0;
  try {
    for (const line of fs.readFileSync(file, "utf8").split("\n")) {
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
        customTitle = o.customTitle;   // a later rename in the same file wins
      }
    }
  } catch { /* unreadable transcript: the caller still has id and time */ }

  const parsed = {
    title: (customTitle || firstMsg || "(untitled)").replace(/\s+/g, " ").trim().slice(0, 140),
    turns,
    branch,
  };
  sessionCache.set(file, { mtime, parsed });
  return parsed;
}

function claudeSessions(repoPath, max = 5) {
  const dir = path.join(CLAUDE_PROJECTS, repoPath.replace(/[\\/]/g, "-"));
  let files;
  try { files = fs.readdirSync(dir).filter((f) => f.endsWith(".jsonl")); } catch { return []; }

  return files.map((f) => {
    const p = path.join(dir, f);
    let mtime = 0;
    try { mtime = fs.statSync(p).mtimeMs; } catch { /* vanished between readdir and stat */ }
    return { id: f.replace(/\.jsonl$/, ""), file: p, mtime };
  })
    .sort((a, b) => b.mtime - a.mtime)
    .slice(0, max)
    .map((s) => ({
      id: s.id,
      ...parseSession(s.file, s.mtime),
      ts: s.mtime ? Math.floor(s.mtime / 1000) : null,
    }));
}

/* ───────── 3. "open it, then start working there" ─────────
 *
 * Opening a folder creates a WINDOW with its own extension host, which this
 * one cannot reach into to open a terminal afterwards. So the intent is left
 * on disk and the next window to activate in that folder claims it. One-shot
 * and short-lived: a stale intent must never fire days later.
 */
const PENDING_TTL_MS = 120000;

function queuePending(dir, root, action) {
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "pending.json"),
    JSON.stringify({ root, action, at: Date.now() }, null, 2) + "\n");
}

// Claim it exactly once, whether or not it turns out to be ours.
function takePending(dir, here) {
  const file = path.join(dir, "pending.json");
  const p = readJson(file);
  if (!p) return null;
  try { fs.unlinkSync(file); } catch { /* already claimed by another window */ }
  if (!p.root || !here) return null;
  if (Date.now() - (p.at || 0) > PENDING_TTL_MS) return null;
  const same = here === p.root || here.startsWith(p.root + path.sep) || p.root.startsWith(here + path.sep);
  return same ? (p.action || null) : null;
}

// A repo-declared file must stay inside that repo's .principia/ directory:
// `..` segments in a committed path must not reach the rest of the disk.
function resolveInside(repoPath, rel, sub) {
  if (typeof rel !== "string" || !rel) return null;
  const base = path.resolve(repoPath, sub || ".");
  const target = path.resolve(repoPath, rel);
  if (target !== base && !target.startsWith(base + path.sep)) return null;
  return target;
}

module.exports = { exists, readThreads, claudeSessions, queuePending, takePending, resolveInside, PENDING_TTL_MS };
