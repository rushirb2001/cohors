// One pane of the activity-bar container (see `sidePart` in view.js for what
// each shows). The editor draws the panes and their headers; this draws what
// is inside one.
//
// It is deliberately small. It draws what view.js returns, remembers what is
// typed in the filter, and turns a click into a message for the extension
// host. It never runs anything itself: a row opens the dashboard at that
// place, and the dashboard is where actions live.

(function () {
  const vscode = acquireVsCodeApi();
  const V = window.CohorsView;
  const app = document.getElementById("app");
  // Which pane this is. The dev preview has no editor to draw pane headers,
  // so it may name several ("fleet,next,risk") and gets a plain heading
  // above each.
  const parts = String(document.body.dataset.part || "fleet").split(",").filter((p) => V.SIDE_PARTS.includes(p));
  const TITLES = { fleet: "Overview", next: "Do next", risk: "Work at risk", tasks: "Open tasks", commits: "Commits, 7 days", repos: "Repositories" };

  const saved = vscode.getState() || {};
  const ui = { q: typeof saved.q === "string" ? saved.q : "" };
  let state = null;

  const persist = () => vscode.setState({ q: ui.q });

  function render() {
    // Redrawing replaces the filter box, so put the cursor back where it was.
    const box = document.activeElement && document.activeElement.id === "side-q" ? document.activeElement : null;
    const caret = box ? box.selectionStart : 0;
    const scroll = document.scrollingElement ? document.scrollingElement.scrollTop : 0;
    const counts = V.sideCounts(state);
    app.innerHTML = parts.map((part) =>
      (parts.length > 1 ? `<h2 class="panehead">${TITLES[part]}${counts && counts[part] != null ? `<span>${counts[part]}</span>` : ""}</h2>` : "")
      + V.sidePart(part, state, ui).map((s) => `<div class="ssec" data-sec="${s.id}">${s.html}</div>`).join("")).join("");
    if (document.scrollingElement) document.scrollingElement.scrollTop = scroll;
    if (box) {
      const again = document.getElementById("side-q");
      if (again) { again.focus(); again.setSelectionRange(caret, caret); }
    }
  }

  const send = (m) => vscode.postMessage(m);

  app.addEventListener("click", (ev) => {
    const el = ev.target.closest("[data-do]");
    if (!el || el.disabled) return;
    const d = el.dataset;
    switch (d.do) {
      // Every row is a way into the dashboard.
      case "show": return send({ type: "show", page: d.page || null, repo: d.repo || null });
      case "refresh": return send({ type: "refresh" });
      // What the "cohors is missing" and "no repositories" pages offer.
      case "redetect": return send({ type: "redetect" });
      case "settings": return send({ type: "settings" });
      case "install-help": return send({ type: "installHelp" });
      case "workspace-roots": return send({ type: "useWorkspaceRoots" });
      case "open-folder": return send({ type: "openFolder" });
      case "copy": return send({ type: "copy", text: d.text });
    }
  });

  app.addEventListener("input", (ev) => {
    if (ev.target.id !== "side-q") return;
    ui.q = ev.target.value;
    persist();
    render();
  });

  window.addEventListener("message", (ev) => {
    const m = ev.data || {};
    if (m.type === "data") { state = m.state || null; render(); }
    if (m.type === "busy") app.classList.toggle("busy", !!m.on);
  });

  render();
  send({ type: "ready" });
})();
