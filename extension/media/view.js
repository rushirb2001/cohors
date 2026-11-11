// cohors dashboard: the pure half of the webview.
//
// Every function here takes data and returns HTML strings. Nothing touches
// the DOM, so the whole file runs under plain Node (see dev/check.js) as well
// as in the webview, where main.js puts the strings on screen.
//
// A page is a list of sections, each `{ id, html }`. main.js compares each
// section with what is already painted and replaces only the ones that
// changed, which is what keeps scroll, focus and the caret alive across a
// background refresh.
//
// Rules:
//  - Numbers are cohors's. The figures, "Do next" and the per-repository
//    judgments come from `cohors overview --json` and are only laid out here.
//  - Severity is a shape plus words (triangle, diamond, ring), never colour
//    alone.
//  - Every string that came from a repository or the disk goes through esc().

(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.CohorsView = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  "use strict";

  /* ───────── small helpers ───────── */

  const ESC = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };
  const esc = (s) => String(s == null ? "" : s).replace(/[&<>"']/g, (c) => ESC[c]);

  // A bundled codicon. The name is sanitised because some come from data.
  const ico = (name, cls) =>
    `<span class="codicon codicon-${String(name || "").replace(/[^a-z0-9-]/g, "")}${cls ? " " + cls : ""}" aria-hidden="true"></span>`;

  const count = (n) => Number(n || 0).toLocaleString("en-US");
  const plural = (n, one, many) => `${count(n)} ${Number(n) === 1 ? one : (many || one + "s")}`;
  const cap = (s) => (s ? String(s)[0].toUpperCase() + String(s).slice(1) : "");
  const baseName = (p) => String(p || "").split(/[\\/]/).filter(Boolean).pop() || "";

  // `~/code/app` instead of the full home path, when it is under home.
  function tilde(p, home) {
    p = String(p || "");
    if (home && (p === home || p.startsWith(home + "/") || p.startsWith(home + "\\"))) return "~" + p.slice(home.length);
    return p;
  }

  // 3h, 5d, 6w, 4mo, 2y: how long ago `ts` was, as of `now` (Unix seconds).
  function age(ts, now) {
    if (!ts || !now) return "";
    const d = Math.max(0, now - ts) / 86400;
    if (d < 1) return Math.max(1, Math.round(d * 24)) + "h";
    if (d < 14) return Math.round(d) + "d";
    if (d < 70) return Math.round(d / 7) + "w";
    if (d < 730) return Math.round(d / 30) + "mo";
    return Math.round(d / 365) + "y";
  }
  const ago = (ts, now) => { const a = age(ts, now); return a ? `${a} ago` : ""; };

  function clock(ts) {
    if (!ts) return "";
    const d = new Date(ts * 1000);
    const p = (n) => String(n).padStart(2, "0");
    return `${p(d.getHours())}:${p(d.getMinutes())}`;
  }

  /* ───────── the brand mark ─────────
   * Drawn from three rows of block characters: each character is a 2x2 grid
   * of cells, and a shaded block is the same cell at reduced opacity. The
   * same rows draw the mark in the terminal UI.
   */
  const MARK_ROWS = ["▜▒▟███▙▒▛", "▟██▌█▐██▙", "▀▐▖▀█▀▗▌▀"];
  // [upper-left, upper-right, lower-left, lower-right]
  const QUADS = {
    "█": [1, 1, 1, 1], "▒": [1, 1, 1, 1], "▀": [1, 1, 0, 0], "▄": [0, 0, 1, 1],
    "▌": [1, 0, 1, 0], "▐": [0, 1, 0, 1], "▜": [1, 1, 0, 1], "▛": [1, 1, 1, 0],
    "▟": [0, 1, 1, 1], "▙": [1, 0, 1, 1], "▖": [0, 0, 1, 0], "▗": [0, 0, 0, 1],
    "▘": [1, 0, 0, 0], "▝": [0, 1, 0, 0],
  };
  function markRects() {
    const out = [];
    MARK_ROWS.forEach((row, y) => [...row].forEach((ch, x) => {
      const q = QUADS[ch];
      if (!q) return;
      q.forEach((on, i) => {
        if (on) out.push(`<rect x="${x * 2 + (i % 2)}" y="${y * 4 + (i >> 1) * 2}" width="1.04" height="2.04"${ch === "▒" ? ' opacity=".45"' : ""}/>`);
      });
    }));
    return out.join("");
  }
  // `fill` and `viewBox` are overridable so the same drawing makes the icon files.
  function markSvg(o) {
    o = o || {};
    return `<svg${o.xmlns ? ' xmlns="http://www.w3.org/2000/svg"' : ""}${o.cls === false ? "" : ' class="mk"'} viewBox="${o.viewBox || "0 0 18 12"}" fill="${o.fill || "currentColor"}"${o.xmlns ? "" : ' aria-hidden="true"'}>${markRects()}</svg>`;
  }

  /* ───────── severity: shape plus words ───────── */

  const SEVERITY = {
    risk:   { cls: "risk", label: "At risk" },
    warn:   { cls: "warn", label: "Needs attention" },
    notice: { cls: "note", label: "Worth a look" },
    info:   { cls: "note", label: "For information" },
    ok:     { cls: "ok",   label: "Clean" },
  };
  function mark(sev) {
    const s = SEVERITY[sev] || SEVERITY.ok;
    return `<span class="mark ${s.cls}" role="img" aria-label="${s.label}" title="${s.label}"></span>`;
  }

  // The severity block that leads a row in the lists a person scans for
  // trouble (Do next, Work at risk, the repository tables): a filled RISK or
  // WARN, a quiet outlined NOTE or INFO, and an empty slot of the same width
  // for a clean repository, so names stay in one column. The word is the
  // signal; the colour only reinforces it. The terminal dashboard and the
  // browser draw the same block.
  const BLOCK = { risk: "RISK", warn: "WARN", notice: "NOTE", info: "INFO" };
  function sevBlock(sev) {
    const word = BLOCK[sev] || "";
    const s = SEVERITY[sev] || SEVERITY.ok;
    const cls = word ? (sev === "notice" ? "note" : sev) : "none";
    return `<span class="blk ${cls}" title="${s.label}">${word}</span>`;
  }

  /* ───────── buttons ───────── */

  // Every control carries `data-do` (what it does) plus plain data arguments;
  // main.js has one click handler that reads them.
  function attrs(data) {
    return Object.entries(data || {})
      .filter(([, v]) => v != null && v !== false)
      .map(([k, v]) => ` data-${k}="${esc(v)}"`).join("");
  }
  function btn(label, data, o) {
    o = o || {};
    const cls = o.link ? "link" : `btn${o.primary ? " primary" : ""}`;
    return `<button type="button" class="${cls}"${attrs(data)}${o.disabled ? " disabled" : ""}${o.title ? ` title="${esc(o.title)}"` : ""}${o.pressed != null ? ` aria-pressed="${o.pressed ? "true" : "false"}"` : ""}>${o.icon ? ico(o.icon) : ""}<span>${esc(label)}</span></button>`;
  }
  const iconBtn = (icon, label, data) =>
    `<button type="button" class="ib"${attrs(data)} title="${esc(label)}" aria-label="${esc(label)}">${ico(icon)}</button>`;

  const sec = (id, html) => ({ id, html });
  // `action` is an optional control that sits at the right end of the header.
  const heading = (icon, text, n, action) =>
    `<div class="caprow"><h2 class="cap2">${ico(icon)}<span>${esc(text)}</span>${n == null ? "" : `<b>${esc(n)}</b>`}</h2>${action || ""}</div>`;
  const block = (inner, foot) => `<div class="block">${inner}${foot ? `<div class="blockfoot">${foot}</div>` : ""}</div>`;
  const blank = (text, action) => `<div class="blank"><p>${text}</p>${action || ""}</div>`;
  // A count of zero is a small dot, not a digit, so the only numbers on a row
  // are the ones that say something (the terminal dashboard does the same).
  const num = (v) => (v ? count(v) : "·");
  const numCell = (v, cls) => `<td class="n r${v ? (cls ? " " + cls : "") : " zero"}">${num(v)}</td>`;

  function top(title, lede, actions) {
    return `<header class="top"><h1>${esc(title)}</h1><p class="lede">${lede || ""}</p><div class="acts">${actions || ""}</div></header>`;
  }

  /* ───────── the model: lookups over what cohors sent ───────── */

  function model(state) {
    const o = (state && state.overview) || {};
    const l = (state && state.launchpad) || {};
    const repos = o.repos || [];
    return {
      o, l, repos,
      fig: o.figures || {},
      now: o.now || (state && state.generatedAt) || 0,
      byId: new Map(repos.map((r) => [r.id, r])),
      entries: new Map((l.repos || []).map((e) => [e.id, e])),
      board: (l.board && l.board.focus) || [],
      activity: l.recent_activity || [],
      agent: (state && state.agent) || { runners: [], prompts: [], error: null },
      threads: (state && state.local && state.local.threads) || {},
      sessions: (state && state.local && state.local.sessions) || {},
      home: (state && state.home) || "",
      here: (state && state.here) || [],
      trusted: !state || state.trusted !== false,
    };
  }

  const changed = (r) => (r.staged || 0) + (r.modified || 0) + (r.untracked || 0) + (r.conflicted || 0);
  const hasReason = (r, re) => (r.reasons || []).some((x) => re.test(x));

  // Saved views. Two of them read cohors's own plain-words reasons instead of
  // re-deriving its thresholds here, so they always agree with the judgment.
  const VIEWS = [
    { id: "risk",        label: "At risk",     test: (r) => !!r.at_risk },
    { id: "unpublished", label: "No upstream", test: (r) => hasReason(r, /^branch never pushed/) },
    { id: "stash",       label: "Has stash",   test: (r) => (r.stashes || 0) > 0 },
    { id: "dirty",       label: "Uncommitted", test: (r) => changed(r) > 0 },
    { id: "stale",       label: "Stale fetch", test: (r) => hasReason(r, /^not fetched in /) },
  ];

  function filterRepos(m, ui) {
    let list = m.repos;
    if (ui.pick && ui.pick.ids) list = list.filter((r) => ui.pick.ids.includes(r.id));
    else if (ui.view) {
      const v = VIEWS.find((x) => x.id === ui.view);
      if (v) list = list.filter(v.test);
    }
    const q = String(ui.q || "").trim().toLowerCase();
    if (q) {
      list = list.filter((r) =>
        [r.name, r.branch, r.path, r.upstream].concat(r.reasons || [], r.groups || []).join("\n").toLowerCase().includes(q));
    }
    return list;
  }

  // Can an agent be started at all, and if not, why not (for the tooltip).
  function agentState(m) {
    if (m.agent.error) return { ok: false, why: m.agent.error };
    if (!m.agent.runners.some((r) => r.available)) return { ok: false, why: "No agent runner is installed. See Agent runners on the Tasks page." };
    return { ok: true, why: "" };
  }
  function agentBtn(m, label, data, o) {
    const st = agentState(m);
    o = Object.assign({}, o);
    if (!st.ok) { o.disabled = true; o.title = st.why; }
    return btn(label, data, o);
  }
  // Anything a repository supplies honours Workspace Trust: its flows
  // (declared or detected), its agents and its tasks. The host enforces the
  // same rule and also shows each command before its first run.
  function trustedBtn(m, label, data, o, isAgent) {
    if (!m.trusted) return btn(label, data, Object.assign({}, o, { disabled: true, title: "Disabled because this workspace is not trusted." }));
    return isAgent ? agentBtn(m, label, data, o) : btn(label, data, o);
  }

  /* ───────── the shell (painted once) ───────── */

  function shell() {
    return `<nav class="rail" id="rail" aria-label="cohors"></nav>
<main class="main" id="main" tabindex="-1"></main>
<aside class="detail" id="detail" aria-label="Repository detail" hidden></aside>
<div class="toast" id="toast" role="status" aria-live="polite" hidden></div>`;
  }

  const PAGES = [
    { id: "overview", label: "Overview",     icon: "dashboard" },
    { id: "repos",    label: "Repositories", icon: "repo" },
    { id: "tasks",    label: "Tasks",        icon: "checklist" },
    { id: "standup",  label: "Standup",      icon: "calendar" },
    { id: "devices",  label: "Devices",      icon: "device-mobile" },
  ];

  function rail(state, ui) {
    const ok = state && state.status === "ok";
    const m = model(ok ? state : null);
    const counts = ok ? { repos: m.fig.repos, tasks: m.fig.open_tasks } : {};
    const out = [];

    out.push(sec("brand", `<div class="brand">${markSvg()}<span class="word">cohors</span><span class="grow"></span>${iconBtn("refresh", "Refresh (r)", { do: "refresh" })}</div>`));

    out.push(sec("nav", `<ul>${PAGES.map((p) =>
      `<li><button type="button" class="item" data-do="page" data-page="${p.id}"${ui.page === p.id ? ' aria-current="page"' : ""}>${ico(p.icon)}<span class="lb">${p.label}</span>${counts[p.id] ? `<span class="n">${count(counts[p.id])}</span>` : ""}</button></li>`).join("")}</ul>`));

    if (ok) {
      out.push(sec("views", `<h2 class="cap">Saved views</h2><ul>${VIEWS.map((v) => {
        const on = ui.page === "repos" && ui.view === v.id && !ui.pick;
        return `<li><button type="button" class="item" data-do="view" data-view="${v.id}" aria-pressed="${on ? "true" : "false"}">${ico("bookmark")}<span class="lb">${v.label}</span><span class="n">${count(m.repos.filter(v.test).length)}</span></button></li>`;
      }).join("")}</ul>`));
    }

    const roots = ok ? (m.o.roots || []) : ((state && state.rootsSetting) || []);
    out.push(sec("roots", `<h2 class="cap"><span>Roots</span>${iconBtn("gear", "Change roots in settings", { do: "settings" })}</h2>${
      roots.length
        ? `<ul>${roots.map((r) => `<li class="item static" title="${esc(r)}">${ico("folder")}<span class="lb">${esc(tilde(r, m.home || (state && state.home)))}</span></li>`).join("")}</ul>`
        : `<p class="hint">From your cohors config.</p>`}`));

    if (ok) {
      out.push(sec("foot", `<p class="hint" title="${esc((state.binary && state.binary.path) || "")}">Scanned ${esc(clock(m.now))}${state.binary && state.binary.version ? `<br>${esc(state.binary.version)}` : ""}</p>`));
    }
    return out;
  }

  /* ───────── problems: missing binary, failed scan ───────── */

  function problem(state) {
    const p = (state && state.problem) || {};
    if (state.status === "missing") {
      return [sec("problem", `<div class="problem">
        ${ico("plug", "big")}
        <h1>${p.configured ? "The cohors.binaryPath setting does not point at cohors" : "The cohors command-line tool is not installed"}</h1>
        <p>${esc(p.message || "cohors was not found.")}</p>
        <p>This dashboard shows what the cohors command-line tool reports about your repositories, so it needs that tool on this machine.</p>
        <ol>
          <li>Install it with one of these, in a terminal:
            <div class="cmd"><code>brew install rushirb2001/tap/cohors</code>${iconBtn("copy", "Copy", { do: "copy", text: "brew install rushirb2001/tap/cohors" })}</div>
            <div class="cmd"><code>pip install cohors</code>${iconBtn("copy", "Copy", { do: "copy", text: "pip install cohors" })}</div>
          </li>
          <li>If it is installed somewhere your shell does not look, set <code>cohors.binaryPath</code> to its full path.</li>
          <li>Choose Try again.</li>
        </ol>
        <div class="acts">${btn("Try again", { do: "redetect" }, { primary: true, icon: "refresh" })}${btn("How to install", { do: "install-help" }, { icon: "link-external" })}${btn("Open settings", { do: "settings" }, { icon: "gear" })}</div>
      </div>`)];
    }
    const detail = String(p.detail || "").split("\n").slice(0, 14).join("\n").trim();
    return [sec("problem", `<div class="problem">
      ${ico("warning", "big")}
      <h1>cohors could not read your repositories</h1>
      <p>${esc(p.message || "cohors exited with an error.")}</p>
      ${detail && detail !== p.message ? `<pre>${esc(detail)}</pre>` : ""}
      <p>${p.kind === "parse"
        ? "The extension and the cohors command-line tool may be different versions. Update both, then try again."
        : "Run <code>cohors overview</code> in a terminal to see the same error. If the roots are wrong, change them in settings."}</p>
      <div class="acts">${btn("Try again", { do: "redetect" }, { primary: true, icon: "refresh" })}${btn("Open settings", { do: "settings" }, { icon: "gear" })}</div>
    </div>`)];
  }

  const loading = (text) => [sec("loading", `<div class="problem quiet">${ico("loading", "big spin")}<p>${esc(text)}</p></div>`)];

  function noRepos(state, m) {
    const roots = m.o.roots || [];
    return `<div class="problem">
      ${ico("repo", "big")}
      <h1>No repositories found</h1>
      <p>${roots.length
        ? `cohors looked under ${roots.map((r) => `<code>${esc(tilde(r, m.home))}</code>`).join(", ")} and found no git repositories.`
        : "cohors has no roots to scan yet. Run <code>cohors init</code> in a terminal to write a config, or point it at folders here."}</p>
      <div class="acts">${m.here.length ? btn("Use this workspace's folders as roots", { do: "workspace-roots" }, { primary: true, icon: "folder" }) : btn("Open a folder", { do: "open-folder" }, { primary: true, icon: "folder-opened" })}${btn("Open settings", { do: "settings" }, { icon: "gear" })}</div>
    </div>`;
  }

  /* ───────── shared pieces ───────── */

  function upstreamCell(r) {
    if (r.error) return `<span class="bad">Unreadable</span>`;
    if (r.upstream) {
      // Level with its upstream: nothing to say, so nothing is said.
      if (!r.ahead && !r.behind) return "";
      const parts = [];
      if (r.ahead) parts.push(`${count(r.ahead)} ahead`);
      if (r.behind) parts.push(`${count(r.behind)} behind`);
      return `<span class="caution">${parts.join(", ")}</span>`;
    }
    if (!r.has_remote) return `<span class="dim">No remote</span>`;
    return hasReason(r, /^branch never pushed/) ? `<span class="bad">No upstream</span>` : `<span class="dim">No upstream</span>`;
  }

  // The compact table of repositories, shared by Overview and Repositories.
  function repoTable(m, list) {
    const rows = list.map((r) => `<tr data-repo="${esc(r.id)}">
      <td class="chk"><input type="checkbox" data-select="${esc(r.id)}" aria-label="Select ${esc(r.name)}"></td>
      <td class="clip">${sevBlock(r.severity)}<button type="button" class="link name" data-do="detail" data-id="${esc(r.id)}">${esc(r.name)}</button></td>
      <td class="dim clip c-branch">${ico("git-branch")}${esc(r.branch)}</td>
      <td class="clip">${upstreamCell(r)}</td>
      ${numCell(r.staged)}${numCell(r.modified)}${numCell(r.untracked)}
      <td class="n r c-extra${r.stashes ? " caution" : " zero"}">${num(r.stashes)}</td>
      <td class="n r c-extra${r.tasks_open ? "" : " zero"}">${num(r.tasks_open)}</td>
      <td class="n r">${esc(r.commit ? age(r.commit.timestamp, m.now) : "")}</td>
      <td class="dim clip c-last">${r.commit ? `<span class="mono">${esc(r.commit.short_id)}</span> ${esc(r.commit.summary)}` : esc(r.error || "No commits")}</td>
    </tr>`).join("");
    return `<table class="repos">
      <thead><tr><th class="chk"><span class="sr">Select</span></th><th class="w-name">Repository</th><th class="w-branch c-branch">Branch</th><th class="w-up">Upstream</th><th class="r w-n" title="Staged files">Stg</th><th class="r w-n" title="Modified files">Mod</th><th class="r w-n" title="Untracked files">New</th><th class="r w-n2 c-extra">Stash</th><th class="r w-n2 c-extra">Tasks</th><th class="r w-n2" title="Since the last commit">Age</th><th class="c-last">Last commit</th></tr></thead>
      <tbody>${rows}</tbody></table>`;
  }

  /* ───────── Overview ───────── */

  // What a "Do next" button does when cohors gave no verb to run.
  const nextButton = (a, i) => btn(a.label || "Open", { do: "next", i }, { primary: i === 0 });

  function figures(m) {
    const f = m.fig;
    const backed = [];
    if (f.unpublished) backed.push(`${count(f.unpublished)} never pushed`);
    if (f.unpushed_repos) backed.push(`${plural(f.unpushed_commits, "commit")} unpushed`);
    const cells = [
      ["cloud-upload", "Not backed up", f.not_backed_up, backed.join(", ") || "everything is on a remote", f.not_backed_up ? "risk" : ""],
      ["archive", "Stashes", f.stashes, f.stashes ? `in ${plural(f.stash_repos, "repo")}${f.stale_stash_repos ? `, ${count(f.stale_stash_repos)} stale` : ""}` : "none parked", f.stale_stash_repos ? "warn" : ""],
      ["diff", "Uncommitted files", f.uncommitted_files, f.dirty_repos ? `across ${plural(f.dirty_repos, "repo")}` : "every tree is clean", ""],
      ["sync", "Stale fetch", f.stale_fetch_repos, f.stale_fetch_repos ? `oldest ${plural(f.oldest_fetch_days, "day")}` : "all fetched recently", ""],
      ["checklist", "Open tasks", f.open_tasks, f.open_tasks ? `${count(f.high_tasks)} high priority` : `${count(f.configured_repos)} of ${count(f.repos)} repos set up`, ""],
      ["git-commit", "Commits, 7 days", f.commits_week, `${count(f.commits_total)} in ${plural((m.o.activity || []).length, "week")}`, ""],
    ];
    return `<div class="figs">${cells.map((c) =>
      `<div class="fig${c[4] ? " " + c[4] : ""}"><div class="k">${ico(c[0])}${c[1]}</div><div class="v">${count(c[2])}</div><div class="q">${esc(c[3])}</div></div>`).join("")}</div>`;
  }

  // The dashboard's centrepiece: the mark, large, with the fleet's headline
  // beside it and the last twelve weeks of commits on the right. It is the
  // same header the terminal dashboard opens with.
  function hero(state, m, actions) {
    const weeks = (m.o.activity || []).slice(-12);
    const tallest = Math.max(1, ...weeks);
    const roots = (m.o.roots || []).map((r) => tilde(r, state.home));
    return `<header class="hero">
      <div class="heromark">${markSvg()}</div>
      <div class="herotext">
        <h1><span class="word">cohors</span><span class="tag">your git fleet</span></h1>
        <p class="lede">${esc(m.o.headline || "")}</p>
        ${roots.length ? `<p class="roots clip" title="${esc(roots.join("\n"))}">${esc(roots.join("   "))}</p>` : ""}
      </div>
      <div class="heroside">
        ${weeks.length ? `<div class="spark" role="img" aria-label="Commits per week over the last ${weeks.length} weeks">${weeks.map((n) => `<i style="height:${Math.max(6, Math.round((n / tallest) * 100))}%" title="${count(n)}"></i>`).join("")}</div>
        <p class="sparkcap">commits, ${weeks.length} weeks</p>` : ""}
        <div class="acts">${actions || ""}</div>
      </div></header>`;
  }

  function overview(state, ui) {
    const m = model(state);
    const f = m.fig;
    const out = [];
    out.push(sec("hero", hero(state, m,
      btn("Fetch all", { do: "fetch-all" }, { icon: "sync", title: "git fetch in every repository. It never touches a working tree." })
      + btn("Act on selected", { do: "act" }, { primary: true, icon: "zap", title: "Fetch, pull, push, commit or stash in the repositories ticked below." }))));

    if (!m.repos.length) { out.push(sec("empty", noRepos(state, m))); return out; }

    const actions = m.o.actions || [];
    out.push(sec("next", `${heading("zap", "Do next")}${block(actions.length
      ? actions.map((a, i) => `<div class="todo">${sevBlock(a.severity)}<span class="t">${esc(a.title)}</span>${nextButton(a, i)}<span class="d">${esc(a.why)}</span></div>`).join("")
      : blank("Nothing needs doing. Every repository is pushed, fetched and clean."))}`));

    out.push(sec("figs", figures(m)));

    const risky = m.repos.filter((r) => r.at_risk);
    out.push(sec("risk", `${heading("warning", "Work at risk", risky.length)}${block(risky.length
      ? `<table class="risk"><thead class="hid"><tr><th class="w-risk">Repository</th><th>Why</th><th class="w-age">Last commit</th><th class="w-act">Next step</th></tr></thead><tbody>${risky.map((r) => `<tr data-repo="${esc(r.id)}">
          <td class="w-risk"><div class="clip">${sevBlock(r.severity)}<button type="button" class="link name" data-do="detail" data-id="${esc(r.id)}">${esc(r.name)}</button></div><div class="sub clip under-blk">${ico("git-branch")}${esc(r.branch)}</div></td>
          <td class="wrap">${esc(cap((r.reasons || []).join(", ")))}</td>
          <td class="n r w-age">${esc(r.commit ? age(r.commit.timestamp, m.now) : "")}</td>
          <td class="r w-act">${r.action ? btn(r.action.label, { do: "repo-action", id: r.id }, { link: true }) : ""}</td></tr>`).join("")}</tbody></table>`
      : blank("No local work is at risk."))}`));

    const work = m.o.work || [];
    const unset = (f.repos || 0) - (f.configured_repos || 0);
    const progress = [
      f.doing_tasks ? `${count(f.doing_tasks)} in progress.` : "Nothing is marked in progress.",
      f.blocked_tasks ? `${count(f.blocked_tasks)} blocked.` : "",
      unset > 0 ? `${plural(unset, "repository", "repositories")} ${unset === 1 ? "has" : "have"} no launchpad file.` : "",
    ].filter(Boolean).join(" ");
    const tasksBlock = `${heading("checklist", "Open tasks", f.open_tasks || 0)}${block(work.length
      ? `<table><thead><tr><th>Repository</th><th class="r w-n2">Open</th><th class="r w-n2">High</th><th class="r w-n2">Focus</th></tr></thead><tbody>${work.map((w) =>
          `<tr><td class="clip"><button type="button" class="link name" data-do="page" data-page="tasks">${esc(w.name)}</button></td>${numCell(w.open)}${numCell(w.high, "caution")}${numCell(w.focus)}</tr>`).join("")}</tbody></table>`
      : blank("No open tasks. A repository lists its own tasks once it has a launchpad file.", btn("Set up a repository", { do: "page", page: "tasks" }, { link: true })),
      esc(progress))}`;

    const commits = m.o.commits || [];
    const max = Math.max(1, ...commits.map((c) => c.count || 0));
    const quiet = (f.repos || 0) - (f.active_repos || 0);
    const commitsBlock = `${heading("git-commit", "Commits, last 7 days", f.commits_week || 0)}${block(commits.length
      ? commits.map((c) => `<div class="bar-row"><span class="clip">${esc(c.name)}</span><span class="track"><i style="width:${Math.max(2, Math.round((c.count / max) * 100))}%"></i></span><span class="num">${count(c.count)}</span></div>`).join("")
      : blank("No commits in the last 7 days."),
      esc(`${plural(quiet, "repository", "repositories")} had no commits. Previous week: ${count(f.commits_prev_week)}.`))}`;
    out.push(sec("pair", `<div class="pair"><div>${tasksBlock}</div><div>${commitsBlock}</div></div>`));

    out.push(sec("all", `${heading("repo", "All repositories", m.repos.length)}${block(repoTable(m, m.repos))}`));
    return out;
  }

  /* ───────── Repositories ───────── */

  function repositories(state, ui) {
    const m = model(state);
    const list = filterRepos(m, ui);
    const out = [];
    out.push(sec("top", top("Repositories",
      list.length === m.repos.length ? plural(m.repos.length, "repository", "repositories") : `${count(list.length)} of ${count(m.repos.length)} repositories`,
      btn("Fetch all", { do: "fetch-all" }, { icon: "sync", title: "git fetch in every repository. It never touches a working tree." }))));

    if (!m.repos.length) { out.push(sec("empty", noRepos(state, m))); return out; }

    // Static on purpose: the input is never re-rendered, so the caret survives.
    out.push(sec("filter", `<label class="search">${ico("search")}<input id="q" type="text" placeholder="Filter by name, branch, path or reason" autocomplete="off" spellcheck="false" aria-label="Filter repositories"><kbd>/</kbd></label>`));

    const chips = [btn("All", { do: "view", view: "" }, { pressed: !ui.view && !ui.pick })]
      .concat(VIEWS.map((v) => btn(`${v.label} ${count(m.repos.filter(v.test).length)}`, { do: "view", view: v.id }, { pressed: ui.view === v.id && !ui.pick })));
    out.push(sec("views", `<div class="seg" role="group" aria-label="Saved views">${chips.join("")}</div>${
      ui.pick ? `<div class="picked">${ico("filter")}<span>Showing: ${esc(ui.pick.label)}</span>${btn("Show all", { do: "clear-pick" }, { link: true })}</div>` : ""}`));

    out.push(sec("bulk", `<div class="bulk" role="group" aria-label="Act on selected repositories">
      <span class="selcount" id="selcount">None selected</span>
      ${btn("Select shown", { do: "select-shown" }, { link: true })}${btn("Clear", { do: "select-none" }, { link: true })}
      <span class="grow"></span>
      ${btn("Fetch", { do: "bulk", verb: "fetch" }, { title: "git fetch. Never touches a working tree." })}${btn("Pull", { do: "bulk", verb: "pull" }, { title: "git pull --ff-only. Asks first." })}${btn("Push", { do: "bulk", verb: "push" }, { title: "git push, never forced. Asks first." })}${btn("Commit", { do: "bulk", verb: "commit" }, { title: "Stage everything and commit with one message. Asks first." })}${btn("Stash", { do: "bulk", verb: "stash" }, { title: "git stash push. Asks first." })}
    </div>`));

    out.push(sec("table", block(list.length
      ? repoTable(m, list)
      : blank("No repository matches.", btn("Show all", { do: "clear-filter" }, { link: true })))));
    return out;
  }

  /* ───────── the detail panel ───────── */

  const TASK_STATUS = {
    todo:    { icon: "circle-large-outline", label: "To do" },
    doing:   { icon: "play-circle",          label: "In progress" },
    blocked: { icon: "circle-slash",         label: "Blocked" },
    done:    { icon: "pass-filled",          label: "Done" },
  };
  const taskStatus = (t) => (TASK_STATUS[t.status] ? t.status : "todo");
  const STATUS_ORDER = { doing: 0, todo: 1, blocked: 2, done: 3 };
  const PRIORITY_ORDER = { high: 0, normal: 1, low: 2 };
  function sortTasks(tasks) {
    return tasks.slice().sort((a, b) =>
      (STATUS_ORDER[taskStatus(a)] - STATUS_ORDER[taskStatus(b)])
      || ((PRIORITY_ORDER[a.priority] == null ? 1 : PRIORITY_ORDER[a.priority]) - (PRIORITY_ORDER[b.priority] == null ? 1 : PRIORITY_ORDER[b.priority])));
  }

  function taskRow(m, repo, t) {
    const st = taskStatus(t);
    const s = TASK_STATUS[st];
    const thread = (m.threads[repo.id] || {})[t.id];
    const meta = [
      `<span class="st">${s.label}</span>`,
      t.priority === "high" ? `<span class="pri">High priority</span>` : t.priority === "low" ? `<span>Low priority</span>` : "",
      t.agent ? `<span>${ico("sparkle")}${esc(t.agent)}</span>` : "",
    ].filter(Boolean).join("");
    const action = st === "done" || !repo.path ? ""
      : trustedBtn(m, thread ? "Resume" : "Start", { do: "task", id: repo.id, task: t.id },
          { title: thread ? "Reopen the conversation this task was started in." : t.agent ? `Start an agent session on this task with the ${t.agent} agent.` : "Start an agent session on this task." }, true);
    return `<div class="trow" data-st="${st}">${ico(s.icon, "tst")}<div class="tmain"><div class="t">${esc(t.title)}</div><div class="d">${meta}</div>${t.notes ? `<div class="notes">${esc(t.notes)}</div>` : ""}</div>${action}</div>`;
  }

  function flowRows(m, repo, flows) {
    return flows.map((f) => {
      const meta = [f.port ? `port ${Number(f.port)}` : "", f.cwd && f.cwd !== "." ? `in ${f.cwd}` : "", f.primary ? "main flow" : ""].filter(Boolean).join(", ");
      const data = { do: "flow", id: repo.id, flow: f.id };
      // Declared or detected, a flow is the repository's own command line.
      const run = trustedBtn(m, "Run", data, { title: "Shows the command, then runs it in a terminal in this repository." });
      return `<div class="frow"><div class="tmain"><div class="t">${esc(f.label || f.id)}${meta ? `<span class="dim"> · ${esc(meta)}</span>` : ""}</div><code>${esc(f.run)}</code></div>${repo.path ? run : ""}</div>`;
    }).join("");
  }

  function detail(state, ui) {
    const m = model(state);
    const r = ui.sel && m.byId.get(ui.sel);
    if (!r) return [];
    const e = m.entries.get(r.id) || { configured: false };
    const c = e.contribution || {};
    const out = [];
    const id = { id: r.id };

    out.push(sec("d-head", `<header class="dhead"><div class="grow"><h2>${mark(r.severity)}${esc(r.name)}</h2><div class="sub clip">${ico("git-branch")}${esc(r.branch)}</div>${c.summary ? `<p class="summary">${esc(c.summary)}</p>` : ""}</div>${iconBtn("close", "Close (Esc)", { do: "close-detail" })}</header>`));

    if (r.path) {
      out.push(sec("d-open", `<div class="dacts">${btn("Open folder", Object.assign({ do: "open" }, id), { icon: "folder-opened" })}${btn("New window", Object.assign({ do: "open", new: "1" }, id), { icon: "multiple-windows", title: "Open this repository in a new window." })}${btn("Terminal", Object.assign({ do: "terminal" }, id), { icon: "terminal" })}${agentBtn(m, "Start working", Object.assign({ do: "start" }, id), { icon: "rocket", title: "Open this repository in its own window and start an agent session there." })}</div>`));
    }

    const tree = changed(r)
      ? [r.staged ? `${count(r.staged)} staged` : "", r.modified ? `${count(r.modified)} modified` : "", r.untracked ? `${count(r.untracked)} untracked` : "", r.conflicted ? `${count(r.conflicted)} conflicted` : ""].filter(Boolean).join(", ")
      : "Clean";
    const upstream = r.upstream
      ? `<span class="mono">${esc(r.upstream)}</span>, ${!r.ahead && !r.behind ? "level" : [r.ahead ? `${count(r.ahead)} ahead` : "", r.behind ? `${count(r.behind)} behind` : ""].filter(Boolean).join(", ")}`
      : !r.has_remote ? "This repository has no remote."
      : hasReason(r, /^branch never pushed/) ? "None. This branch has never been pushed." : "None";
    out.push(sec("d-state", `<h3>State</h3><dl class="kv">
      <dt>Path</dt><dd class="mono wrap">${esc(tilde(r.path || "", m.home)) || "Unknown"}</dd>
      <dt>Upstream</dt><dd>${upstream}</dd>
      <dt>Last fetch</dt><dd>${r.fetched ? esc(ago(r.fetched, m.now)) : r.has_remote ? "No record of a fetch" : "Not applicable"}</dd>
      <dt>Working tree</dt><dd>${esc(tree)}</dd>
      <dt>Stashes</dt><dd>${r.stashes ? count(r.stashes) : "None"}</dd>
      <dt>Last commit</dt><dd>${r.commit ? `<span class="mono">${esc(r.commit.short_id)}</span>, ${esc(ago(r.commit.timestamp, m.now))}${r.commit.author ? ` by ${esc(r.commit.author)}` : ""}<br><span class="dim">${esc(r.commit.summary)}</span>` : "No commits"}</dd>
      ${(r.groups || []).length ? `<dt>Groups</dt><dd>${esc(r.groups.join(", "))}</dd>` : ""}
    </dl>`));

    const reasons = r.reasons || [];
    out.push(sec("d-why", `<h3>${reasons.length ? "Why it is listed" : "Nothing to report"}</h3>${
      r.error ? `<p class="bad">${esc(r.error)}</p>` : ""}${
      reasons.length ? `<ul class="why">${reasons.map((x) => `<li>${mark(r.severity)}<span>${esc(cap(x))}</span></li>`).join("")}</ul>` : `<p class="dim">This repository is pushed, fetched and clean.</p>`}
      <div class="dacts">${r.action && r.action.verb ? btn(r.action.label, Object.assign({ do: "repo-action" }, id), { primary: true }) : ""}${
        ["fetch", "pull", "push", "commit", "stash"].filter((v) => !(r.action && r.action.verb === v)).map((v) => btn(cap(v), Object.assign({ do: "bulk-one", verb: v }, id), { link: true })).join("")}</div>`));

    if (!e.configured) {
      out.push(sec("d-setup", `<h3>Launchpad</h3><p class="dim">${e.invalid
        ? `Its <code>.cohors/repo.json</code> is not valid: ${esc(e.invalid)}.`
        : "This repository has no launchpad file, so it declares no flows, agents or tasks yet."}</p>
        <div class="dacts">${r.path ? agentBtn(m, e.invalid ? "Fix with an agent" : "Set up with an agent", Object.assign({ do: "prompt", kind: "setup-repo" }, id), { primary: true, icon: "sparkle" }) : ""}</div>`));
    }

    const declared = c.flows || [];
    const detected = e.detected_flows || [];
    if (declared.length || detected.length) {
      out.push(sec("d-flows", `${declared.length ? `<h3>Flows</h3>${flowRows(m, r, declared)}` : ""}${detected.length ? `<h3>Detected scripts</h3>${flowRows(m, r, detected)}` : ""}`));
    }

    const agents = c.agents || [];
    if (agents.length) {
      out.push(sec("d-agents", `<h3>Agents</h3>${agents.map((a) => `<div class="frow"><div class="tmain"><div class="t">${esc(a.label || a.id)}</div><div class="d"><span class="mono">${esc(a.file)}</span>${(a.supports || []).length ? `<span>runs with ${esc(a.supports.join(", "))}</span>` : ""}</div></div>${btn("Open prompt", { do: "agent-file", id: r.id, agent: a.id }, { link: true })}${r.path ? trustedBtn(m, "Run", { do: "agent", id: r.id, agent: a.id }, {}, true) : ""}</div>`).join("")}`));
    }

    const tasks = sortTasks(c.tasks || []);
    if (tasks.length) {
      const open = tasks.filter((t) => taskStatus(t) !== "done");
      const done = tasks.length - open.length;
      out.push(sec("d-tasks", `<h3>Tasks</h3>${open.map((t) => taskRow(m, r, t)).join("") || `<p class="dim">No open tasks.</p>`}${done ? `<p class="dim">${plural(done, "finished task")} not shown.</p>` : ""}`));
    }

    const sessions = m.sessions[r.id] || [];
    if (sessions.length) {
      out.push(sec("d-sessions", `<h3>Claude Code sessions</h3>${sessions.map((s) => `<div class="frow"><div class="tmain"><div class="t">${esc(s.title || "(untitled)")}</div><div class="d"><span>${esc(ago(s.ts, state.generatedAt || m.now))}</span>${s.turns == null ? "" : `<span>${plural(s.turns, "turn")}</span>`}${s.branch ? `<span class="mono">${esc(s.branch)}</span>` : ""}</div></div>${agentBtn(m, "Resume", { do: "resume", id: r.id, session: s.id }, {})}</div>`).join("")}`));
    }

    if (e.configured && r.path) {
      const prompts = m.agent.prompts.filter((p) => p.needs_repo);
      out.push(sec("d-prompts", `<h3>Ask an agent</h3>${m.agent.error ? `<p class="dim">${esc(m.agent.error)}</p>` : prompts.map((p) =>
        `<div class="frow"><div class="tmain"><div class="t">${esc(p.title)}</div>${p.summary ? `<div class="notes">${esc(p.summary)}</div>` : ""}</div>${agentBtn(m, "Ask", { do: "prompt", kind: p.id, id: r.id }, {})}</div>`).join("")}`));
    }

    const links = (c.links || []).filter((l) => /^https?:\/\//i.test(l.url || ""));
    if (links.length) {
      out.push(sec("d-links", `<h3>Links</h3><div class="dacts">${links.map((l) => btn(l.label || l.url, { do: "link", id: r.id, url: l.url }, { link: true, icon: "link-external", title: l.url })).join("")}</div>`));
    }
    return out;
  }

  /* ───────── Tasks ───────── */

  function runnerPicker(m) {
    const rs = m.agent.runners;
    return `<label class="pick"><span>Run with</span><select id="runner" data-change="runner" aria-label="Agent runner"${rs.length ? "" : " disabled"}>
      <option value="">First installed</option>${rs.map((r) => `<option value="${esc(r.id)}"${r.available ? "" : " disabled"}>${esc(r.label)}${r.available ? "" : " (not installed)"}</option>`).join("")}
    </select></label>`;
  }

  // The getting-started steps for the folder that is open. Each step's "done"
  // is derived from what is on screen, never stored, so it cannot go stale.
  function workspaceSteps(m) {
    if (!m.here.length) return "";
    const here = m.here[0];
    const repo = m.repos.find((r) => r.path && (here === r.path || here.startsWith(r.path + "/") || here.startsWith(r.path + "\\")));
    const name = repo ? repo.name : baseName(here);
    const e = (repo && m.entries.get(repo.id)) || { configured: false };
    const c = e.contribution || {};
    const usable = m.agent.runners.filter((r) => r.available);
    const ask = (label, kind) => repo
      ? agentBtn(m, label, { do: "prompt", kind, id: repo.id }, { link: true })
      : agentBtn(m, label, { do: "setup-here", kind }, { link: true });
    const steps = [
      { done: usable.length > 0, title: "Install an agent runner",
        note: usable.length ? usable.map((r) => r.label).join(", ") : "None found on your PATH.",
        action: btn("Detect again", { do: "redetect" }, { link: true }) },
      { done: !!e.configured, title: `Let ${name} describe itself`,
        note: e.configured ? "It has a launchpad file." : "An agent reads the repository and commits .cohors/repo.json.",
        action: ask(e.configured ? "Run setup again" : "Set up with an agent", "setup-repo") },
      { done: (c.flows || []).length > 0, title: "Declare a flow worth launching",
        note: (c.flows || []).length ? `${plural(c.flows.length, "flow")} declared.` : "Only what detection cannot infer: a label, a port, the main flow.",
        action: ask("Declare flows", "declare-flows") },
      { done: (c.agents || []).length > 0, title: "Ship an agent with the repository",
        note: (c.agents || []).length ? `${plural(c.agents.length, "agent")} shipped.` : "A committed prompt becomes a one-click action for anyone who clones it.",
        action: ask("Add an agent", "add-agent") },
      { done: (c.tasks || []).length > 0, title: "Track work as tasks",
        note: (c.tasks || []).length ? `${plural(c.tasks.length, "task")} tracked.` : "Short-lived, concrete work an agent can pick up.",
        action: ask("Find tasks", "track-tasks") },
      { done: m.board.length > 0, title: "Plan focus across every repository",
        note: m.board.length ? `${plural(m.board.length, "item")} on the focus board.` : "One agent looks at everything and writes the focus board.",
        action: agentBtn(m, "Plan my day", { do: "prompt", kind: "plan-day" }, { link: true }) },
    ];
    const done = steps.filter((s) => s.done).length;
    return `${heading("folder-opened", `This workspace: ${name}`, `${done} of ${steps.length}`)}${block(
      (repo ? "" : `<div class="frow"><div class="tmain"><div class="notes">This folder is not under your cohors roots, so it is not listed with your other repositories.</div></div>${btn("Use this workspace's folders as roots", { do: "workspace-roots" }, { link: true })}</div>`)
      + steps.map((s) => `<div class="frow step${s.done ? " done" : ""}">${ico(s.done ? "pass-filled" : "circle-large-outline", "tst")}<div class="tmain"><div class="t">${esc(s.title)}<span class="sr"> ${s.done ? "(done)" : "(to do)"}</span></div><div class="notes">${esc(s.note)}</div></div>${s.action}</div>`).join(""))}`;
  }

  function tasks(state, ui) {
    const m = model(state);
    const f = m.fig;
    const out = [];
    const st = agentState(m);

    out.push(sec("top", top("Tasks",
      esc(`${plural(f.open_tasks || 0, "open task")} in ${plural(f.task_repos || 0, "repository", "repositories")}. ${count(f.configured_repos || 0)} of ${count(f.repos || 0)} set up.`),
      runnerPicker(m) + agentBtn(m, "Plan my day", { do: "prompt", kind: "plan-day" }, { primary: true, icon: "sparkle", title: "An agent looks at uncommitted work, unpushed branches and open tasks, then writes the focus board." }))));

    if (!st.ok) out.push(sec("agent-note", `<div class="notice">${ico("info")}<span>${esc(st.why)}</span>${btn("Detect again", { do: "redetect" }, { link: true })}</div>`));
    if (!m.trusted) out.push(sec("trust-note", `<div class="notice">${ico("shield")}<span>This workspace is not trusted, so every flow, and every agent or task declared by a repository, is disabled.</span></div>`));

    // Focus board.
    out.push(sec("focus", `${heading("target", "Focus board", m.board.length)}${block(m.board.length
      ? m.board.map((item) => {
          const repo = m.repos.find((r) => r.name === item.repo);
          const s = TASK_STATUS[taskStatus(item)];
          const resume = !!item.thread || !!(repo && (m.threads[repo.id] || {})[item.ref || item.id]);
          return `<div class="trow" data-st="${taskStatus(item)}">${ico(s.icon, "tst")}<div class="tmain"><div class="t">${esc(item.title)}</div><div class="d"><span class="st">${s.label}</span>${item.repo ? `<span>${ico("repo")}${esc(item.repo)}</span>` : ""}${item.repo && !repo ? `<span>not under your roots</span>` : ""}</div></div>${
            repo && repo.path && taskStatus(item) !== "done" ? trustedBtn(m, resume ? "Resume" : "Start", { do: "focus", item: item.id }, { title: resume ? "Reopen the conversation this was started in." : "Start an agent session on this." }, true) : ""}</div>`;
        }).join("")
      : blank("Nothing is on the focus board. An agent can look at uncommitted work, unpushed branches and open tasks, and decide what deserves today.",
          agentBtn(m, "Plan my day", { do: "prompt", kind: "plan-day" }, { link: true })))}`));

    // Tasks by repository.
    const withTasks = m.repos.map((r) => ({ r, e: m.entries.get(r.id) }))
      .filter((x) => x.e && x.e.contribution && (x.e.contribution.tasks || []).length);
    const doneTotal = withTasks.reduce((n, x) => n + x.e.contribution.tasks.filter((t) => taskStatus(t) === "done").length, 0);
    const toggle = doneTotal ? btn(ui.showDone ? "Hide finished" : `Show finished (${count(doneTotal)})`, { do: "toggle-done" }, { link: true }) : "";
    const groups = withTasks.map((x) => {
      const all = sortTasks(x.e.contribution.tasks);
      const open = all.filter((t) => taskStatus(t) !== "done");
      const shown = ui.showDone ? all : open;
      if (!shown.length) return "";
      return `<div class="group"><h3 class="grouphead"><button type="button" class="link name" data-do="detail" data-id="${esc(x.r.id)}">${esc(x.r.name)}</button><span class="dim">${plural(open.length, "open task")}</span></h3>${block(shown.map((t) => taskRow(m, x.r, t)).join(""))}</div>`;
    }).join("");
    out.push(sec("tasks", `${heading("checklist", "Tasks by repository", f.open_tasks || 0, toggle)}${
      groups || block(blank(doneTotal ? "Every task is finished." : "No repository tracks tasks yet. Tasks are short-lived, concrete work a repository lists in its launchpad file."))}`));

    // Declared flows.
    const withFlows = m.repos.map((r) => ({ r, e: m.entries.get(r.id) }))
      .filter((x) => x.e && x.e.contribution && (x.e.contribution.flows || []).length);
    const flowCount = withFlows.reduce((n, x) => n + x.e.contribution.flows.length, 0);
    const detectedCount = (m.l.repos || []).reduce((n, e) => n + (e.detected_flows || []).length, 0);
    out.push(sec("flows", `${heading("play", "Flows", flowCount)}${block(withFlows.length
      ? withFlows.map((x) => `<div class="flowgroup"><div class="who clip"><button type="button" class="link name" data-do="detail" data-id="${esc(x.r.id)}">${esc(x.r.name)}</button></div><div>${flowRows(m, x.r, x.e.contribution.flows)}</div></div>`).join("")
      : blank("No repository has declared a flow. A flow is a command a repository names on purpose: a dev server, a test run, a build."),
      detectedCount ? `${plural(detectedCount, "script")} detected from package.json, Cargo, Make and the like. They are listed in each repository's detail panel.` : "")}`));

    // Repositories not set up.
    const unset = (m.l.repos || []).filter((e) => !e.configured).map((e) => ({ e, r: m.byId.get(e.id) })).filter((x) => x.r);
    out.push(sec("unset", `${heading("repo", "Not set up", unset.length)}${block(unset.length
      ? `<table><thead class="hid"><tr><th class="w-name">Repository</th><th>Path</th><th class="w-setup">Set up</th></tr></thead><tbody>${unset.map((x) => `<tr data-repo="${esc(x.r.id)}">
          <td class="clip w-name">${sevBlock(x.r.severity)}<button type="button" class="link name" data-do="detail" data-id="${esc(x.r.id)}">${esc(x.r.name)}</button></td>
          <td class="dim clip">${x.e.invalid ? `<span class="bad">Launchpad file is not valid: ${esc(x.e.invalid)}</span>` : `<span class="mono">${esc(tilde(x.r.path || "", m.home))}</span>`}</td>
          <td class="r w-setup">${x.r.path ? agentBtn(m, x.e.invalid ? "Fix with an agent" : "Set up with an agent", { do: "prompt", kind: "setup-repo", id: x.r.id }, { link: true }) : ""}</td></tr>`).join("")}</tbody></table>`
      : blank("Every repository has a launchpad file."),
      unset.length ? "Setting up asks an agent to read the repository and commit .cohors/repo.json: its flows, the agents it ships and its current tasks." : "")}`));

    // Recent agent activity.
    const acts = m.activity.slice(0, 12);
    const stamp = (ts) => Math.floor(Date.parse(ts) / 1000) || 0;
    const runnerName = (id) => (m.agent.runners.find((r) => r.id === id) || {}).label || id || "Agent";
    out.push(sec("activity", `${heading("history", "Recent agent activity", acts.length || null)}${block(acts.length
      ? `<table><thead><tr><th class="w-n2">When</th><th class="w-up">Runner</th><th>Repository</th><th class="c-branch">Branch</th><th>What</th></tr></thead><tbody>${acts.map((a) => {
          const repo = m.repos.find((r) => r.path && r.path === a.repo);
          return `<tr><td class="n" title="${esc(a.ts)}">${esc(age(stamp(a.ts), state.generatedAt || m.now))}</td><td class="clip">${esc(runnerName(a.runner))}</td><td class="clip">${repo ? `<button type="button" class="link name" data-do="detail" data-id="${esc(repo.id)}">${esc(repo.name)}</button>` : esc(baseName(a.repo))}</td><td class="dim clip c-branch">${a.branch ? `${ico("git-branch")}${esc(a.branch)}` : ""}</td><td class="dim clip">${esc([a.phase === "launch" ? "started from cohors" : a.phase, a.agent].filter(Boolean).join(", "))}</td></tr>`;
        }).join("")}</tbody></table>`
      : blank("No agent sessions recorded yet. Sessions started from here are logged, and the cohors plugin for Claude Code logs the ones you start anywhere."))}`));

    // Runners: which are installed, and why one is not.
    out.push(sec("runners", `${heading("server-process", "Agent runners", m.agent.runners.filter((r) => r.available).length, btn("Detect again", { do: "redetect" }, { link: true }))}${block(m.agent.runners.length
      ? m.agent.runners.map((r) => `<div class="frow">${ico(r.available ? "pass-filled" : "circle-slash", "tst")}<div class="tmain"><div class="t">${esc(r.label)}<span class="dim"> · ${r.available ? "installed" : "not installed"}</span></div><div class="notes">${esc(r.available ? tilde(r.path || r.bin, m.home) : (r.reason || `${r.bin} was not found on your PATH.`))}${r.available && !r.can_resume ? " Cannot reopen a conversation by id, so Resume starts a new one." : ""}</div></div></div>`).join("")
      : blank(esc(m.agent.error || "cohors reported no agent runners.")))}`));

    const steps = workspaceSteps(m);
    if (steps) out.push(sec("workspace", steps));
    return out;
  }

  /* ───────── Standup ───────── */

  function standup(state, ui) {
    const m = model(state);
    const win = ui.standupWindow === "today" ? "today" : "week";
    const slot = (ui.standup && ui.standup[win]) || {};
    const d = slot.data;
    const out = [];
    const what = win === "today" ? "today" : "in the last 7 days";

    out.push(sec("top", top("Standup",
      d ? (d.author ? esc(`${plural(d.total || 0, "commit")} by ${d.author} ${what}.`) : "") : "",
      `<div class="seg" role="group" aria-label="Period">${btn("Today", { do: "standup-window", window: "today" }, { pressed: win === "today" })}${btn("Last 7 days", { do: "standup-window", window: "week" }, { pressed: win === "week" })}</div>`
      + btn("Copy as markdown", { do: "copy-standup" }, { primary: true, icon: "copy", disabled: !d || !d.total, title: "Copies the same digest `cohors standup` prints." }))));

    if (slot.error) {
      out.push(sec("body", `<div class="notice bad">${ico("warning")}<span>${esc(slot.error)}</span>${btn("Try again", { do: "standup-window", window: win }, { link: true })}</div>`));
    } else if (!d) {
      out.push(sec("body", `<div class="problem quiet">${ico("loading", "big spin")}<p>Reading your commits…</p></div>`));
    } else if (!d.author) {
      out.push(sec("body", block(blank("cohors lists the commits you wrote, matched by <code>git config user.email</code>, and that is not set. Set it with <code>git config --global user.email you@example.com</code>, then refresh."))));
    } else if (!(d.groups || []).length) {
      out.push(sec("body", block(blank(`No commits by ${esc(d.author)} ${what}.`, win === "today" ? btn("Show the last 7 days", { do: "standup-window", window: "week" }, { link: true }) : ""))));
    } else {
      out.push(sec("body", d.groups.map((g) => `<div class="group"><h3 class="grouphead"><span class="name">${esc(g.repo)}</span><span class="dim">${plural((g.commits || []).length, "commit")}</span></h3>${block(
        `<table><tbody>${(g.commits || []).map((c) => `<tr><td class="mono dim w-sha">${esc(c.short_id)}</td><td class="clip" title="${esc(c.summary)}">${esc(c.summary)}</td><td class="n r w-n2">${esc(age(c.timestamp, state.generatedAt || m.now))}</td></tr>`).join("")}</tbody></table>`)}</div>`).join("")));
    }
    return out;
  }

  /* ───────── Devices ───────── */

  function devices(state, ui) {
    const d = ui.devices;
    const out = [sec("top", top("Devices", "Simulators, emulators and local servers on this machine.",
      btn("Refresh", { do: "devices" }, { icon: "refresh" })))];
    if (!d) { out.push(sec("body", `<div class="problem quiet">${ico("loading", "big spin")}<p>Looking for devices…</p></div>`)); return out; }

    const a = d.android || {}, i = d.ios || {}, ports = d.ports || [];
    out.push(sec("android", `${heading("device-mobile", "Android emulator", a.booted ? 1 : 0)}${block(!a.available
      ? blank(esc(a.reason || "The Android SDK was not found."))
      : `<div class="frow"><div class="tmain"><div class="t">${a.booted ? esc(a.device) : "No emulator running"}<span class="dim"> · ${a.booted ? "running" : "stopped"}</span></div><div class="notes">${(a.avds || []).length ? esc(a.avds.join(", ")) : "No virtual devices are configured."}</div></div>${(a.avds || []).length ? btn(a.booted ? "Stop" : "Boot", { do: "android", action: a.booted ? "stop" : "boot" }, { title: a.booted ? "" : "Boots the first virtual device without a window." }) : ""}</div>`)}`));

    const booted = (i.booted || []).length;
    out.push(sec("ios", `${heading("device-mobile", "iOS simulators", i.available ? `${booted} running` : null)}${block(!i.available
      ? blank(esc(i.reason || "The Xcode command line tools were not found."))
      : !(i.all || []).length ? blank("No simulators are available. Create one in Xcode.")
      : i.all.slice(0, 12).map((s) => `<div class="frow"><div class="tmain"><div class="t">${esc(s.name)}<span class="dim"> · ${s.state === "Booted" ? "running" : "stopped"}</span></div></div>${btn(s.state === "Booted" ? "Shut down" : "Boot", { do: "ios", action: s.state === "Booted" ? "shutdown" : "boot", udid: s.udid })}</div>`).join(""),
      i.available && (i.all || []).length > 12 ? `${count(i.all.length - 12)} more not shown.` : "")}`));

    out.push(sec("ports", `${heading("radio-tower", "Listening ports", ports.length)}${block(ports.length
      ? `<table><thead><tr><th class="w-n2">Port</th><th>Process</th><th class="r w-setup">Open</th></tr></thead><tbody>${ports.map((p) => `<tr><td class="n">${Number(p.port)}</td><td class="clip">${esc(p.cmd || "")}</td><td class="r">${btn("In editor", { do: "port", port: p.port }, { link: true })}${btn("In browser", { do: "port", port: p.port, external: "1" }, { link: true })}</td></tr>`).join("")}</tbody></table>`
      : blank("Nothing is listening. A dev server you start shows up here with a link to open it."))}`));
    return out;
  }

  /* ───────── the sidebar: the overview, one pane per section ─────────
   *
   * The activity-bar container holds six views, and the editor draws them the
   * way it draws Explorer's: a header for each that opens, closes, reorders
   * and hides it. So the accordion is the editor's own, and this file only
   * says what goes inside each pane:
   *
   *   fleet    the mark, the headline, the six figures, the roots
   *   next     what to do next
   *   risk     work at risk
   *   tasks    open tasks, the focus board first
   *   commits  the week's commits by repository
   *   repos    every repository, with a filter
   *
   * The count for a pane is on its header (the host sets it from
   * `sideCounts`). Rows are ways in: choosing one opens the dashboard at that
   * place. Nothing here changes a repository.
   */
  const SIDE_PARTS = ["fleet", "next", "risk", "tasks", "commits", "repos"];

  // One row of a pane: a button that opens the dashboard somewhere.
  const srow = (go, first, second, right) =>
    `<button type="button" class="srow" data-do="show"${attrs(go)}><span class="l1">${first}${right ? `<span class="rt">${right}</span>` : ""}</span>${second ? `<span class="l2">${second}</span>` : ""}</button>`;
  const none = (text) => `<p class="snone">${esc(text)}</p>`;

  // The number each pane's header carries, or null before there is data.
  function sideCounts(state) {
    if (!state || state.status !== "ok") return null;
    const m = model(state);
    return {
      next: (m.o.actions || []).length,
      risk: m.repos.filter((r) => r.at_risk).length,
      tasks: m.fig.open_tasks || 0,
      commits: m.fig.commits_week || 0,
      repos: m.repos.length,
    };
  }

  function sidePart(part, state, ui) {
    ui = ui || {};
    // Only the first pane explains a problem; the others stay quiet under it.
    if (!state) return part === "fleet" ? loading("Reading your repositories…") : [sec("wait", none("Reading…"))];
    if (state.status !== "ok") return part === "fleet" ? problem(state) : [sec("wait", none("Nothing to show yet. The Overview pane says why."))];
    const m = model(state);
    const f = m.fig;
    const body = (html) => [sec(part, `<div class="part h-${part}">${html}</div>`)];

    if (part === "fleet") {
      const roots = m.o.roots || [];
      return [
        sec("head", `<header class="shead">
          <div class="brand">${markSvg()}<span class="word">cohors</span></div>
          <p class="lede">${esc(m.o.headline || "")}</p></header>`),
        sec("figs", m.repos.length ? figures(m) : noRepos(state, m)),
        sec("foot", `<footer class="sfoot">
          ${roots.length ? `<ul>${roots.map((r) => `<li title="${esc(r)}">${ico("folder")}<span class="clip">${esc(tilde(r, state.home))}</span></li>`).join("")}</ul>` : ""}
          <p title="${esc((state.binary && state.binary.path) || "")}">Scanned ${esc(clock(m.now))}${state.binary && state.binary.version ? ` · ${esc(state.binary.version)}` : ""}${iconBtn("gear", "Change roots in settings", { do: "settings" })}</p></footer>`),
      ];
    }

    if (part === "next") {
      const actions = m.o.actions || [];
      return body(actions.length
        ? actions.map((a) => srow({ page: "overview" }, `${sevBlock(a.severity)}<span class="t say">${esc(a.title)}</span>`, esc(a.why))).join("")
        : none("Nothing needs doing."));
    }

    if (part === "risk") {
      const risky = m.repos.filter((r) => r.at_risk);
      return body(risky.length
        ? risky.map((r) => srow({ page: "repos", repo: r.id },
            `${sevBlock(r.severity)}<span class="t">${esc(r.name)}</span>`,
            `${ico("git-branch")}${esc(r.branch)} · ${esc((r.reasons || []).join(", "))}`,
            esc(r.commit ? age(r.commit.timestamp, m.now) : ""))).join("")
        : none("No local work is at risk."));
    }

    if (part === "tasks") {
      const work = m.o.work || [];
      const focus = m.board.slice(0, 5);
      const unset = (f.repos || 0) - (f.configured_repos || 0);
      return body(
        (focus.length ? `<h3 class="sub3">In focus</h3>${focus.map((t) => srow({ page: "tasks" }, `<span class="t say">${esc(t.title)}</span>`, `${esc(t.repo || "")}${t.status ? ` · ${esc(t.status)}` : ""}`)).join("")}` : "")
        + (work.length
          ? `<h3 class="sub3">By repository</h3>${work.map((w) => srow({ page: "tasks" }, `<span class="t">${esc(w.name)}</span>`, "",
              `${count(w.open)} open${w.high ? `<span class="caution"> · ${count(w.high)} high</span>` : ""}`)).join("")}`
          : none("No open tasks."))
        + (unset > 0 ? `<p class="snote">${plural(unset, "repository", "repositories")} ${unset === 1 ? "has" : "have"} no launchpad file.</p>` : ""));
    }

    if (part === "commits") {
      const commits = m.o.commits || [];
      const max = Math.max(1, ...commits.map((c) => c.count || 0));
      return body(commits.length
        ? `${commits.map((c) => `<div class="bar-row"><span class="clip">${esc(c.name)}</span><span class="track"><i style="width:${Math.max(2, Math.round((c.count / max) * 100))}%"></i></span><span class="num">${count(c.count)}</span></div>`).join("")}
           <p class="snote">Previous week: ${count(f.commits_prev_week)}.</p>${btn("Open the standup", { do: "show", page: "standup" }, { link: true })}`
        : none("No commits in the last 7 days."));
    }

    // repos
    const q = String(ui.q || "").trim().toLowerCase();
    const shown = q ? m.repos.filter((r) => String(r.name).toLowerCase().includes(q)) : m.repos;
    const changes = (r) => (r.staged || 0) + (r.modified || 0) + (r.untracked || 0);
    return body(
      `<label class="search"><span class="sr">Filter repositories</span>${ico("search")}<input id="side-q" type="search" placeholder="Filter by name" value="${esc(ui.q || "")}" autocomplete="off" spellcheck="false"></label>`
      + (shown.length
        ? shown.map((r) => srow({ page: "repos", repo: r.id },
            `${sevBlock(r.severity)}<span class="t${r.at_risk ? "" : " plain"}">${esc(r.name)}</span>`,
            "",
            [r.ahead ? `${count(r.ahead)}↑` : "", r.behind ? `${count(r.behind)}↓` : "", changes(r) ? `${count(changes(r))} changed` : "", r.stashes ? `${count(r.stashes)} stashed` : ""].filter(Boolean).join(" · ")
              || `<span class="dim">${esc(r.commit ? age(r.commit.timestamp, m.now) : "")}</span>`)).join("")
        : none("No repository matches.")));
  }

  /* ───────── entry point ───────── */

  const RENDER = { overview, repos: repositories, tasks, standup, devices };

  // The sections of the main column for the current state and page.
  function page(state, ui) {
    if (!state) return loading("Reading your repositories…");
    if (state.status !== "ok") return problem(state);
    return (RENDER[ui.page] || overview)(state, ui);
  }

  return {
    esc, ico, age, ago, tilde, plural, count, cap, mark, sevBlock, markSvg, markRects, btn,
    model, filterRepos, agentState, VIEWS, PAGES, SEVERITY,
    shell, rail, page, detail, problem, sidePart, sideCounts, SIDE_PARTS,
    overview, repositories, tasks, standup, devices,
  };
});
