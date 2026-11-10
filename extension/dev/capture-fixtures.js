#!/usr/bin/env node
// Builds dev/fixtures/ from cohors's built-in demo fleet, so the preview and
// the checks never need (or leak) anybody's real repositories.
//
//   cohors web --demo --no-open --port 8787          # in the cohors checkout
//   curl -s localhost:8787/api/overview  > /tmp/overview.json
//   curl -s localhost:8787/api/launchpad > /tmp/launchpad.json
//   node dev/capture-fixtures.js /tmp/overview.json /tmp/launchpad.json
//
// The CLI itself has no `--demo` for `overview --json`, so the demo fleet is
// captured from the demo web server, which serves the same Overview model.
// That capture is then NORMALISED to look like real `cohors … --json` output,
// and a few things the demo fleet does not contain are INVENTED so every part
// of the UI has something to draw. Both are listed in dev/README.md.

const fs = require("fs");
const path = require("path");

const [overviewFile, launchpadFile] = process.argv.slice(2);
if (!overviewFile || !launchpadFile) {
  console.error("usage: node dev/capture-fixtures.js <overview.json> <launchpad.json>");
  process.exit(2);
}
const OUT = path.join(__dirname, "fixtures");
const HOME = "/home/dev";
const ROOT = `${HOME}/code`;

const overview = JSON.parse(fs.readFileSync(overviewFile, "utf8"));
const demoPad = JSON.parse(fs.readFileSync(launchpadFile, "utf8"));
const now = overview.now;
const HOUR = 3600, DAY = 86400;

// A stable 7-hex id from a string, so each repository gets its own commit id.
function sha(text) {
  let h = 0x811c9dc5;
  for (const ch of text) { h ^= ch.charCodeAt(0); h = Math.imul(h, 0x01000193) >>> 0; }
  return h.toString(16).padStart(8, "0").slice(0, 7);
}

/* ── overview: normalise ─────────────────────────────────────────────── */

// 1. Real output carries absolute paths and the scanned roots.
overview.roots = [ROOT];
for (const r of overview.repos) {
  if (r.path) r.path = r.path.replace(/^~\/code/, ROOT);
}
// 2. The demo launchpad names its entries `payments` while the demo fleet
//    uses `demo/payments`. In real output the two always share one id.
const idOf = new Map(overview.repos.map((r) => [r.name, r.id]));
const entries = (demoPad.entries || []).map((e) => ({ ...e, id: idOf.get(e.name) || e.id }));
for (const w of overview.work) w.id = idOf.get(w.name) || w.id;
// 3. With matching ids, each repository line knows about its tasks.
for (const r of overview.repos) {
  const e = entries.find((x) => x.id === r.id);
  const open = e && e.contribution ? e.contribution.tasks.filter((t) => t.status !== "done") : [];
  r.configured = !!(e && e.configured);
  r.tasks_open = open.length;
  r.tasks_high = open.filter((t) => t.priority === "high").length;
}
// 4. The demo fleet gives every commit the same id and no fetch time.
overview.repos.forEach((r, i) => {
  if (r.commit) r.commit.short_id = sha(r.name);
  if (r.has_remote && !r.error) r.fetched = now - (2 + i * 5) * HOUR;
});

/* ── launchpad: reshape to `cohors launchpad --json`, then fill in ───── */

const entryOf = (name) => entries.find((e) => e.name === name);

// INVENTED: an agent, a link, a finished task and task notes, so those rows exist.
const web = entryOf("web-app");
if (web) {
  web.contribution.agents = [{
    id: "a11y-audit", label: "Accessibility audit",
    file: ".cohors/agents/a11y-audit.md", supports: ["claude-code", "codex"],
  }];
  web.contribution.tasks[0].agent = "a11y-audit";
  web.contribution.tasks[0].notes = "Tab order skips the card number field. Reproduce with the keyboard only.";
  web.contribution.flows.push({ id: "e2e", label: "End-to-end tests", run: "pnpm test:e2e", kind: "test" });
}
const pay = entryOf("payments");
if (pay) {
  pay.contribution.links = [{ label: "Runbook", url: "https://example.com/payments/runbook" }];
  pay.contribution.flows[0].port = 8080;
  pay.contribution.tasks.push({ id: "rotate-webhook-secret", title: "Rotate the webhook signing secret", status: "done", priority: "normal" });
}

