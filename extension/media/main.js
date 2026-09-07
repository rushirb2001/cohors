// Launch pad UI (package: principia).
//
// Rules this file enforces:
//  1. The shell is built once. Only sections whose HTML actually changed get
//     replaced. A background refresh must not destroy scroll, hover, keyboard
//     focus, or the caret in the filter box.
//  2. No horizontal scroll. Rows are grids with minmax(0,1fr); long text wraps.
//  3. Every tab has a real empty state. This ships empty, so the empty state IS
//     the product for the first ten minutes.

(function () {
  const vscode = acquireVsCodeApi();
  const app = document.getElementById("app");
  const saved = vscode.getState() || {};

  let D = saved.D || null;
  let tab = saved.tab || "today";
  let q = saved.q || "";
  let focus = -1;
  let built = false;
  const painted = new Map();

  // Flat order for keyboard 1-9; `under` nests an item in the sidebar.
  // `hint` explains what the badge number counts, shown on hover.
  const TABS = [
    ["today", "Today", "home", null, "what needs attention right now"],
    ["tasks", "Tasks", "checklist", null, "open tasks across all repos"],
    ["recents", "Recents", "history", null, "your editor's recently-opened list"],
    ["repos", "Repos", "folder", null, "repositories discovered from Recents"],
    ["workflows", "Workflows", "rocket", null, "flows a repo declared on purpose"],
    ["agents", "Agents", "sparkle", "workflows", "prompts a repo ships, plus runner status"],
    ["scripts", "Scripts", "play", "workflows", "commands auto-detected with zero config"],
    ["devices", "Devices", "device-mobile", "workflows", "booted simulators/emulators right now"],
    ["setup", "Setup", "gear", null, "repos with no .principia/ yet — needs an agent"],
  ];

  const send = (m) => vscode.postMessage(m);
  const persist = () => vscode.setState({ D, tab, q });
  const esc = (s) => String(s == null ? "" : s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  const ico = (n, cls = "") => {
    const safe = String(n || "").replace(/[^a-z0-9-]/g, "");
    return `<span class="ci codicon codicon-${safe} ${cls}"></span>`;
  };
  const hit = (s) => !q || String(s || "").toLowerCase().includes(q);
  const base = (p) => String(p || "").split("/").filter(Boolean).pop() || "";

  /* ───────── shell ───────── */

  function buildShell() {
    app.innerHTML = `
      <header class="top">
        <span class="mark">${ico("compass")}</span>
        <h1>Launch pad</h1>
        <span class="sub" id="where"></span>
        <span class="grow"></span>
        <label class="find">${ico("search", "sm")}<input id="q" placeholder="Filter" autocomplete="off" spellcheck="false"><kbd>/</kbd></label>
        <span class="stat" id="mem"></span>
        <span class="stat" id="load"></span>
        <button class="ib" data-act="refresh" title="Refresh (r)">${ico("refresh")}</button>
      </header>

      <aside class="side">
        <div class="ov" id="ov"></div>
        <nav id="tabs">
          ${TABS.map(([id, label, icon, under, hint], i) =>
            `<button class="nav${under ? " sub" : ""}" data-tab="${id}" title="${esc(hint || label)}">${ico(icon, "sm")}<span class="lb">${label}</span><span class="grow"></span><b class="n${id === "setup" ? " warn" : ""}" data-n="${id}"></b><kbd>${i + 1}</kbd></button>`
          ).join("")}
        </nav>
      </aside>

      <main id="body"></main>

      <footer class="bot">
        <span id="runners"></span>
        <span class="grow"></span>
        <span class="keys"><kbd>↑↓</kbd> move <kbd>⏎</kbd> open <kbd>/</kbd> filter <kbd>1-9</kbd> nav</span>
      </footer>
      <div class="note" id="note" hidden></div>`;
    document.getElementById("q").value = q;
    wire();
    built = true;
  }

  function bootedDevices() {
    const a = D.android || {}, i = D.ios || {};
    return (a.booted ? 1 : 0) + ((i.booted || []).length);
  }
  function unconfiguredRepos() { return (D.repos || []).filter((r) => !r.configured).length; }
  function firstUnconfigured() { return (D.repos || []).find((r) => !r.configured); }

  function counts() {
    const c = (D && D.counts) || {};
    return {
      today: openTasks().filter((t) => t.today || t.status === "doing").length,
      tasks: c.tasks || 0,
      // Recents restates the repo list; a count adds nothing, so no badge.
      recents: 0,
      repos: c.repos || 0,
      workflows: (D.repos || []).reduce((n, r) => n + r.flows.filter((f) => f.source === "declared").length, 0),
      agents: c.agents || 0,
      scripts: (D.repos || []).reduce((n, r) => n + r.flows.filter((f) => f.source === "detected").length, 0),
      // A real device booted, not the count of background dev-server ports.
      devices: bootedDevices(),
      setup: unconfiguredRepos(),
    };
  }

  function paintChrome() {
    if (!D) return;
    const set = (id, html) => {
      const e = document.getElementById(id);
      if (e && e.innerHTML !== html) e.innerHTML = html;
    };
    const m = D.machine || {};
    set("where", D.here ? esc(base(D.here)) : "no folder open");
    set("mem", m.free == null ? "" : `free <b class="${m.free < 2 ? "hot" : ""}">${m.free} GB</b>`);
    set("load", m.load == null ? "" : `load <b>${esc(m.load)}</b>`);

    const c = counts();
    for (const [id] of TABS) {
      const b = document.querySelector(`[data-n="${id}"]`);
      if (!b) continue;
      const v = c[id] ? String(c[id]) : "";
      if (b.textContent !== v) b.textContent = v;
      b.hidden = !v;
    }
    document.querySelectorAll(".nav").forEach((t) => t.classList.toggle("on", t.dataset.tab === tab));
    {
      const dirty = (D.repos || []).filter((r) => r.dirty > 0).length;
      const configured = c.repos - unconfiguredRepos();
      set("ov", `<div class="ovg">
        <div class="ovi"><b>${c.repos}</b><span>repos</span></div>
        <div class="ovi ${dirty ? "warn" : ""}" title="repos with uncommitted changes"><b>${dirty}</b><span>dirty</span></div>
        <div class="ovi ${configured ? "ok" : ""}" title="repos with a .principia/ contribution"><b>${configured}/${c.repos}</b><span>set up</span></div>
      </div>`);
    }
    document.getElementById("q").placeholder = `Filter ${tab}`;

    set("runners", (D.runners || []).map((r) =>
      `<span class="rn ${r.available ? "ok" : "no"}" title="${esc(r.reason || r.pathTo || "")}">${ico(r.available ? "pass" : "circle-slash", "sm")}${esc(r.label)}</span>`
    ).join(""));

    const n = document.getElementById("note");
    if (n) {
      const msg = D.error ? esc(D.error)
        : !D.contract ? `The contract is not on disk, so agent setup is unavailable. Point <code>principia.specPath</code> at a checkout of the Principia repo.`
        : "";
      n.hidden = !msg;
      if (msg && n.innerHTML !== msg) n.innerHTML = msg;
    }
  }

  /* ───────── task model: repo tasks + the cross-repo board ───────── */

  function allTasks() {
    const out = [];
    for (const r of D.repos || []) {
      for (const t of r.tasks || []) out.push({ ...t, repo: r.name, root: r.root, group: r.group || "",
        thread: (r.threads || {})[t.id] || null });
    }
    for (const f of D.board || []) {
      const r = (D.repos || []).find((x) => x.name === f.repo);
      out.push({ id: f.id, title: f.title, status: f.status || "todo", priority: "normal",
        repo: f.repo || "", root: r ? r.root : null, group: r ? r.group : "", today: true,
        agent: f.agent, thread: f.thread || null, ref: f.ref || null });
    }
    return out;
  }
  const openTasks = () => allTasks().filter((t) => t.status !== "done");
  const PRI = { high: 3, normal: 2, low: 1 };
  const byPri = (a, b) => (PRI[b.priority] || 2) - (PRI[a.priority] || 2);

  /* ───────── building blocks ───────── */

  let rows = [];
  const F = (attrs, cls = "") => { rows.push(attrs); return `class="row focusable${cls}" ${attrs}`; };

  function card(id, title, icon, inner, aside = "") {
    return `<section class="card" data-sec="${esc(id)}">
      <header>${ico(icon, "sm")}<h3>${esc(title)}</h3><span class="grow"></span>${aside}</header>
      <div class="cbody">${inner}</div>
    </section>`;
  }

  // Every tab renders into an explicit main column, and only tabs with a
  // genuinely short/bounded companion card (status, tips) get a rail. Nothing
  // is ever paired by DOM-order luck (see main.css for why that broke).
  const layout = (main, rail) =>
    `<div class="content">
      <div class="main">${main}</div>
      ${rail ? `<div class="rail">${rail}</div>` : ""}
    </div>`;

  const empty = (icon, title, lines, action = "") =>
    `<div class="empty">${ico(icon, "big")}<b>${esc(title)}</b>${lines.map((l) => `<p>${l}</p>`).join("")}${action}</div>`;

  // One row for a `~/.principia/history/*.jsonl` entry. Shared by Today's
  // "pick up where you left off" and the Agents tab's activity feed — same
  // data, same shape, two different framings.
  // One runner card, used by both the Agents and Setup rails. It used to be
  // pasted in three places, which is how three copies drift.
  function runnerCard(runners, title) {
    const list = runners || [];
    return card("runners", title || "Runners", "server-process",
      `<div class="rgrid one">${list.map((r) => `<div class="rc ${r.available ? "ok" : "no"}">
        <div class="l1">${ico(r.available ? "pass-filled" : "circle-slash")}<b>${esc(r.label)}</b>
          <span class="grow"></span>
          <span class="mono dim rcp" title="${esc(r.available ? (r.pathTo || r.bin) : (r.reason || "not installed"))}">${esc(r.available ? (r.pathTo || r.bin) : (r.reason || "not installed"))}</span>
        </div>
      </div>`).join("")}</div>`,
      `<button class="ib" data-act="redetect" title="Re-detect">${ico("refresh")}</button>`);
  }

  function relTime(iso) {
    if (!iso) return "";
    const ms = Date.now() - new Date(iso).getTime();
    const m = Math.floor(ms / 60000);
    if (m < 1) return "just now";
    if (m < 60) return `${m}m ago`;
    const h = Math.floor(m / 60);
    if (h < 24) return `${h}h ago`;
    return `${Math.floor(h / 24)}d ago`;
  }

  // One row for a real Claude Code session, read from ~/.claude/projects/ for
  // a repo that has opted in (see discovery.js: gated at the read, not here).
  function sessionRow(r, s, latest) {
    const cmd = `claude --resume ${s.id}`;
    return `<div ${F(`data-root="${esc(r.root)}"`, ` compact${latest ? " latest" : ""}`)}>
      <span class="ic">${ico("sparkle", "sm")}</span>
      <div class="main"><div class="l1"><b>${esc(s.title || "(untitled)")}</b></div>
        <div class="l2"><span class="dim">${esc(relTime(s.ts))}</span><span class="mono dim">${s.turns} turns</span>${s.branch ? `<span class="mono dim">${esc(s.branch)}</span>` : ""}<span class="mono dim">${esc(s.id.slice(0, 8))}</span></div></div>
      <div class="rslot n4"><span class="sa">
        ${s.path ? `<button class="ib" data-reveal="${esc(s.path)}" title="Reveal transcript in Finder">${ico("go-to-file")}</button>` : ""}
        <button class="ib" data-copy="${esc(cmd)}" title="Copy resume command">${ico("copy")}</button>
        <button class="btn" data-resume="${esc(s.id)}" data-root="${esc(r.root)}">${ico("debug-restart", "sm")} Resume</button>
      </span></div>
    </div>`;
  }

  function historyRow(e) {
    return `<div ${F(`data-root="${esc(e.repo || "")}"`)}>
      <span class="ic">${ico(e.phase === "launch" ? "sparkle" : "comment-discussion")}</span>
      <div class="main"><div class="l1"><b>${esc(e.runner || "agent")}</b>${e.agent ? `<span class="chip">${esc(e.agent)}</span>` : ""}${e.phase && e.phase !== "launch" ? `<span class="chip">${esc(e.phase)}</span>` : ""}</div>
      <div class="l2"><span class="mono dim">${esc(base(e.repo))}</span><span class="dim">${esc(String(e.ts || "").replace("T", " ").replace("Z", ""))}</span></div></div>
      <div class="rslot n1"><span class="sa">${e.repo ? `<button class="ib" data-open="${esc(e.repo)}" title="Open">${ico("folder-opened")}</button>` : ""}</span></div>
    </div>`;
  }

  // Say what the numbers mean. "●17 ↑8" assumes the reader already knows the
  // convention; the words cost a few pixels and remove the guessing.
  const gitBits = (r) => [
    r.branch ? `<span class="gb" title="Current branch">${ico("git-branch")}<span class="mono">${esc(r.branch)}</span></span>` : "",
    r.dirty ? `<span class="gb d" title="${r.dirty} file(s) changed but not committed">${r.dirty} uncommitted</span>` : "",
    r.ahead ? `<span class="gb a" title="${r.ahead} commit(s) not pushed to the upstream branch">${r.ahead} unpushed</span>` : "",
    r.behind ? `<span class="gb b" title="${r.behind} commit(s) on the upstream branch you do not have locally">${r.behind} behind</span>` : "",
  ].filter(Boolean).join("");

  function repoActions(r) {
    return `${r.configured ? "" : `<button class="ib" data-setup="${esc(r.root)}" title="Set up with an agent">${ico("sparkle")}</button>`}
      <button class="ib" data-term="${esc(r.root)}" title="Terminal here">${ico("terminal")}</button>
      <button class="ib" data-open="${esc(r.root)}" title="Open">${ico("folder-opened")}</button>
      <button class="ib" data-open="${esc(r.root)}" data-new="1" title="Open in new window">${ico("empty-window")}</button>`;
  }

  function taskRow(t) {
    const box = t.status === "done" ? "pass-filled" : t.status === "blocked" ? "circle-slash" : "circle-large-outline";
    return `<div ${F(`data-root="${esc(t.root || "")}"`)} data-st="${esc(t.status)}">
      <span class="ic">${ico(box)}</span>
      <div class="main">
        <div class="l1"><b>${esc(t.title)}</b></div>
        <div class="l2">
          ${t.group ? `<span class="chip">${esc(t.group)}</span>` : ""}
          ${t.repo ? `<span class="chip">${ico("repo", "sm")}${esc(t.repo)}</span>` : ""}
          ${t.priority === "high" ? `<span class="chip hi">high</span>` : ""}
          ${t.status === "doing" ? `<span class="chip go">in progress</span>` : ""}
          ${t.status === "blocked" ? `<span class="chip bad">blocked</span>` : ""}
          ${t.notes ? `<span class="nt">${esc(t.notes)}</span>` : ""}
        </div>
      </div>
      <div class="rslot ${t.thread ? "n5" : "n4"}"><span class="sa">
        ${t.root && t.status !== "done" ? (t.thread
          ? `<button class="btn" data-task="${esc(t.id)}" data-root="${esc(t.root)}" title="Reopen session ${esc(String(t.thread).slice(0, 8))}, where this task was started">${ico("debug-restart", "sm")} Resume</button>`
          : `<button class="ib" data-task="${esc(t.id)}" data-root="${esc(t.root)}" title="${t.agent ? `Run ${esc(t.agent)} agent` : "Work on this now"}">${ico("sparkle")}</button>`) : ""}
        ${t.root ? `<button class="ib" data-term="${esc(t.root)}" title="Terminal here">${ico("terminal")}</button>` : ""}
        ${t.root ? `<button class="ib" data-open="${esc(t.root)}" title="Open">${ico("folder-opened")}</button>` : ""}
        ${t.root ? `<button class="ib" data-open="${esc(t.root)}" data-new="1" title="Open in new window">${ico("empty-window")}</button>` : ""}
      </span></div>
    </div>`;
  }

  function repoRow(r) {
    return `<div ${F(`data-root="${esc(r.root)}"`)}>
      <span class="ic">${ico(r.icon || "repo")}</span>
      <div class="main">
        <div class="l1"><b>${esc(r.name)}</b>
          ${r.lastSubject ? `<span class="subj" title="${esc(r.lastSubject)}">${esc(r.lastSubject)}</span>` : ""}
          ${r.configured ? `<span class="chip ok">configured</span>` : `<span class="chip">not set up</span>`}
          ${r.invalid ? `<span class="chip bad">${esc(r.invalid)}</span>` : ""}
          ${r.staleBy ? `<span class="chip hi">${r.staleBy} commits since setup</span>` : ""}
        </div>
        ${r.summary ? `<div class="l2"><span class="nt">${esc(r.summary)}</span></div>` : ""}
      </div>
      <div class="rslot n4">
        <span class="sm">${gitBits(r)}${r.lastCommit ? `<span class="dim">${esc(r.lastCommit)}</span>` : ""}</span>
        <span class="sa">${repoActions(r)}</span>
      </div>
    </div>`;
  }

  const flowIcon = (f) => f.kind === "server" ? "play" : f.kind === "test" ? "beaker" : f.kind === "build" ? "package" : "tools";

  // Flows get the card's full width as their own strip, instead of competing
  // for the row's middle column with the git meta and actions. Squeezed into
  // that column they stacked one-per-line and wrapped mid-label, while most of
  // the row sat empty.
  function flowStrip(r, flows) {
    if (!flows.length) return "";
    return `<div class="chips pad">${flows.map((f) => {
      const meta = [f.run, f.port ? `port ${f.port}` : "", f.cwd && f.cwd !== "." ? `in ${f.cwd}` : ""].filter(Boolean).join(" · ");
      return `<button class="chip act${f.primary ? " pri" : ""}" data-flow="${esc(f.id)}" data-root="${esc(r.root)}" title="${esc(meta)}">${ico(flowIcon(f), "sm")}${esc(f.label)}${f.port ? `<span class="port">:${f.port}</span>` : ""}</button>`;
    }).join("")}</div>`;
  }

  /* ───────── tabs ───────── */
  const tToday = () => layout(tTodayInner());
  const tTasks = () => layout(tTasksInner());
  const tWorkflows = () => layout(tWorkflowsInner());
  const tRepos = () => layout(tReposInner());
  const tScripts = () => layout(tScriptsInner());
  const tRecents = () => layout(tRecentsInner());


  function tTodayInner() {
    rows = [];
    if (!(D.repos || []).length) {
      return card("boot", "Nothing discovered yet", "compass",
        empty("telescope", "Open a repository and it will appear here",
          ["Launch pad reads the projects you have recently opened in this editor, so there is nothing to configure.",
           "Your filesystem is never scanned and nothing about you is stored in the extension."]));
    }

    const today = openTasks().filter((t) => t.today || t.status === "doing").sort(byPri);
    const blocked = openTasks().filter((t) => t.status === "blocked");
    const dirty = (D.repos || []).filter((r) => r.dirty > 0).sort((a, b) => b.dirty - a.dirty);
    const done = allTasks().filter((t) => t.status === "done").length;
    const total = allTasks().length || 1;
    const pct = Math.round((done / total) * 100);
    const date = new Date().toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" });
    const h = new Date().getHours();
    const greet = h < 5 ? "Still up" : h < 12 ? "Good morning" : h < 17 ? "Good afternoon" : h < 22 ? "Good evening" : "Late night";

    let out = `<section class="hero" data-sec="hero">
      <div class="hl"><h2>${greet}</h2><div class="hd">${esc(date)}</div></div>
      <div class="hr">
        <div class="hs"><b>${today.length}</b><span>for today</span></div>
        <div class="hs"><b>${openTasks().length}</b><span>open</span></div>
        <div class="hs ${blocked.length ? "bad" : ""}"><b>${blocked.length}</b><span>blocked</span></div>
        <div class="hs ${dirty.length ? "warn" : ""}"><b>${dirty.length}</b><span>dirty repos</span></div>
      </div>
      <div class="bar"><i style="width:${pct}%"></i></div>
    </section>`;

    const anyRunner = (D.runners || []).some((x) => x.available);
    out += card("focus", "Focus today", "target",
      today.length ? today.map(taskRow).join("")
        : empty("target", "Nothing set for today",
            [`Focus comes from a repo's own <code>.principia/repo.json</code> tasks, plus the cross-repo board at <code>~/.principia/board.json</code>.`,
             `An agent looks at uncommitted work, unpushed branches and repo tasks, then decides what is worth your attention today.`],
            `<button class="btn p" data-plan="1" ${anyRunner ? "" : "disabled"} title="${anyRunner ? "" : "No agent runner is installed"}">${ico("sparkle", "sm")} Ask agent to plan my day</button>`),
      `<button class="ib" data-act="openSpec" title="Open the contract">${ico("book")}</button>`);

    if (blocked.length) out += card("blocked", "Blocked", "circle-slash", blocked.sort(byPri).map(taskRow).join(""));

    out += card("dirty", "Uncommitted work", "source-control",
      dirty.length ? dirty.slice(0, 8).map((r) => repoRow(r)).join("")
        : empty("check", "Everything is committed", ["No tracked repository has uncommitted changes."]),
      dirty.length ? `<span class="cnt">${dirty.reduce((n, r) => n + r.dirty, 0)} files</span>` : "");

    const hist = (D.history || []).slice(0, 5);
    out += card("resume", "Pick up where you left off", "sparkle",
      hist.length ? hist.map(historyRow).join("")
        : empty("history", "No agent sessions recorded yet",
            [`Runs launched from Launch pad are logged to <code>~/.principia/history/</code>.`,
             `Install the Principia Claude Code plugin and its <code>SessionStart</code> hook records sessions you start anywhere, not only ones launched from here.`],
            `<button class="btn" data-tab-go="setup">${ico("gear", "sm")} Go to Setup</button>`));
    return out;
  }

  function tTasksInner() {
    rows = [];
    const all = allTasks().filter((t) => hit(t.title) || hit(t.repo) || hit(t.notes));
    if (!allTasks().length) {
      const anyRunner = (D.runners || []).some((x) => x.available);
      return card("t", "Tasks", "checklist",
        empty("checklist", "No tasks yet",
          [`A repository declares its own short-lived work in <code>.principia/repo.json</code> under <code>tasks</code>. Cross-repo focus lives in <code>~/.principia/board.json</code>.`,
           `Neither is written by hand. Ask an agent for whichever you need.`],
          `<button class="btn p" data-plan="1" ${anyRunner ? "" : "disabled"}>${ico("sparkle", "sm")} Ask agent to plan my day</button>
           <button class="btn" data-tab-go="repos">${ico("repo", "sm")} Go to Repos</button>`));
    }
    if (!all.length) return card("t", "Tasks", "checklist", empty("search", "Nothing matches", ["Try a different filter."]));

    const groups = [...new Set(all.map((t) => t.group || ""))];
    return groups.map((g) => {
      const ts = all.filter((t) => (t.group || "") === g)
        .sort((a, b) => (a.status === "done") - (b.status === "done") || byPri(a, b));
      const open = ts.filter((t) => t.status !== "done").length;
      return card(`t-${g || "none"}`, g || "Ungrouped", "checklist", ts.map(taskRow).join(""),
        `<span class="cnt">${open} open · ${ts.length} total</span>`);
    }).join("");
  }

  function tWorkflowsInner() {
    rows = [];
    const declared = (D.repos || []).filter((r) => r.flows.some((f) => f.source === "declared")).filter((r) => hit(r.name));
    if (!declared.length) {
      const anyRunner = (D.runners || []).some((x) => x.available);
      const s0 = firstUnconfigured();
      return card("w", "Workflows", "rocket",
        empty("rocket", "No repository has declared its flows yet",
          [`A <b>workflow</b> is a flow a repo declares on purpose: a real label, a port, an ordering, a composite command. Anything auto-detected from <code>package.json</code>, Cargo, Make or Gradle shows under <b>Scripts</b> instead, with no setup at all.`,
           s0 ? `Ask an agent to look at <b>${esc(s0.name)}</b> and decide what is worth declaring.` : `Ask an agent, in any repo, to declare what it offers.`],
          s0 ? `<button class="btn p" data-setup="${esc(s0.root)}" ${anyRunner ? "" : "disabled"}>${ico("sparkle", "sm")} Ask agent to set up ${esc(s0.name)}</button>`
             : `<button class="btn" data-tab-go="repos">${ico("repo", "sm")} Go to Repos</button>`));
    }
    const groups = [...new Set(declared.map((r) => r.group || ""))];
    return groups.map((g) => {
      const rs = declared.filter((r) => (r.group || "") === g);
      const n = rs.reduce((t, r) => t + r.flows.filter((f) => f.source === "declared").length, 0);
      return card(`w-${g || "none"}`, g || "Ungrouped", "rocket",
        rs.map((r) => repoRow(r) + flowStrip(r, r.flows.filter((f) => f.source === "declared"))).join(""),
        `<span class="cnt">${n} flow${n === 1 ? "" : "s"} · ${rs.length} repo${rs.length === 1 ? "" : "s"}</span>`);
    }).join("");
  }

  function tReposInner() {
    rows = [];
    if (!(D.repos || []).length) {
      return card("r", "Repos", "repo",
        empty("repo", "No repositories discovered",
          ["Open a git repository in this editor once and it shows up here, ranked by how recently you used it."],
          `<button class="btn" data-cmd="workbench.action.files.openFolder">${ico("folder-opened", "sm")} Open a folder</button>`));
    }
    const rs = (D.repos || []).filter((r) => hit(r.name) || hit(r.summary) || hit(r.group));
    if (!rs.length) return card("r", "Repos", "repo", empty("search", "Nothing matches", ["Try a different filter."]));
    const groups = [...new Set(rs.map((r) => r.group || ""))];
    return groups.map((g) => {
      const list = rs.filter((r) => (r.group || "") === g);
      const unset = list.filter((r) => !r.configured).length;
      return card(`r-${g || "none"}`, g || "Ungrouped", "repo", list.map((r) => repoRow(r)).join(""),
        `<span class="cnt">${list.length}${unset ? ` · ${unset} not set up` : ""}</span>`);
    }).join("");
  }

  function tAgents() {
    rows = [];
    const rail = runnerCard(D.runners);

    const hist = (D.history || []).slice(0, 8);
    const activity = card("activity", "Recent activity", "history",
      hist.length ? hist.map(historyRow).join("")
        : empty("history", "No sessions recorded yet",
            [`Agent runs launched from here, and any Claude Code session once the plugin's <code>SessionStart</code>/<code>SessionEnd</code> hooks are installed, log to <code>~/.principia/history/</code>.`,
             `Hooks only cover sessions that start after the plugin is installed — a session already running when you install it will not appear retroactively.`]));

    const configuredRepos = (D.repos || []).filter((r) => r.configured).filter((r) => hit(r.name));
    const withSessions = configuredRepos.filter((r) => r.sessions && r.sessions.length);
    const sessionsMain = withSessions.length
      ? withSessions.map((r) => card(`s-${r.dirName}`, `${r.name} sessions`, "comment-discussion",
          r.sessions.map((s, i) => sessionRow(r, s, i === 0)).join(""),
          `<button class="ib" data-resume="${esc(r.sessions[0].id)}" data-root="${esc(r.root)}" title="Continue latest session">${ico("play")}</button>
           <button class="ib" data-new-session="${esc(r.root)}" title="Start a new session here">${ico("add")}</button>
           <span class="cnt">${r.sessions.length}</span>`)).join("")
      : card("s", "Claude Code sessions", "comment-discussion",
          empty("comment-discussion", "No sessions to show",
            [`Real session history is read from <code>~/.claude/projects/</code>, but only for a repo that has opted in by committing <code>.principia/repo.json</code> — that is the permission gate, enforced before the transcript is ever read, not just before it is shown.`,
             configuredRepos.length ? `${configuredRepos.length} repo(s) here are configured but none has a recorded session yet.` : `No repository here is configured yet.`]));

    const withAgents = (D.repos || []).filter((r) => r.agents.length).filter((r) => hit(r.name));
    if (!withAgents.length) {
      const anyRunner = (D.runners || []).some((x) => x.available);
      const s0 = firstUnconfigured();
      const main = card("a", "Repo agents", "sparkle",
        empty("sparkle", "No repository ships an agent yet",
          [`A repo declares agents as prompt files in <code>.principia/agents/*.md</code>, committed alongside its code. They are runner-agnostic: the same prompt runs under Claude Code, Codex, Gemini CLI or agy.`,
           s0 ? `Ask an agent to look at <b>${esc(s0.name)}</b> and decide whether it needs one.` : `Ask an agent, in any repo, whether it needs one.`],
          s0 ? `<button class="btn p" data-setup="${esc(s0.root)}" ${anyRunner ? "" : "disabled"}>${ico("sparkle", "sm")} Ask agent to set up ${esc(s0.name)}</button>`
             : `<button class="btn" data-tab-go="repos">${ico("repo", "sm")} Go to Repos</button>`));
      return layout(sessionsMain + main, rail + activity);
    }
    const main = withAgents.map((r) => card(`a-${r.dirName}`, r.name, "sparkle",
      r.agents.map((a) => `<div ${F(`data-root="${esc(r.root)}" data-agent="${esc(a.id)}"`)}>
        <span class="ic">${ico("sparkle")}</span>
        <div class="main"><div class="l1"><b>${esc(a.label || a.id)}</b></div>
          <div class="l2"><span class="mono dim">${esc(a.file)}</span>${(a.supports || []).map((s) => `<span class="chip">${esc(s)}</span>`).join("")}</div></div>
        <div class="rslot n3"><span class="sa">
          <button class="ib" data-open-file="${esc(r.root)}/${esc(a.file)}" title="Open the prompt file">${ico("go-to-file")}</button>
          <button class="btn" data-agent="${esc(a.id)}" data-root="${esc(r.root)}">${ico("play", "sm")} Run</button>
        </span></div>
      </div>`).join("")
    )).join("");
    return layout(sessionsMain + main, rail + activity);
  }

  function tScriptsInner() {
    rows = [];
    const rs = (D.repos || []).filter((r) => r.flows.some((f) => f.source === "detected")).filter((r) => hit(r.name));
    if (!rs.length) {
      return card("s", "Scripts", "play",
        empty("play", "Nothing auto-detected",
          [`Scripts are found with zero config from <code>package.json</code>, <code>Cargo.toml</code>, <code>pyproject.toml</code>, <code>Makefile</code> and Gradle.`,
           `None of your recent repositories expose any.`]));
    }
    return rs.map((r) => card(`s-${r.dirName}`, r.name, "play",
      `<div class="chips pad">${r.flows.filter((f) => f.source === "detected").map((f) =>
        `<button class="chip act" data-flow="${esc(f.id)}" data-root="${esc(r.root)}" title="${esc(f.run)}">${ico(f.kind === "server" ? "play" : f.kind === "test" ? "beaker" : "tools", "sm")}${esc(f.label)}</button>`).join("")}</div>`,
      `<span class="cnt mono">${esc(r.branch)}</span>`)).join("");
  }

  function tRecentsInner() {
    rows = [];
    const rs = (D.recents || []).filter((r) => hit(r.name));
    if (!rs.length) {
      return card("re", "Recents", "history",
        empty("history", "No recent projects",
          ["This mirrors your editor's own recently-opened list, filtered to git repositories."],
          `<button class="btn" data-cmd="workbench.action.files.openFolder">${ico("folder-opened", "sm")} Open a folder</button>`));
    }
    return card("re", "Recently opened", "history",
      rs.map((r) => `<div ${F(`data-root="${esc(r.root)}"`)}>
        <span class="ic">${ico("repo")}</span>
        <div class="main"><div class="l1"><b>${esc(r.name)}</b>${r.ws ? `<span class="chip">workspace</span>` : ""}</div></div>
        <div class="rslot n3">
          <span class="sm">${gitBits(r)}</span>
          <span class="sa">
            <button class="ib" data-term="${esc(r.root)}" title="Terminal here">${ico("terminal")}</button>
            <button class="ib" data-open="${esc(r.root)}" title="Open">${ico("folder-opened")}</button>
            <button class="ib" data-open="${esc(r.root)}" data-new="1" title="Open in new window">${ico("empty-window")}</button>
          </span>
        </div>
      </div>`).join(""), `<span class="cnt">${rs.length}</span>`);
  }

  function tDevices() {
    rows = [];
    const a = D.android || {}, i = D.ios || {};

    const rail = card("android", "Android", "device-mobile",
      !a.available ? empty("circle-slash", "Android SDK not found", [esc(a.reason || "Install the Android SDK to boot an emulator from here.")])
      : `<div class="row"><span class="ic">${ico("device-mobile")}</span>
          <div class="main"><div class="l1"><b>${a.booted ? esc(a.device) : "No emulator running"}</b>${a.booted ? `<span class="chip ok">booted</span>` : ""}</div>
          <div class="l2"><span class="dim">${a.avds && a.avds.length ? esc(a.avds.join(", ")) : "no AVDs configured"}</span></div></div>
          <div class="rslot static">${a.avds && a.avds.length ? `<button class="btn" data-android="${a.booted ? "stop" : "boot"}">${ico(a.booted ? "debug-stop" : "play", "sm")} ${a.booted ? "Stop" : "Boot"}</button>` : ""}</div>
        </div>`);

    let main = card("ios", "iOS simulators", "device-mobile",
      !i.available ? empty("circle-slash", "Xcode tools not found", [esc(i.reason || "Install Xcode command line tools to use simulators.")])
      : !(i.all || []).length ? empty("device-mobile", "No simulators available", ["Create one in Xcode."])
      : i.all.slice(0, 8).map((d) => `<div class="row">
          <span class="ic">${ico("device-mobile")}</span>
          <div class="main"><div class="l1"><b>${esc(d.name)}</b>${d.state === "Booted" ? `<span class="chip ok">booted</span>` : ""}</div></div>
          <div class="rslot static"><button class="btn" data-ios="${d.state === "Booted" ? "shutdown" : "boot"}" data-udid="${esc(d.udid)}">${d.state === "Booted" ? "Shutdown" : "Boot"}</button></div>
        </div>`).join(""),
      i.available ? `<span class="cnt">${(i.booted || []).length} booted · ${(i.all || []).length} available</span>` : "");

    const ports = D.ports || [];
    main += card("ports", "Listening ports", "radio-tower",
      ports.length ? ports.map((p) => `<div class="row">
          <span class="ic">${ico("radio-tower")}</span>
          <div class="main"><div class="l1"><b>${p.port}</b><span class="chip">${esc(p.cmd || "")}</span></div></div>
          <div class="rslot static">
            <button class="ib" data-browser="http://localhost:${p.port}" title="Simple Browser">${ico("globe")}</button>
            <button class="ib" data-external="http://localhost:${p.port}" title="External browser">${ico("link-external")}</button>
          </div>
        </div>`).join("")
      : empty("radio-tower", "Nothing listening", ["Dev servers you start from here show up with a link to open them."]),
      ports.length ? `<span class="cnt">${ports.length}</span>` : "");
    return layout(main, rail);
  }

  // The getting-started flow, in the order the product actually works:
  // runner -> a repo opts in -> it declares flows -> it ships an agent ->
  // it tracks tasks -> one agent plans across all of them.
  //
  // Two rules here, both deliberate:
  //  1. Every step's "done" is DERIVED from data already on screen, never
  //     stored. There is no checkbox state to go stale, and a step that stops
  //     being true (a repo's contribution is deleted) correctly un-completes.
  //  2. Nothing auto-collapses. An earlier version of this hid each step as it
  //     completed, which left you unable to see what you had done or repeat it.
  //     Done steps dim and keep their action; they never disappear.
  // Which repo the walkthrough is about: the folder actually open, not the
  // fleet. Aggregating across every repo ever opened reported "6 of 6, all
  // done" while the project in front of you had no .principia/ at all — the
  // checklist has to answer "is THIS project wired up", or it answers nothing.
  function hereRepo() {
    const here = D.here;
    if (!here) return null;
    const repos = D.repos || [];
    return repos.find((r) => r.root === here)
      || repos.find((r) => here.startsWith(r.root + "/"))
      // Open folder that discovery has not listed yet: still offer to set it up.
      || { root: here, name: base(here), configured: false, flows: [], agents: [], tasks: [], unlisted: true };
  }

  function walkthroughSteps() {
    const runners = D.runners || [];
    const avail = runners.filter((r) => r.available);
    const runnable = avail.length > 0;
    const board = D.board || [];
    const repo = hereRepo();

    // Machine-level and cross-repo steps stay global; they are not properties
    // of one folder and pretending otherwise would be a lie.
    const runnerStep = {
      id: "runner",
      title: "Install an agent runner",
      why: `Every file Principia shows is written by an agent, never by you. No runner is privileged: Claude Code, Codex, Gemini CLI and agy all read the same prompts.`,
      done: runnable,
      evidence: runnable ? avail.map((r) => esc(r.label)).join(", ") : "none found on PATH",
      action: `<button class="btn" data-act="redetect">${ico("refresh", "sm")} Re-detect</button>`,
      global: true,
    };
    const boardStep = {
      id: "board",
      title: "Plan focus across every repo",
      why: `One agent looks at everything Principia knows about — uncommitted work, unpushed branches, open tasks — and writes the cross-repo board at <code>~/.principia/board.json</code>.`,
      done: board.length > 0,
      evidence: board.length ? `${board.length} item(s) on the board` : "board is empty",
      action: `<button class="btn" data-plan="1" ${runnable ? "" : "disabled"} title="${runnable ? "" : "No agent runner is installed"}">${ico("sparkle", "sm")} Plan my day</button>`,
      global: true,
    };

    // No folder open: the repo steps have no subject, so say that plainly
    // rather than quietly grading them against some other repo.
    if (!repo) {
      const openFolder = `<button class="btn" data-cmd="workbench.action.files.openFolder">${ico("folder-opened", "sm")} Open a folder</button>`;
      const blank = (id, title, why) => ({ id, title, why, done: false, evidence: "no folder open", action: openFolder });
      return [
        runnerStep,
        blank("opt-in", "Let this repository describe itself", `Open a project and an agent can read it and commit its <code>.principia/repo.json</code>.`),
        blank("flows", "Declare a workflow worth launching", `A declared flow adds what detection cannot infer: a real label, a port, which one is primary.`),
        blank("agents", "Ship an agent with the repo", `A prompt committed at <code>.principia/agents/*.md</code> becomes a one-click action for anyone who clones it.`),
        blank("tasks", "Track work as tasks", `Tasks in <code>repo.json</code> get a "work on this" action that hands them straight to a runner.`),
        boardStep,
      ];
    }

    const name = esc(repo.name);
    const flows = (repo.flows || []).filter((f) => f.source === "declared");
    const agents = repo.agents || [];
    const tasks = repo.tasks || [];
    const ask = (label, primary) =>
      `<button class="btn${primary ? " p" : ""}" data-setup="${esc(repo.root)}" ${runnable ? "" : "disabled"} title="${runnable ? `Ask an agent to work on ${name}` : "No agent runner is installed"}">${ico("sparkle", "sm")} ${label}</button>`;
    // Every step is something you can hand to an agent, not just a link to a
    // tab. The prompt lives in prompts/<id>.md so all runners get the same
    // instruction; the tab button stays as a secondary way to go look.
    const askPrompt = (id, label, primary) =>
      `<button class="btn${primary ? " p" : ""}" data-prompt="${id}" data-root="${esc(repo.root)}" ${runnable ? "" : "disabled"} title="${runnable ? `Ask an agent, in ${name}` : "No agent runner is installed"}">${ico("sparkle", "sm")} ${label}</button>`;
    const goTab = (tab, icon, title) =>
      `<button class="ib" data-tab-go="${tab}" title="${title}">${ico(icon)}</button>`;

    return [
      runnerStep,
      {
        id: "opt-in",
        title: `Let ${name} describe itself`,
        why: `An agent reads this repo and commits <code>.principia/repo.json</code>. It travels with the clone, so your other machine and your teammates get it for free — and it is what unlocks session history for this repo.`,
        done: !!repo.configured,
        evidence: repo.configured
          ? `${name} is configured`
          : (repo.unlisted ? `${name} has no .principia/ yet` : `${name} has not opted in yet`),
        action: ask(repo.configured ? "Re-run setup" : `Set up ${name}`, !repo.configured),
      },
      {
        id: "flows",
        title: "Declare a workflow worth launching",
        why: `Scripts from <code>package.json</code>, Cargo, Make and Gradle are already detected with zero config. A declared flow adds only what detection cannot infer: a real label, a port, a composite command, which one is primary.`,
        done: flows.length > 0,
        evidence: flows.length ? `${flows.length} declared in ${name}` : `${name} declares none yet`,
        action: askPrompt("declare-flows", "Declare flows", !flows.length) + goTab("workflows", "rocket", "Go to Workflows"),
      },
      {
        id: "agents",
        title: "Ship an agent with the repo",
        why: `A prompt committed at <code>.principia/agents/*.md</code> becomes a one-click action for anyone who clones this repo, under whichever runner they happen to have.`,
        done: agents.length > 0,
        evidence: agents.length ? `${agents.length} in ${name}` : `${name} ships none yet`,
        action: askPrompt("add-agent", "Add an agent", !agents.length) + goTab("agents", "sparkle", "Go to Agents"),
      },
      {
        id: "tasks",
        title: "Track work as tasks",
        why: `Tasks in <code>repo.json</code> show up here with a "work on this" action that hands the task, and any agent it names, straight to a runner.`,
        done: tasks.length > 0,
        evidence: tasks.length
          ? `${tasks.filter((t) => t.status !== "done").length} open of ${tasks.length} in ${name}`
          : `${name} tracks none yet`,
        action: askPrompt("track-tasks", "Find tasks", !tasks.length) + goTab("tasks", "checklist", "Go to Tasks"),
      },
      boardStep,
    ];
  }

  function walkthrough() {
    const steps = walkthroughSteps();
    const repo = hereRepo();
    const done = steps.filter((s) => s.done).length;
    const nextId = (steps.find((s) => !s.done) || {}).id;
    const pct = Math.round((done / steps.length) * 100);

    return `<section class="card" data-sec="walk">
      <header>${ico("rocket", "sm")}<h3>Getting started</h3>
        ${repo ? `<span class="chip">${ico("repo", "sm")}${esc(repo.name)}</span>`
               : `<span class="chip">no folder open</span>`}
        <span class="grow"></span>
        <span class="cnt">${done} of ${steps.length}</span></header>
      <div class="bar"><i style="width:${pct}%"></i></div>
      <div class="cbody"><ol class="wt">
        ${steps.map((s, i) => `<li class="wtstep${s.done ? " done" : ""}${s.id === nextId ? " next" : ""}">
          <span class="wtn">${s.done ? ico("pass-filled") : i + 1}</span>
          <div class="wtb">
            <div class="wth"><b>${s.title}</b>
              ${s.id === nextId ? `<span class="chip hi">next</span>` : ""}
              ${s.global ? `<span class="chip" title="Applies to this machine, not just the open folder">all repos</span>` : ""}
            </div>
            <p class="nt">${s.why}</p>
            <div class="wte">${s.done ? ico("check", "sm") : ""}<span class="dim">${s.evidence}</span></div>
          </div>
          <div class="wta">${s.action}</div>
        </li>`).join("")}
      </ol></div>
    </section>`;
  }

  function tSetup() {
    rows = [];
    const unset = (D.repos || []).filter((r) => !r.configured);
    const runners = D.runners || [];
    const runnable = runners.some((r) => r.available);
    const configuredCount = (D.repos || []).length - unset.length;

    let main = walkthrough();

    main += card("todo", "Waiting on setup", "repo",
      unset.length ? unset.map((r) => `<div ${F(`data-root="${esc(r.root)}"`)}>
          <span class="ic">${ico(r.icon || "repo")}</span>
          <div class="main"><div class="l1"><b>${esc(r.name)}</b></div>
            ${r.summary ? `<div class="l2"><span class="nt">${esc(r.summary)}</span></div>` : ""}
          </div>
          <div class="rslot n3">
            <span class="sm">${gitBits(r) || `<span class="dim">not set up</span>`}</span>
            <span class="sa">
              ${runnable ? `<button class="btn" data-setup="${esc(r.root)}" title="Ask agent to set this up">${ico("sparkle", "sm")} Set up</button>` : ""}
              <button class="ib" data-open="${esc(r.root)}" title="Open">${ico("folder-opened")}</button>
            </span>
          </div>
        </div>`).join("")
        : empty("check", "Every repository has opted in", ["Nothing left to set up."]),
      unset.length ? `<span class="cnt">${configuredCount}/${(D.repos || []).length} set up</span>` : "");

    const rail = runnerCard(runners, runnable ? "Runners" : "No agent runner found")
      + card("how", "How this fills up", "lightbulb",
        `<div class="empty">
          <p>Launch pad ships empty on purpose. It reads the projects you recently opened, then asks each one what it offers.</p>
          <p>A repository answers by committing a <code>.principia/</code> directory: its flows, the agents it ships, and its current work. You never fill this in by hand, you ask an agent to.</p>
          <p>Scripts auto-detected from <code>package.json</code>, Cargo, Make and Gradle need no setup at all. Setting up a repo only adds what detection cannot infer.</p>
        </div>`);

    return layout(main, rail);
  }

  /* ───────── render ───────── */

  const RENDER = { today: tToday, tasks: tTasks, workflows: tWorkflows, repos: tRepos, agents: tAgents, scripts: tScripts, recents: tRecents, devices: tDevices, setup: tSetup };

  function paintBody(animate) {
    const body = document.getElementById("body");
    if (!body) return;
    if (!D) { body.innerHTML = `<div class="empty">${ico("loading", "spin big")}<b>Reading your recent projects…</b></div>`; return; }

    let html;
    try { html = (RENDER[tab] || tToday)(); }
    catch (e) { html = `<div class="empty">${ico("error", "big")}<b>Render failed</b><p>${esc(e.message)}</p></div>`; }

    const next = document.createElement("div");
    next.innerHTML = html;
    const incoming = [...next.querySelectorAll("[data-sec]")];
    const currentIds = [...body.querySelectorAll("[data-sec]")].map((n) => n.dataset.sec);

    if (animate || currentIds.join("|") !== incoming.map((n) => n.dataset.sec).join("|")) {
      const y = body.scrollTop;
      body.innerHTML = html;
      painted.clear();
      for (const n of incoming) painted.set(n.dataset.sec, n.outerHTML);
      if (animate) { body.classList.remove("in"); void body.offsetWidth; body.classList.add("in"); }
      else body.scrollTop = y;
    } else {
      for (const n of incoming) {
        const id = n.dataset.sec;
        if (painted.get(id) === n.outerHTML) continue;
        const live = body.querySelector(`[data-sec="${CSS.escape(id)}"]`);
        if (live) live.replaceWith(n.cloneNode(true));
        painted.set(id, n.outerHTML);
      }
    }
    if (focus >= 0) setFocus(focus, true);
  }

  function render(animate) {
    if (!built) buildShell();
    paintChrome();
    paintBody(animate);
  }

  function setTab(id) {
    if (id === tab) return;
    tab = id; focus = -1; persist();
    paintChrome(); paintBody(true);
    document.getElementById("body").scrollTo({ top: 0, behavior: "smooth" });
  }

  /* ───────── interaction ───────── */

  function wire() {
    const qi = document.getElementById("q");
    let t = null;
    qi.addEventListener("input", () => {
      clearTimeout(t);
      t = setTimeout(() => { q = qi.value.trim().toLowerCase(); focus = -1; persist(); paintBody(false); }, 70);
    });
    document.getElementById("tabs").addEventListener("click", (ev) => {
      const b = ev.target.closest("[data-tab]"); if (b) setTab(b.dataset.tab);
    });
    app.addEventListener("click", (ev) => {
      const el = ev.target.closest("[data-act],[data-open],[data-open-file],[data-setup],[data-plan],[data-prompt],[data-flow],[data-agent],[data-task],[data-resume],[data-reveal],[data-copy],[data-new-session],[data-term],[data-android],[data-ios],[data-browser],[data-external],[data-tab-go],[data-cmd]");
      if (!el || el.disabled) return;
      ev.stopPropagation();
      const d = el.dataset;
      if (d.tabGo) return setTab(d.tabGo);
      if (d.act === "refresh") { spin(); return send({ type: "refresh" }); }
      if (d.act) return send({ type: d.act });
      if (d.setup) return send({ type: "setup", root: d.setup });
      if (d.plan) return send({ type: "plan" });
      if (d.prompt) return send({ type: "prompt", id: d.prompt, root: d.root });
      if (d.cmd) return send({ type: "command", id: d.cmd });
      if (d.flow) return send({ type: "flow", root: d.root, id: d.flow });
      if (d.agent && d.root) return send({ type: "agent", root: d.root, id: d.agent });
      if (d.task && d.root) return send({ type: "task", root: d.root, id: d.task });
      if (d.resume && d.root) return send({ type: "resume", root: d.root, id: d.resume });
      if (d.openFile) return send({ type: "openFile", path: d.openFile });
      if (d.reveal) return send({ type: "reveal", path: d.reveal });
      if (d.copy) return send({ type: "copy", text: d.copy });
      if (d.newSession) return send({ type: "newSession", root: d.newSession });
      if (d.term) return send({ type: "terminal", root: d.term });
      if (d.android) return send({ type: "android", action: d.android });
      if (d.ios) return send({ type: "ios", action: d.ios, udid: d.udid });
      if (d.browser) return send({ type: "browser", url: d.browser });
      if (d.external) return send({ type: "external", url: d.external });
      if (d.open) return send({ type: "open", root: d.open, newWindow: d.new === "1" });
    });
    app.addEventListener("mousemove", (ev) => {
      const r = ev.target.closest(".row.focusable"); if (!r) return;
      const it = items(), i = it.indexOf(r);
      if (i >= 0 && i !== focus) { focus = i; it.forEach((e, n) => e.classList.toggle("on", n === i)); }
    });
  }

  function spin() {
    const i = document.querySelector('[data-act="refresh"] .codicon');
    if (i) { i.classList.add("spin"); setTimeout(() => i.classList.remove("spin"), 700); }
  }

  const items = () => [...app.querySelectorAll(".row.focusable")];
  function setFocus(n, quiet) {
    const it = items(); if (!it.length) { focus = -1; return; }
    focus = Math.max(0, Math.min(it.length - 1, n));
    it.forEach((e, i) => e.classList.toggle("on", i === focus));
    if (!quiet) it[focus].scrollIntoView({ block: "nearest", behavior: "smooth" });
  }

  document.addEventListener("keydown", (ev) => {
    const inInput = ["INPUT", "TEXTAREA", "SELECT"].includes(ev.target.tagName);
    if (!inInput && ev.key >= "1" && ev.key <= "9") return setTab(TABS[+ev.key - 1][0]);
    if (ev.key === "/" && !inInput) { ev.preventDefault(); return document.getElementById("q").focus(); }
    if (ev.key === "Escape") {
      const qi = document.getElementById("q");
      if (q) { q = ""; qi.value = ""; persist(); paintBody(false); }
      qi.blur(); focus = -1; items().forEach((e) => e.classList.remove("on"));
      return;
    }
    if (ev.key === "ArrowDown") { ev.preventDefault(); if (inInput) ev.target.blur(); return setFocus(focus + 1); }
    if (ev.key === "ArrowUp") { ev.preventDefault(); if (inInput) ev.target.blur(); return setFocus(focus - 1); }
    if (inInput) return;
    const cur = items()[focus];
    if (ev.key === "Enter" && cur && cur.dataset.root) return send({ type: "open", root: cur.dataset.root, newWindow: ev.shiftKey });
    if (ev.key === "r") { spin(); send({ type: "refresh" }); }
  });

  window.addEventListener("message", (ev) => {
    const m = ev.data;
    if (m.type === "data") { D = m; D.error = null; persist(); render(false); }
    if (m.type === "error") {
      D = D || { repos: [], runners: [], board: [], history: [], recents: [], ports: [], android: {}, ios: {}, counts: {}, machine: {} };
      D.error = m.message; render(false);
    }
  });

  render(true);
  send({ type: "ready" });
})();
