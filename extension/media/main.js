// Principia UI.
//
// Two rules this file exists to enforce:
//
// 1. The shell is built once. Only section bodies are patched, and only the
//    sections whose HTML actually changed. A background poll must not destroy
//    scroll position, hover, keyboard focus, or the caret in the filter box.
// 2. Nothing may cause horizontal scroll. Every flex/grid child gets min-width:0
//    and long strings wrap. The page scrolls one way: down.

(function () {
  const vscode = acquireVsCodeApi();
  const app = document.getElementById("app");
  const saved = vscode.getState() || {};

  let D = saved.D || null;
  let view = saved.view || "focus";
  let q = saved.q || "";
  let focus = -1;
  let built = false;
  const painted = new Map();   // section id -> last html

  const VIEWS = [
    ["focus", "Focus", "target"],
    ["repos", "Repos", "repo"],
    ["agents", "Agents", "sparkle"],
    ["activity", "Activity", "history"],
  ];

  const send = (m) => vscode.postMessage(m);
  const persist = () => vscode.setState({ D, view, q });
  const esc = (s) => String(s == null ? "" : s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  // `n` can come from a repo's committed repo.json, so constrain it to the
  // characters codicon names actually use rather than trusting the file.
  const ico = (n, cls = "") => {
    const safe = String(n || "").replace(/[^a-z0-9-]/g, "");
    return `<span class="ci codicon codicon-${safe} ${cls}"></span>`;
  };
  const hit = (s) => !q || String(s || "").toLowerCase().includes(q);

  /* ───────── shell ───────── */

  function buildShell() {
    app.innerHTML = `
      <header class="bar">
        <div class="brand">${ico("compass")}<span>Principia</span></div>
        <div class="where" id="where"></div>
        <div class="spring"></div>
        <div class="tally" id="tally"></div>
        <button class="ib" data-act="refresh" title="Refresh (r)">${ico("refresh")}</button>
        <button class="ib" data-act="openSpec" title="Open the contract">${ico("book")}</button>
      </header>

      <nav class="nav" id="nav">
        ${VIEWS.map(([id, label, icon], i) =>
          `<button class="nb" data-view="${id}">${ico(icon, "sm")}<span>${label}</span><b class="cnt" data-cnt="${id}"></b><kbd>${i + 1}</kbd></button>`
        ).join("")}
      </nav>

      <label class="find">${ico("search", "sm")}<input id="q" placeholder="Filter" autocomplete="off" spellcheck="false"><kbd>/</kbd></label>

      <div class="warn" id="warn" hidden></div>
      <main id="body"></main>

      <footer class="foot">
        <span id="runners"></span>
        <div class="spring"></div>
        <span class="keys"><kbd>↑↓</kbd> move <kbd>⏎</kbd> open <kbd>1-4</kbd> views <kbd>r</kbd> refresh</span>
      </footer>`;
    document.getElementById("q").value = q;
    wire();
    built = true;
  }

  /* ───────── chrome, patched in place ───────── */

  function paintChrome() {
    if (!D) return;
    const set = (id, html) => {
      const e = document.getElementById(id);
      if (e && e.innerHTML !== html) e.innerHTML = html;
    };
    const c = D.counts || {};
    set("where", D.here ? esc(D.here.split("/").pop()) : "no folder open");
    set("tally", [
      `<b>${c.repos || 0}</b> repos`,
      `<b>${c.configured || 0}</b> configured`,
      c.dirty ? `<b class="hot">${c.dirty}</b> dirty` : "",
    ].filter(Boolean).join("<i>·</i>"));

    const counts = {
      focus: (D.board || []).length,
      repos: (D.repos || []).length,
      agents: c.agents || 0,
      activity: (D.history || []).length,
    };
    for (const [id] of VIEWS) {
      const b = document.querySelector(`[data-cnt="${id}"]`);
      if (!b) continue;
      const v = counts[id] ? String(counts[id]) : "";
      if (b.textContent !== v) b.textContent = v;
      b.hidden = !v;
    }
    document.querySelectorAll(".nb").forEach((n) => n.classList.toggle("on", n.dataset.view === view));

    const rs = (D.runners || []).map((r) =>
      `<span class="rn ${r.available ? "ok" : "no"}" title="${esc(r.reason || r.pathTo || "")}">${ico(r.available ? "pass" : "circle-slash", "sm")}${esc(r.label)}</span>`
    ).join("");
    set("runners", rs || "");

    const w = document.getElementById("warn");
    if (w) {
      const msg = D.error ? esc(D.error)
        : !D.contract ? `The contract is not on disk, so agent setup is unavailable. Set <code>principia.specPath</code> to a checkout of the Principia repo.`
        : "";
      w.hidden = !msg;
      if (msg && w.innerHTML !== msg) w.innerHTML = msg;
    }
  }

  /* ───────── body: per-section diffing ───────── */

  function section(id, title, inner, aside = "") {
    return `<section class="sec" data-sec="${esc(id)}">
      <h2>${esc(title)}<div class="spring"></div>${aside}</h2>
      <div class="secbody">${inner}</div>
    </section>`;
  }

  function paintBody(animate) {
    const body = document.getElementById("body");
    if (!body) return;
    if (!D) { body.innerHTML = `<div class="blank">${ico("loading", "spin")} Reading your recent projects…</div>`; return; }

    const html = ({ focus: vFocus, repos: vRepos, agents: vAgents, activity: vActivity }[view] || vFocus)();

    // Diff at the section level: a poll that changes nothing touches no DOM.
    const next = document.createElement("div");
    next.innerHTML = html;
    const incoming = [...next.querySelectorAll("[data-sec]")];
    const currentIds = [...body.querySelectorAll("[data-sec]")].map((n) => n.dataset.sec);
    const nextIds = incoming.map((n) => n.dataset.sec);

    if (animate || currentIds.join("|") !== nextIds.join("|")) {
      const y = window.scrollY;
      body.innerHTML = html;
      painted.clear();
      for (const n of incoming) painted.set(n.dataset.sec, n.outerHTML);
      if (animate) { body.classList.remove("in"); void body.offsetWidth; body.classList.add("in"); }
      else window.scrollTo(0, y);
    } else {
      for (const n of incoming) {
        const id = n.dataset.sec;
        if (painted.get(id) === n.outerHTML) continue;      // unchanged: leave the DOM alone
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

  function setView(id) {
    if (id === view) return;
    view = id; focus = -1; persist();
    paintChrome(); paintBody(true);
    window.scrollTo({ top: 0, behavior: "smooth" });
  }

  /* ───────── rows ───────── */

  let rows = [];
  const F = (attrs) => { rows.push(attrs); return `class="row focusable" data-i="${rows.length - 1}" ${attrs}`; };

  const gitBits = (r) => [
    r.branch ? `<span class="mono">${esc(r.branch)}</span>` : "",
    r.dirty ? `<span class="g d">${r.dirty} changed</span>` : "",
    r.ahead ? `<span class="g a">↑${r.ahead}</span>` : "",
    r.behind ? `<span class="g b">↓${r.behind}</span>` : "",
  ].filter(Boolean).join("");

  function repoRow(r) {
    const runnable = (D.runners || []).some((x) => x.available);
    return `<div ${F(`data-root="${esc(r.root)}"`)}>
      <span class="ic">${ico(r.icon || "repo")}</span>
      <div class="body">
        <div class="line1">
          <b>${esc(r.name)}</b>
          ${r.configured ? `<span class="pill ok">configured</span>` : `<span class="pill">not set up</span>`}
          ${r.invalid ? `<span class="pill bad">${esc(r.invalid)}</span>` : ""}
          ${r.staleBy ? `<span class="pill warn">${r.staleBy} commits since setup</span>` : ""}
        </div>
        ${r.summary ? `<div class="line2">${esc(r.summary)}</div>` : ""}
        <div class="meta">${gitBits(r)}${r.flows.length ? `<span>${r.flows.length} flow${r.flows.length > 1 ? "s" : ""}${r.configured ? "" : " detected"}</span>` : ""}${r.agents.length ? `<span>${r.agents.length} agent${r.agents.length > 1 ? "s" : ""}</span>` : ""}</div>
        ${r.flows.length ? `<div class="chips">${r.flows.slice(0, 6).map((f) =>
          `<button class="chip" data-flow="${esc(f.id)}" data-root="${esc(r.root)}" title="${esc(f.run)}">${ico(f.kind === "server" ? "play" : f.kind === "test" ? "beaker" : "tools", "sm")}${esc(f.label)}</button>`
        ).join("")}</div>` : ""}
        ${r.agents.length ? `<div class="chips">${r.agents.map((a) =>
          `<button class="chip ag" data-agent="${esc(a.id)}" data-root="${esc(r.root)}" title="${esc((a.supports || []).join(", "))}">${ico("sparkle", "sm")}${esc(a.label || a.id)}</button>`
        ).join("")}</div>` : ""}
      </div>
      <div class="acts">
        ${!r.configured ? `<button class="btn" data-setup="${esc(r.root)}" ${runnable ? "" : "disabled"} title="${runnable ? "Ask an agent to write .principia/repo.json" : "No agent runner installed"}">${ico("sparkle", "sm")} Set up</button>` : ""}
        <button class="ib" data-open="${esc(r.root)}" title="Open">${ico("folder-opened")}</button>
        <button class="ib" data-open="${esc(r.root)}" data-new="1" title="Open in new window">${ico("empty-window")}</button>
      </div>
    </div>`;
  }

  /* ───────── views ───────── */

  function vFocus() {
    rows = [];
    let out = "";

    if (!D.repos.length) {
      return section("empty", "Nothing discovered yet",
        `<div class="blank">
          <p>Principia reads the projects you have recently opened in this editor. Open a git repository once and it will appear here.</p>
          <p class="dim">Nothing is scanned from your filesystem and nothing is stored in the extension.</p>
        </div>`);
    }

    if (D.board && D.board.length) {
      const items = D.board.filter((f) => hit(f.title) || hit(f.repo));
      out += section("board", "Focus",
        items.map((f) => {
          const r = D.repos.find((x) => x.name === f.repo);
          return `<div ${F(r ? `data-root="${esc(r.root)}"` : "")}>
            <span class="ic">${ico(f.status === "blocked" ? "circle-slash" : f.status === "doing" ? "debug-start" : "circle-large-outline")}</span>
            <div class="body">
              <div class="line1"><b>${esc(f.title)}</b>${f.repo ? `<span class="pill">${esc(f.repo)}</span>` : ""}</div>
              ${f.agent && r ? `<div class="chips"><button class="chip ag" data-agent="${esc(f.agent)}" data-root="${esc(r.root)}">${ico("sparkle", "sm")}${esc(f.agent)}</button></div>` : ""}
            </div>
            <div class="acts">${r ? `<button class="ib" data-open="${esc(r.root)}" title="Open">${ico("folder-opened")}</button>` : ""}</div>
          </div>`;
        }).join("") || `<div class="blank sm">Nothing matches.</div>`,
        D.boardUpdated ? `<span class="age">updated ${esc(String(D.boardUpdated).slice(0, 10))}</span>` : "");
    } else {
      out += section("board", "Focus",
        `<div class="blank sm">
          <p>No cross-repo board yet. It lives at <code>~/.principia/board.json</code> and is written by an agent.</p>
          <p class="dim">Ask any agent: <em>plan my focus across repos, following the Principia plan-day prompt.</em></p>
        </div>`);
    }

    const dirty = D.repos.filter((r) => r.dirty > 0).filter((r) => hit(r.name));
    if (dirty.length) {
      out += section("dirty", "Uncommitted work",
        dirty.slice(0, 8).map(repoRow).join(""),
        `<span class="age">${dirty.reduce((n, r) => n + r.dirty, 0)} files</span>`);
    }

    const unset = D.repos.filter((r) => !r.configured).filter((r) => hit(r.name));
    if (unset.length) {
      out += section("unset", "Not set up yet",
        unset.slice(0, 8).map(repoRow).join(""),
        `<span class="age">${unset.length} of ${D.repos.length}</span>`);
    }
    return out;
  }

  function vRepos() {
    rows = [];
    const groups = (D.groups || []).map((g) => ({
      name: g.name, repos: g.repos.filter((r) => hit(r.name) || hit(r.summary) || hit(r.group)),
    })).filter((g) => g.repos.length);
    if (!groups.length) return section("none", "Repos", `<div class="blank sm">Nothing matches.</div>`);
    return groups.map((g, i) =>
      section(`g-${i}`, g.name || "Ungrouped", g.repos.map(repoRow).join(""), `<span class="age">${g.repos.length}</span>`)
    ).join("");
  }

  function vAgents() {
    rows = [];
    const withAgents = D.repos.filter((r) => r.agents.length);
    let out = section("runners", "Runners",
      `<div class="rgrid">${(D.runners || []).map((r) =>
        `<div class="rcard ${r.available ? "ok" : "no"}">
          <div class="line1">${ico(r.available ? "pass-filled" : "circle-slash")}<b>${esc(r.label)}</b></div>
          <div class="line2">${r.available ? `<span class="mono">${esc(r.pathTo || r.bin)}</span>` : esc(r.reason || "not installed")}</div>
        </div>`).join("")}</div>`,
      `<button class="ib" data-act="redetect" title="Re-detect">${ico("refresh")}</button>`);

    if (!withAgents.length) {
      out += section("noagents", "Repo agents",
        `<div class="blank sm">
          <p>No repository ships an agent yet.</p>
          <p class="dim">A repo declares agents in <code>.principia/agents/*.md</code>. Use <b>Set up</b> on a repo and ask for one.</p>
        </div>`);
      return out;
    }
    out += withAgents.filter((r) => hit(r.name)).map((r) =>
      section(`a-${r.dirName}`, r.name,
        r.agents.map((a) => `<div ${F(`data-root="${esc(r.root)}" data-agent="${esc(a.id)}"`)}>
          <span class="ic">${ico("sparkle")}</span>
          <div class="body">
            <div class="line1"><b>${esc(a.label || a.id)}</b></div>
            <div class="meta"><span class="mono">${esc(a.file)}</span>${(a.supports || []).map((s) => `<span class="pill">${esc(s)}</span>`).join("")}</div>
          </div>
          <div class="acts"><button class="btn" data-agent="${esc(a.id)}" data-root="${esc(r.root)}">${ico("play", "sm")} Run</button></div>
        </div>`).join("")
      )).join("");
    return out;
  }

  function vActivity() {
    rows = [];
    const h = (D.history || []).filter((e) => hit(e.repo) || hit(e.runner) || hit(e.agent));
    if (!h.length) {
      return section("noact", "Activity",
        `<div class="blank sm">
          <p>No agent sessions recorded yet.</p>
          <p class="dim">Runs launched from Principia are logged to <code>~/.principia/history/</code>. Claude Code also logs automatically once the Principia plugin is installed.</p>
        </div>`);
    }
    return section("act", "Recent agent activity",
      h.slice(0, 60).map((e) => `<div class="row">
        <span class="ic">${ico(e.phase === "launch" ? "rocket" : e.phase === "start" ? "debug-start" : "debug-stop")}</span>
        <div class="body">
          <div class="line1"><b>${esc(e.runner || "?")}</b>${e.agent ? `<span class="pill">${esc(e.agent)}</span>` : ""}<span class="pill">${esc(e.phase)}</span></div>
          <div class="meta"><span class="mono">${esc((e.repo || "").split("/").pop())}</span>${e.branch ? `<span class="mono">${esc(e.branch)}</span>` : ""}<span class="dim">${esc(String(e.ts || "").replace("T", " ").replace("Z", ""))}</span></div>
        </div>
      </div>`).join(""));
  }

  /* ───────── interaction ───────── */

  function wire() {
    const qi = document.getElementById("q");
    let t = null;
    qi.addEventListener("input", () => {
      clearTimeout(t);
      t = setTimeout(() => { q = qi.value.trim().toLowerCase(); focus = -1; persist(); paintBody(false); }, 70);
    });

    document.getElementById("nav").addEventListener("click", (ev) => {
      const b = ev.target.closest("[data-view]"); if (b) setView(b.dataset.view);
    });

    app.addEventListener("click", (ev) => {
      const el = ev.target.closest("[data-act],[data-open],[data-setup],[data-flow],[data-agent]");
      if (!el || el.disabled) return;
      ev.stopPropagation();
      const d = el.dataset;
      if (d.act === "refresh") { spin(); return send({ type: "refresh" }); }
      if (d.act) return send({ type: d.act === "redetect" ? "redetect" : d.act });
      if (d.setup) return send({ type: "setup", root: d.setup });
      if (d.flow) return send({ type: "flow", root: d.root, id: d.flow });
      if (d.agent && d.root) return send({ type: "agent", root: d.root, id: d.agent });
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
    if (!inInput && ev.key >= "1" && ev.key <= "4") return setView(VIEWS[+ev.key - 1][0]);
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
      D = D || { repos: [], groups: [], runners: [], board: [], history: [], counts: {} };
      D.error = m.message; render(false);
    }
  });

  render(true);
  send({ type: "ready" });
})();
