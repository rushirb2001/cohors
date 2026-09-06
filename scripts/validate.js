#!/usr/bin/env node
// Principia contract validator. No dependencies on purpose: an agent in any repo
// should be able to run this with nothing installed.
//
//   node scripts/validate.js [path-to-repo]     (defaults to cwd)
//
// Exits 0 when valid, 1 when not. Prints one finding per line.

const fs = require("fs");
const path = require("path");

const SUPPORTED_SPEC = [1];
const RUNNERS = ["claude-code", "codex", "gemini-cli", "agy"];
const KINDS = ["server", "task", "test", "build", "tool"];
const STATUS = ["todo", "doing", "blocked", "done"];
const PRIORITY = ["high", "normal", "low"];

const ID_RE = /^[a-z0-9]+(-[a-z0-9]+)*$/;
const ABS_RE = /^(\/|~|[A-Za-z]:)/;

const errors = [];
const warnings = [];
const err = (m) => errors.push(m);
const warn = (m) => warnings.push(m);

function uniqueIds(list, where) {
  const seen = new Set();
  for (const item of list) {
    if (!item || typeof item.id !== "string") continue;
    if (!ID_RE.test(item.id)) err(`${where}: id "${item.id}" must be kebab-case`);
    if (seen.has(item.id)) err(`${where}: duplicate id "${item.id}"`);
    seen.add(item.id);
  }
}

function main() {
  const root = path.resolve(process.argv[2] || process.cwd());
  const dir = path.join(root, ".principia");
  const file = path.join(dir, "repo.json");

  if (!fs.existsSync(dir)) {
    console.log(`no .principia/ in ${root}`);
    console.log("This repo has not opted in. That is valid: the launchpad will");
    console.log("still show it, auto-detect its flows, and offer agent setup.");
    return 0;
  }
  if (!fs.existsSync(file)) {
    err(".principia/ exists but repo.json is missing");
    return report();
  }

  let doc;
  try {
    doc = JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (e) {
    err(`repo.json is not valid JSON: ${e.message}`);
    return report();
  }

  // --- required ---
  if (!SUPPORTED_SPEC.includes(doc.specVersion)) {
    err(`specVersion must be one of ${SUPPORTED_SPEC.join(", ")}, got ${JSON.stringify(doc.specVersion)}`);
  }
  if (typeof doc.name !== "string" || !doc.name.trim()) err("name is required");
  if (doc.summary && doc.summary.length > 80) err(`summary is ${doc.summary.length} chars, max 80`);

  // --- flows ---
  const flows = doc.flows || [];
  if (!Array.isArray(flows)) err("flows must be an array");
  else {
    uniqueIds(flows, "flows");
    let primaries = 0;
    for (const f of flows) {
      const at = `flows[${f && f.id ? f.id : "?"}]`;
      if (!f || typeof f.run !== "string" || !f.run.trim()) err(`${at}: run is required`);
      else if (ABS_RE.test(f.run.trim())) err(`${at}: run must not start with an absolute path`);
      if (!f || typeof f.label !== "string") err(`${at}: label is required`);
      if (f && f.cwd && ABS_RE.test(f.cwd)) err(`${at}: cwd must be relative to the repo root`);
      if (f && f.kind && !KINDS.includes(f.kind)) err(`${at}: kind must be one of ${KINDS.join(", ")}`);
      if (f && f.port != null && (!Number.isInteger(f.port) || f.port < 1 || f.port > 65535)) err(`${at}: port must be 1-65535`);
      if (f && f.primary) primaries++;
    }
    if (primaries > 1) err(`only one flow may be primary, found ${primaries}`);
  }

  // --- agents ---
  const agents = doc.agents || [];
  if (!Array.isArray(agents)) err("agents must be an array");
  else {
    uniqueIds(agents, "agents");
    for (const a of agents) {
      const at = `agents[${a && a.id ? a.id : "?"}]`;
      // Runner checks run first and unconditionally: a missing prompt file must
      // not mask an unknown runner id.
      for (const r of (a && a.supports) || []) {
        if (!RUNNERS.includes(r)) err(`${at}: unknown runner "${r}" (known: ${RUNNERS.join(", ")})`);
      }
      if (!a || !a.supports || !a.supports.length) warn(`${at}: no supports[]; the launchpad cannot tell which runners can execute it`);

      if (!a || typeof a.file !== "string") { err(`${at}: file is required`); continue; }
      if (ABS_RE.test(a.file)) { err(`${at}: file must be relative to the repo root`); continue; }
      if (a.file.split(/[\\/]/).includes("..")) { err(`${at}: file must not contain ".."; keep it inside .principia/`); continue; }
      if (!/^\.principia[\\/]/.test(a.file)) { err(`${at}: file must live under .principia/`); continue; }
      const p = path.join(root, a.file);
      if (!fs.existsSync(p)) { err(`${at}: file not found: ${a.file}`); continue; }
      const body = fs.readFileSync(p, "utf8");
      if (!body.startsWith("---")) warn(`${at}: ${a.file} has no YAML frontmatter`);
    }
  }

  // --- tasks ---
  const tasks = doc.tasks || [];
  if (!Array.isArray(tasks)) err("tasks must be an array");
  else {
    uniqueIds(tasks, "tasks");
    for (const t of tasks) {
      const at = `tasks[${t && t.id ? t.id : "?"}]`;
      if (!t || typeof t.title !== "string" || !t.title.trim()) err(`${at}: title is required`);
      if (t && t.status && !STATUS.includes(t.status)) err(`${at}: status must be one of ${STATUS.join(", ")}`);
      if (t && t.priority && !PRIORITY.includes(t.priority)) err(`${at}: priority must be one of ${PRIORITY.join(", ")}`);
    }
  }

  // --- hygiene ---
  if (!doc.updated) warn("no updated timestamp; the dashboard cannot show how fresh this is");
  const gi = path.join(root, ".gitignore");
  if (fs.existsSync(gi) && !fs.readFileSync(gi, "utf8").includes(".principia/local.json")) {
    warn("add `.principia/local.json` to .gitignore: it is machine-local and must not be committed");
  }
  const local = path.join(dir, "local.json");
  if (fs.existsSync(local)) {
    try {
      const out = require("child_process").execFileSync("git", ["-C", root, "ls-files", "--error-unmatch", ".principia/local.json"], { stdio: ["ignore", "pipe", "ignore"] });
      if (String(out).trim()) err("local.json is committed; it holds machine-local values and must be gitignored");
    } catch { /* not tracked: correct */ }
  }

  return report();
}

function report() {
  for (const w of warnings) console.log(`warn   ${w}`);
  for (const e of errors) console.log(`ERROR  ${e}`);
  if (!errors.length && !warnings.length) console.log("valid");
  else console.log(`\n${errors.length} error(s), ${warnings.length} warning(s)`);
  return errors.length ? 1 : 0;
}

process.exit(main());