// INVENTED: what detection would find in the repositories with no launchpad
// file. The real CLI lists every repository, configured or not.
const cargo = [
  { id: "cargo-check", label: "cargo check", run: "cargo check", kind: "build", source: "detected" },
  { id: "cargo-test", label: "cargo test", run: "cargo test", kind: "test", source: "detected" },
];
const npm = (pm) => [
  { id: "npm-dev", label: "dev", run: `${pm} run dev`, kind: "server", background: true, source: "detected" },
  { id: "npm-build", label: "build", run: `${pm} run build`, kind: "build", source: "detected" },
  { id: "npm-test", label: "test", run: `${pm} run test`, kind: "test", source: "detected" },
];
const DETECTED = {
  "cli-tools": cargo, "auth-service": cargo, "analytics": [{ id: "py-test", label: "pytest", run: "uv run pytest", kind: "test", source: "detected" }],
  "design-system": npm("pnpm"), "marketing-site": npm("npm"), "mobile-app": npm("yarn"),
};
const repos = overview.repos.map((r) => {
  const e = entries.find((x) => x.id === r.id) || { id: r.id, name: r.name, configured: false };
  return { ...e, detected_flows: e.configured ? [] : (DETECTED[r.name] || []) };
});

// INVENTED: a few agent-session events, shaped like the activity log.
const iso = (secs) => new Date(secs * 1000).toISOString().replace(/\.\d+Z$/, "Z");
const event = (agoSecs, phase, runner, repo, branch, agent) =>
  ({ ts: iso(now - agoSecs), phase, runner, repo: `${ROOT}/${repo}`, branch, agent: agent || "", source: phase === "launch" ? "cohors" : "" });
const recent_activity = [
  event(40 * 60, "end", "claude-code", "payments", "main"),
  event(2 * HOUR, "launch", "claude-code", "payments", "main", "task:idempotency-keys"),
  event(5 * HOUR, "start", "codex", "web-app", "feat/checkout"),
  event(26 * HOUR, "launch", "claude-code", "web-app", "feat/checkout", "a11y-audit"),
  event(2 * DAY, "launch", "claude-code", "data-pipeline", "main", "setup-repo"),
  event(3 * DAY, "end", "gemini-cli", "design-system", "main"),
];

const launchpad = {
  summary: {
    repos: repos.length,
    configured: repos.filter((e) => e.configured).length,
    open_tasks: overview.figures.open_tasks,
  },
  repos,
  board: demoPad.board,
  recent_activity,
};

/* ── standup: INVENTED (the demo fleet has no real history to read) ──── */

const SUBJECTS = [
  "fix: retry the charge when the gateway times out", "feat: idempotency keys on the retry path",
  "test: cover the duplicate-webhook case", "chore: bump the base image", "refactor: one error type for the worker",
  "docs: how to run the nightly job locally", "fix: keep focus inside the checkout dialog",
  "feat: show the saved card first", "perf: stop re-parsing the manifest on every run",
  "fix: handle an empty export without failing", "ci: cache the dependency install", "feat: dry-run flag for the importer",
];
function standup(windowLabel, days) {
  let n = 0;
  const groups = overview.commits.slice(0, days === 1 ? 2 : 5).map((c) => {
    const count = days === 1 ? Math.min(2, c.count) : Math.min(4, c.count);
    const commits = Array.from({ length: count }, (_, i) => {
      n++;
      return {
        repo: c.name,
        short_id: sha(`${c.name}-${windowLabel}-${i}`),
        summary: SUBJECTS[(n * 7) % SUBJECTS.length],
        timestamp: now - Math.round(((i + 1) * days * DAY) / (count + 1)) - n * 600,
      };
    });
    return { repo: c.name, commits };
  });
  return { window: windowLabel, author: "dev@example.com", total: groups.reduce((t, g) => t + g.commits.length, 0), groups };
}

