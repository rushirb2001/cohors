// cohors dashboard: the DOM half of the webview.
//
// view.js turns data into HTML strings; this file puts them on screen and
// turns clicks into messages for the extension host.
//
// The rule this file exists to keep: the page is never rebuilt wholesale on a
// refresh. The shell is painted once. Each section is replaced only when its
// HTML actually changed, and state that lives in the DOM (the filter's caret,
// ticked checkboxes, the highlighted row, the chosen runner) is synchronised
// separately instead of being baked into the HTML. So a background refresh
// leaves scroll position, keyboard focus and whatever is being typed alone.

(function () {
  "use strict";
  const vscode = acquireVsCodeApi();
  const V = window.CohorsView;
  const app = document.getElementById("app");
  const saved = vscode.getState() || {};

  let state = saved.state || null;
  const ui = Object.assign(
    { page: "overview", q: "", view: "", pick: null, sel: null, runner: "", standupWindow: "week", showDone: false },
    saved.ui || {},
    { standup: {}, devices: null },   // fetched on demand, never persisted
  );
  if (!V.PAGES.some((p) => p.id === ui.page)) ui.page = "overview";
  const selected = new Set(saved.selected || []);
  let busy = false;

  const send = (m) => vscode.postMessage(m);
  function persist() {
    vscode.setState({
      state,
      ui: { page: ui.page, q: ui.q, view: ui.view, pick: ui.pick, sel: ui.sel, runner: ui.runner, standupWindow: ui.standupWindow, showDone: ui.showDone },
      selected: [...selected],
    });
  }
  const ok = () => !!state && state.status === "ok";
  const repos = () => (ok() ? state.overview.repos : []);
  const $ = (id) => document.getElementById(id);

  /* ───────── painting: replace only what changed ───────── */

  const painted = new Map();   // container id -> Map(section id -> html)
  const focusables = (n) => [...n.querySelectorAll("button, input, select, a[href]")].filter((e) => !e.disabled);

  // Replace one section's contents, keeping keyboard focus on the control in
  // the same position if focus was inside it.
  function swap(node, html) {
    const active = document.activeElement;
    const index = active && node.contains(active) ? focusables(node).indexOf(active) : -1;
    node.innerHTML = html;
    if (index >= 0) {
      const f = focusables(node);
      const target = f[Math.min(index, f.length - 1)];
      if (target) target.focus({ preventScroll: true });
    }
  }

  function paint(container, sections) {
    let cache = painted.get(container.id);
    if (!cache) { cache = new Map(); painted.set(container.id, cache); }
    const ids = sections.map((s) => s.id);
    const live = [...container.children].map((n) => n.dataset.sec);

    if (ids.length === live.length && ids.every((id, i) => id === live[i])) {
      // The usual case, a refresh: same sections, patch the changed ones.
      sections.forEach((s, i) => {
        if (cache.get(s.id) === s.html) return;
        swap(container.children[i], s.html);
        cache.set(s.id, s.html);
      });
      return;
    }

    // The set of sections changed: a navigation, or the fleet changed shape.
    // Sections that still apply keep their nodes; the rest are built.
    const keep = new Map([...container.children].map((n) => [n.dataset.sec, n]));
    const next = sections.map((s) => {
      let node = keep.get(s.id);
      if (!node) {
        node = document.createElement("section");
        node.className = "sec";
        node.dataset.sec = s.id;
        node.innerHTML = s.html;
      } else if (cache.get(s.id) !== s.html) {
        swap(node, s.html);
      }
      cache.set(s.id, s.html);
      return node;
    });
    for (const id of [...cache.keys()]) if (!ids.includes(id)) cache.delete(id);
    const active = document.activeElement;
    container.replaceChildren(...next);
    if (active && active !== document.activeElement && container.contains(active)) active.focus({ preventScroll: true });
  }

  let built = false;
  function render() {
    if (!built) {
      app.innerHTML = V.shell();
      built = true;
      wire();
    }
    paint($("rail"), V.rail(state, ui));
    // Section ids are scoped to the page, so two pages never share a node.
    const scope = ok() ? ui.page : "status";
    paint($("main"), V.page(state, ui).map((s) => ({ id: `${scope}:${s.id}`, html: s.html })));
    const d = ok() ? V.detail(state, ui) : [];
    $("detail").hidden = !d.length;
    app.classList.toggle("has-detail", d.length > 0);
    paint($("detail"), d);
    sync();
  }

  // State that lives in the DOM rather than in the HTML strings.
  function sync() {
    const main = $("main");
    main.querySelectorAll("input[data-select]").forEach((c) => { c.checked = selected.has(c.dataset.select); });
    main.querySelectorAll("tr[data-repo]").forEach((r) => r.classList.toggle("sel", r.dataset.repo === ui.sel));

    const n = selected.size;
    const count = $("selcount");
    if (count) count.textContent = n ? `${n} selected` : "None selected";
    main.querySelectorAll('[data-do="bulk"]').forEach((b) => { b.disabled = !n; });
    main.querySelectorAll('[data-do="act"]').forEach((b) => {
      const label = b.querySelector("span:not(.codicon)");
      if (label) label.textContent = n ? `Act on ${n} selected` : "Act on selected";
    });

    const q = $("q");
    if (q && document.activeElement !== q && q.value !== ui.q) q.value = ui.q;

    const runner = $("runner");
    if (runner) {
      const usable = [...runner.options].some((o) => o.value === ui.runner && !o.disabled);
      if (!usable) ui.runner = "";
      if (runner.value !== ui.runner) runner.value = ui.runner;
    }

    const spin = document.querySelector('#rail [data-do="refresh"] .codicon');
    if (spin) spin.classList.toggle("spin", busy);
  }

  /* ───────── navigation ───────── */

  function requestStandup() {
    const w = ui.standupWindow === "today" ? "today" : "week";
    send({ type: "standup", window: w });
  }

  function go(page, keepScroll) {
    ui.page = page;
    if (page !== "repos") ui.pick = null;
    if (ok() && page === "standup") requestStandup();
    if (ok() && page === "devices") send({ type: "devices" });
    persist();
    render();
    if (!keepScroll) $("main").scrollTop = 0;
  }

  function openDetail(id, viaKeyboard) {
    ui.sel = id;
    persist();
    render();
    $("detail").scrollTop = 0;
    if (viaKeyboard) {
      const close = $("detail").querySelector('[data-do="close-detail"]');
      if (close) close.focus();
    }
  }

  function closeDetail() {
    const id = ui.sel;
    const hadFocus = $("detail").contains(document.activeElement);
    ui.sel = null;
    persist();
    render();
    if (hadFocus && id) {
      const back = [...$("main").querySelectorAll('[data-do="detail"]')].find((b) => b.dataset.id === id);
      if (back) back.focus();
    }
  }

  // "Do next": run the verb cohors named, or go to where the work is.
  function doNext(i) {
    const a = ok() && state.overview.actions[i];
    if (!a) return;
    if (a.verb) return send({ type: "verb", verb: a.verb, ids: a.repos });
    if (a.kind === "start_task") return go("tasks");
    if (a.repos.length === 1) { go("repos"); return openDetail(a.repos[0]); }
    // Several repositories: show exactly those on the Repositories page.
    ui.view = ""; ui.q = "";
    ui.pick = { label: a.title, ids: a.repos.slice() };
    const q = $("q");
    if (q) q.value = "";
    go("repos");
  }

  function repoAction(id, viaKeyboard) {
    const r = repos().find((x) => x.id === id);
    if (!r) return;
    if (r.action && r.action.verb) return send({ type: "verb", verb: r.action.verb, ids: [id] });
    openDetail(id, viaKeyboard);
  }

  let toastTimer = null;
  function toast(text) {
    const t = $("toast");
    t.textContent = text;
    t.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { t.hidden = true; }, 2600);
  }

  /* ───────── actions: one handler for every control ───────── */

  function act(d, viaKeyboard) {
    const runner = ui.runner || undefined;
    switch (d.do) {
      case "refresh": return send({ type: "refresh" });
      case "redetect": return send({ type: "redetect" });
      case "page": return go(d.page);
      case "view":
        ui.view = d.view || ""; ui.pick = null;
        return go("repos", ui.page === "repos");
      case "clear-pick": ui.pick = null; persist(); return render();
      case "clear-filter": {
        ui.q = ""; ui.view = ""; ui.pick = null;
        const q = $("q"); if (q) q.value = "";
        persist(); return render();
      }
      case "detail": return ui.sel === d.id ? closeDetail() : openDetail(d.id, viaKeyboard);
      case "close-detail": return closeDetail();
      case "next": return doNext(Number(d.i));
      case "repo-action": return repoAction(d.id, viaKeyboard);

      case "fetch-all": return send({ type: "verb", verb: "fetch", all: true });
      case "act": {
        if (selected.size) return send({ type: "act", ids: [...selected] });
        // Nothing ticked yet: show where to tick instead of a dead button.
        const table = $("main").querySelector("table.repos");
        if (table) table.scrollIntoView({ block: "start", behavior: "smooth" });
        return toast("Tick the repositories to act on in the table, then choose Act on selected.");
      }
      case "bulk": return send({ type: "verb", verb: d.verb, ids: [...selected] });
      case "bulk-one": return send({ type: "verb", verb: d.verb, ids: [d.id] });
      case "select-shown":
        V.filterRepos(V.model(state), ui).forEach((r) => selected.add(r.id));
        persist(); return sync();
      case "select-none": selected.clear(); persist(); return sync();

      case "open": return send({ type: "open", id: d.id, newWindow: d.new === "1" });
      case "terminal": return send({ type: "terminal", id: d.id });
      case "start": return send({ type: "start", id: d.id, runner });
      case "flow": return send({ type: "flow", id: d.id, flow: d.flow });
      case "link": return send({ type: "link", id: d.id, url: d.url });
      case "agent": return send({ type: "agent", id: d.id, agent: d.agent, runner });
      case "agent-file": return send({ type: "openAgentFile", id: d.id, agent: d.agent });
      case "task": return send({ type: "task", id: d.id, task: d.task, runner });
      case "focus": return send({ type: "focus", item: d.item, runner });
      case "resume": return send({ type: "resume", id: d.id, session: d.session, runner });
      case "prompt": return send({ type: "prompt", kind: d.kind, id: d.id, runner });
      case "setup-here": return send({ type: "setupHere", kind: d.kind, runner });
      case "toggle-done": ui.showDone = !ui.showDone; persist(); return render();

      case "standup-window":
        ui.standupWindow = d.window === "today" ? "today" : "week";
        persist(); requestStandup(); return render();
      case "copy-standup": return send({ type: "copyStandup", window: ui.standupWindow });

      case "devices": return send({ type: "devices", force: true });
      case "android": return send({ type: "android", action: d.action });
      case "ios": return send({ type: "ios", action: d.action, udid: d.udid });
      case "port": return send({ type: "port", port: Number(d.port), external: d.external === "1" });

      case "copy": return send({ type: "copy", text: d.text });
      case "settings": return send({ type: "settings" });
      case "workspace-roots": return send({ type: "useWorkspaceRoots" });
      case "install-help": return send({ type: "installHelp" });
      case "open-folder": return send({ type: "openFolder" });
      default: return undefined;
    }
  }

  function wire() {
    app.addEventListener("click", (ev) => {
      const el = ev.target.closest("[data-do]");
      if (el) {
        if (el.disabled) return;
        // A click with no pointer detail came from the keyboard.
        return void act(el.dataset, ev.detail === 0);
      }
      // Anywhere else on a repository row opens its detail.
      if (ev.target.closest("input, button, a, select, label")) return;
      const row = ev.target.closest("tr[data-repo]");
      if (row) act({ do: "detail", id: row.dataset.repo }, false);
    });

    app.addEventListener("change", (ev) => {
      const t = ev.target;
      if (t.matches("input[data-select]")) {
        if (t.checked) selected.add(t.dataset.select); else selected.delete(t.dataset.select);
        persist();
        return sync();
      }
      if (t.id === "runner") { ui.runner = t.value; persist(); }
    });

    let typing = null;
    app.addEventListener("input", (ev) => {
      if (ev.target.id !== "q") return;
      clearTimeout(typing);
      typing = setTimeout(() => { ui.q = ev.target.value; persist(); render(); }, 80);
    });
  }

  document.addEventListener("keydown", (ev) => {
    const t = ev.target;
    const inField = t && ((t.tagName === "INPUT" && t.type !== "checkbox") || t.tagName === "TEXTAREA" || t.tagName === "SELECT");
    if (ev.key === "Escape") {
      if (t && t.id === "q") {
        if (ui.q) { ui.q = ""; t.value = ""; persist(); render(); }
        return void t.blur();
      }
      if (ui.sel) return void closeDetail();
      return;
    }
    if (inField || ev.metaKey || ev.ctrlKey || ev.altKey || !built) return;

    if (ev.key === "/") {
      ev.preventDefault();
      if (ui.page !== "repos") go("repos");
      const q = $("q");
      if (q) q.focus();
      return;
    }
    if (ev.key === "r") return void send({ type: "refresh" });
    const n = Number(ev.key);
    if (n >= 1 && n <= V.PAGES.length) return void go(V.PAGES[n - 1].id);

    // Up and down move between repository names in the same table.
    if ((ev.key === "ArrowDown" || ev.key === "ArrowUp") && t && t.matches && t.matches('[data-do="detail"]')) {
      const table = t.closest("table");
      if (!table) return;
      const names = [...table.querySelectorAll('[data-do="detail"]')];
      const next = names[names.indexOf(t) + (ev.key === "ArrowDown" ? 1 : -1)];
      if (next) { ev.preventDefault(); next.focus(); }
    }
  });

  /* ───────── messages from the extension host ───────── */

  window.addEventListener("message", (ev) => {
    const m = ev.data || {};
    switch (m.type) {
      case "data": {
        state = m.state || null;
        const known = new Set(repos().map((r) => r.id));
        for (const id of [...selected]) if (!known.has(id)) selected.delete(id);
        if (ok()) {
          if (ui.sel && !known.has(ui.sel)) ui.sel = null;
          if (ui.pick) {
            ui.pick.ids = ui.pick.ids.filter((id) => known.has(id));
            if (!ui.pick.ids.length) ui.pick = null;
          }
          // The pages that load their own data follow the refresh.
          if (ui.page === "standup") requestStandup();
          if (ui.page === "devices") send({ type: "devices" });
        }
        persist();
        return render();
      }
      case "standup":
        ui.standup[m.window === "today" ? "today" : "week"] = m.error ? { error: m.error } : { data: m.data };
        if (ui.page === "standup") render();
        return;
      case "devices":
        ui.devices = m.data || null;
        if (ui.page === "devices") render();
        return;
      case "busy":
        busy = !!m.on;
        if (built) sync();
        return;
      case "toast":
        if (built) toast(String(m.text || ""));
        return;
      case "reveal":
        if (m.repo) ui.sel = m.repo;
        if (m.page && V.PAGES.some((p) => p.id === m.page)) return go(m.page);
        persist();
        return render();
      case "dev-css":
        document.querySelectorAll('link[rel="stylesheet"]').forEach((l) => {
          if (l.href.includes(m.file)) l.href = l.href.split("?")[0] + "?v=" + Date.now();
        });
        return;
      default:
        return;
    }
  });

  render();
  send({ type: "ready" });
})();
