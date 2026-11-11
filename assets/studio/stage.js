// The stage: what every studio page (the film, the post images) is built from.
//
// A page is a pure function of time. It lays its pieces out once, then
// `seek(ms)` computes where each piece is at that instant and writes it to the
// DOM. Nothing here uses timers, transitions or animations, so a frame looks
// the same whenever and however slowly it is rendered (scripts/studio/
// render.mjs calls `seek` once per frame and photographs the result).
//
// The pieces:
//   capture(name)      a real screen of cohors, as the HTML that
//                      scripts/studio/capture.sh saved. It is text, not a
//                      picture, so a camera can push into it and stay sharp.
//   terminal(names)    a window holding several captures, one showing.
//   picture(src)       a window holding a picture of another surface.
//   camera             pan and zoom over the "world" the windows sit in.
//   track / ease       values that change over time.
//   mark()             the cohors mark, as cells that can be animated.

window.Stage = (() => {
  const CAPTURES = "/target/assets/captures/";
  const clamp = (v, lo = 0, hi = 1) => Math.min(hi, Math.max(lo, v));
  const lerp = (a, b, t) => a + (b - a) * t;

  // Easing: slow out of a hold, slow into the next one. Camera moves that
  // start and stop abruptly are what makes a screen recording feel cheap.
  const ease = {
    linear: (t) => t,
    inOut: (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2),
    out: (t) => 1 - Math.pow(1 - t, 3),
    in: (t) => t * t * t,
  };

  // How far through [start, end] `t` is, eased, held at 0 before and 1 after.
  const span = (t, start, end, curve = ease.inOut) => curve(clamp((t - start) / Math.max(1, end - start)));

  // A value over time from keyframes [[ms, value], ...]. Each value may be a
  // number or an object of numbers; a keyframe's third item is the curve
  // used to arrive at it.
  function track(keys) {
    return (t) => {
      if (t <= keys[0][0]) return keys[0][1];
      for (let i = 1; i < keys.length; i++) {
        const [t1, v1, curve] = keys[i];
        if (t <= t1) {
          const [t0, v0] = keys[i - 1];
          const k = (curve || ease.inOut)(clamp((t - t0) / Math.max(1, t1 - t0)));
          if (typeof v1 === "number") return lerp(v0, v1, k);
          const out = {};
          for (const name of Object.keys(v1)) out[name] = lerp(v0[name], v1[name], k);
          return out;
        }
      }
      return keys[keys.length - 1][1];
    };
  }

  // How visible something shown from `on` to `off` is, fading over `fade` ms.
  const during = (t, on, off, fade = 350) =>
    Math.min(span(t, on, on + fade, ease.out), 1 - span(t, off - fade, off, ease.in));

  /* ───────── captures ───────── */

  let styled = false;
  async function capture(name) {
    const text = await (await fetch(`${CAPTURES}${name}.html`)).text();
    const doc = new DOMParser().parseFromString(text, "text/html");
    if (!styled) {
      // The capture's own stylesheet (cell size, the painted block cells).
      const style = document.createElement("style");
      style.textContent = doc.querySelector("style").textContent;
      document.head.appendChild(style);
      styled = true;
    }
    const node = doc.querySelector(".t");
    node.dataset.capture = name;
    return document.importNode(node, true);
  }

  // A terminal window holding the named captures stacked on one another.
  // `win.show(name, amount)` sets how visible each is.
  async function terminal(names, title = "cohors") {
    const win = document.createElement("div");
    win.className = "win";
    win.innerHTML = `<div class="winbar"><i></i><i></i><i></i><span>${title}</span></div><div class="winbody"></div>`;
    const body = win.querySelector(".winbody");
    const layers = {};
    for (const name of names) {
      const layer = await capture(name);
      layer.classList.add("layer");
      body.appendChild(layer);
      layers[name] = layer;
    }
    win.layers = layers;
    win.show = (visible) => {
      for (const [name, layer] of Object.entries(layers)) {
        const amount = visible[name] || 0;
        layer.style.opacity = amount;
        layer.style.visibility = amount > 0.001 ? "visible" : "hidden";
      }
    };
    return win;
  }

  // A window holding a picture of another surface (the editor extension, the
  // browser dashboard), `width` wide. `win.loaded` resolves once the picture
  // has arrived, so a frame is never photographed without it.
  function picture(src, title, width) {
    const win = document.createElement("div");
    win.className = "win";
    win.style.width = `${width}px`;
    win.innerHTML = `<div class="winbar"><i></i><i></i><i></i><span>${title}</span></div><img class="winpic" alt="">`;
    const img = win.querySelector("img");
    win.loaded = new Promise((resolve, reject) => {
      img.onload = resolve;
      img.onerror = () => reject(new Error(`${src} is missing: run scripts/studio/build.sh posts first`));
    });
    img.src = src;
    return win;
  }

  // Where a piece of a capture is, in the window's own coordinates. Call it
  // before any transform is applied (at load), and keep the answer.
  //   find(win, "overview", "Do next")                 the whole row
  //   find(win, "overview", "Do next", { rows: 6 })    that row and five below
  function find(win, name, text, { rows = 1, pad = 0 } = {}) {
    const layer = win.layers[name];
    const lines = [...layer.querySelectorAll(".r")];
    const index = lines.findIndex((line) => line.textContent.includes(text));
    if (index < 0) throw new Error(`"${text}" is not on the ${name} capture`);
    const origin = win.getBoundingClientRect();
    const first = lines[index].getBoundingClientRect();
    const last = lines[Math.min(lines.length - 1, index + rows - 1)].getBoundingClientRect();
    const frame = layer.getBoundingClientRect();
    return {
      x: frame.left - origin.left - pad,
      y: first.top - origin.top - pad,
      w: frame.width + pad * 2,
      h: last.bottom - first.top + pad * 2,
    };
  }

  /* ───────── camera ───────── */

  // The camera looks at a point of the world at a zoom. `frame(rect, fill)`
  // gives the shot that fits `rect` into `fill` of the stage.
  function camera(world, width, height) {
    return {
      set({ x, y, zoom }) {
        world.style.transform = `translate(${width / 2 - x * zoom}px, ${height / 2 - y * zoom}px) scale(${zoom})`;
      },
      frame(rect, fill = 0.8, maxZoom = 3) {
        const zoom = Math.min(maxZoom, (width * fill) / rect.w, (height * fill) / rect.h);
        return { x: rect.x + rect.w / 2, y: rect.y + rect.h / 2, zoom };
      },
    };
  }

  /* ───────── the mark ───────── */

  // The cohors mark as a grid of cells, each a <rect> that can be moved and
  // faded on its own. Three rows of block characters, each character a 2x2
  // grid of quarters; the shaded block is the same cell, fainter. (The same
  // rows draw the mark in the terminal and in the editor extension.)
  const MARK_ROWS = ["▜▒▟███▙▒▛", "▟██▌█▐██▙", "▀▐▖▀█▀▗▌▀"];
  const QUARTERS = {
    "█": [1, 1, 1, 1], "▒": [1, 1, 1, 1], "▀": [1, 1, 0, 0], "▄": [0, 0, 1, 1],
    "▌": [1, 0, 1, 0], "▐": [0, 1, 0, 1], "▜": [1, 1, 0, 1], "▛": [1, 1, 1, 0],
    "▟": [0, 1, 1, 1], "▙": [1, 0, 1, 1], "▖": [0, 0, 1, 0], "▗": [0, 0, 0, 1],
  };
  function mark() {
    const cells = [];
    MARK_ROWS.forEach((row, y) => [...row].forEach((ch, x) => {
      (QUARTERS[ch] || []).forEach((on, i) => {
        if (on) cells.push({ x: x * 2 + (i % 2), y: y * 4 + (i >> 1) * 2, faint: ch === "▒" });
      });
    }));
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", "0 0 18 12");
    // Cells meet edge to edge; without crisp edges each seam is drawn as a
    // faint line where two anti-aliased edges overlap.
    svg.setAttribute("shape-rendering", "crispEdges");
    svg.classList.add("mark");
    svg.cells = cells.map((cell) => {
      const rect = document.createElementNS("http://www.w3.org/2000/svg", "rect");
      rect.setAttribute("x", cell.x);
      rect.setAttribute("y", cell.y);
      rect.setAttribute("width", 1);
      rect.setAttribute("height", 2);
      // The shaded block is the same cell, fainter.
      if (cell.faint) rect.style.opacity = 0.45;
      svg.appendChild(rect);
      return { ...cell, rect, base: cell.faint ? 0.45 : 1 };
    });
    return svg;
  }

  const fonts = () => document.fonts.ready;

  return { clamp, lerp, ease, span, track, during, capture, terminal, picture, find, camera, mark, fonts };
})();