/* ── agent contract, machine-local extras, devices: INVENTED ─────────── */

const agent = {
  runners: [
    { id: "claude-code", label: "Claude Code", bin: "claude", available: true, path: `${HOME}/.local/bin/claude`, reason: null, can_link: true, can_resume: true },
    { id: "codex", label: "Codex", bin: "codex", available: true, path: "/usr/local/bin/codex", reason: null, can_link: false, can_resume: true },
    { id: "gemini-cli", label: "Gemini CLI", bin: "gemini", available: false, path: null, reason: "gemini was not found on your PATH", can_link: true, can_resume: false },
    { id: "agy", label: "agy", bin: "agy", available: false, path: null, reason: "agy was not found on your PATH", can_link: false, can_resume: true },
  ],
  prompts: [
    { id: "setup-repo", title: "Set up this repository", summary: "Read the repository and write its launchpad file: flows, agents and current tasks.", needs_repo: true },
    { id: "declare-flows", title: "Declare flows", summary: "Name the commands worth launching that detection cannot infer.", needs_repo: true },
    { id: "add-agent", title: "Add an agent", summary: "Ship a prompt for recurring, repository-specific work.", needs_repo: true },
    { id: "track-tasks", title: "Track tasks", summary: "Turn uncommitted work, TODOs and failing tests into short-lived tasks.", needs_repo: true },
    { id: "plan-day", title: "Plan my day", summary: "Look across every repository and write the focus board.", needs_repo: false },
  ],
  error: null,
};

const local = {
  threads: { [idOf.get("payments")]: { "idempotency-keys": "6f1c2b7e-3a54-4d1e-9c0f-2b8a7d5e4c31" } },
  sessions: {
    [idOf.get("payments")]: [
      { id: "6f1c2b7e-3a54-4d1e-9c0f-2b8a7d5e4c31", title: "Idempotency keys on charge retries", turns: 14, branch: "main", ts: now - 2 * HOUR },
      { id: "0d9e8c7b-6a5f-4e3d-8c2b-1a0f9e8d7c6b", title: "Why does the worker drop the last batch?", turns: 6, branch: "main", ts: now - 3 * DAY },
    ],
  },
};

const devices = {
  at: now,
  android: { available: true, reason: null, booted: false, device: null, avds: ["Pixel_8_API_35"] },
  ios: {
    available: true, reason: null,
    booted: [{ name: "iPhone 16", udid: "8A1B2C3D-0000-4000-8000-000000000001", state: "Booted" }],
    all: [
      { name: "iPhone 16", udid: "8A1B2C3D-0000-4000-8000-000000000001", state: "Booted" },
      { name: "iPad Air 11-inch", udid: "8A1B2C3D-0000-4000-8000-000000000002", state: "Shutdown" },
      { name: "iPhone SE (3rd generation)", udid: "8A1B2C3D-0000-4000-8000-000000000003", state: "Shutdown" },
    ],
  },
  ports: [{ port: 3000, pid: 4101, cmd: "node" }, { port: 5173, pid: 4188, cmd: "node" }, { port: 8080, pid: 4230, cmd: "payments" }],
};

// What the extension host adds around the CLI output before sending it on.
const host = {
  generatedAt: now,
  home: HOME,
  here: [`${ROOT}/payments`],
  trusted: true,
  rootsSetting: [],
  binary: { path: `${HOME}/.cargo/bin/cohors`, version: "cohors 0.6.0" },
};

fs.mkdirSync(OUT, { recursive: true });
const write = (name, value) => fs.writeFileSync(path.join(OUT, name), JSON.stringify(value, null, 2) + "\n");
write("overview.json", overview);
write("launchpad.json", launchpad);
write("standup-week.json", standup("this week", 7));
write("standup-today.json", standup("today", 1));
write("agent.json", agent);
write("local.json", local);
write("devices.json", devices);
write("host.json", host);
console.log(`wrote 8 fixtures to ${OUT}`);
