#!/usr/bin/env node
// Render a studio page to still images or to the frames of a video.
//
// The marketing assets (post images, the launch video, the README loop) are
// web pages under assets/studio/. A page draws itself as a pure function of
// time: it exposes `window.seek(ms)`, which puts every element where it
// belongs at that instant, and `window.ready`, a promise that resolves once
// its fonts and captures have loaded. This script opens the page in the
// Chrome already on the machine, calls `seek` for each frame and saves what
// the page looks like. Because nothing runs in real time, a frame is the same
// on every run and on every machine, however slow.
//
// No dependencies: Chrome is driven over its DevTools protocol with Node's
// built-in WebSocket, and the page is served by a few lines of `node:http`.
//
//   scripts/studio/render.mjs --page assets/studio/post.html?card=github \
//       --size 1280x640 --still target/assets/social/github.png
//
//   scripts/studio/render.mjs --page assets/studio/film.html \
//       --size 1920x1080 --fps 30 --frames target/assets/frames/film
//
// Options:
//   --page PATH[?query]   page to open, relative to the repository root
//   --url URL             or any address (a running `cohors web`, say)
//   --wait MS             let a page that is not a studio page settle first
//   --size WxH            the page's size in CSS pixels
//   --scale N             device pixels per CSS pixel (default 1; 2 for stills)
//   --still FILE          save one image (at --at ms, default 0)
//   --frames DIR          save every frame as DIR/000001.png ...
//   --fps N               frames per second (default 30)
//   --from MS --to MS     the stretch of the page's timeline to render
//                         (default: all of it, from the page's `duration`)

import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

const CHROME = [
  process.env.CHROME,
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/Applications/Chromium.app/Contents/MacOS/Chromium",
  "/usr/bin/google-chrome",
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
].find((candidate) => candidate && existsSync(candidate));

function options() {
  const out = { scale: 1, fps: 30, at: 0 };
  const args = process.argv.slice(2);
  for (let i = 0; i < args.length; i += 2) {
    const key = args[i].replace(/^--/, "");
    out[key] = args[i + 1];
  }
  if ((!out.page && !out.url) || !out.size || (!out.still && !out.frames)) {
    console.error("usage: render.mjs --page PATH --size WxH (--still FILE | --frames DIR) [--scale N] [--fps N] [--from MS] [--to MS] [--at MS]");
    process.exit(2);
  }
  const [width, height] = String(out.size).split("x").map(Number);
  return { ...out, width, height, scale: Number(out.scale), fps: Number(out.fps), at: Number(out.at) };
}

const TYPES = {
  ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8", ".json": "application/json", ".svg": "image/svg+xml", ".png": "image/png",
  ".jpg": "image/jpeg", ".ttf": "font/ttf", ".woff2": "font/woff2", ".txt": "text/plain; charset=utf-8",
};

// Serve the repository to the page, and nothing outside it.
function serve() {
  const server = createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, "http://x").pathname);
      const file = path.resolve(ROOT, "." + pathname);
      if (file !== ROOT && !file.startsWith(ROOT + path.sep)) throw new Error("outside the repository");
      const body = await readFile(file);
      response.writeHead(200, { "content-type": TYPES[path.extname(file)] || "application/octet-stream", "cache-control": "no-store" });
      response.end(body);
    } catch {
      response.writeHead(404).end("not found");
    }
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(server)));
}

// Start Chrome with no window and find where its DevTools endpoint is.
async function launch(profile, width, height) {
  const chrome = spawn(CHROME, [
    "--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`,
    "--hide-scrollbars", "--mute-audio", "--no-first-run", "--no-default-browser-check",
    "--disable-background-timer-throttling", "--disable-renderer-backgrounding",
    `--window-size=${width},${height}`, "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  const endpoint = await new Promise((resolve, reject) => {
    let seen = "";
    const timer = setTimeout(() => reject(new Error("Chrome did not start")), 20000);
    chrome.stderr.on("data", (chunk) => {
      seen += chunk;
      const found = /DevTools listening on (ws:\/\/\S+)/.exec(seen);
      if (found) { clearTimeout(timer); resolve(found[1]); }
    });
    chrome.on("exit", () => reject(new Error("Chrome exited before it was ready")));
  });
  return { chrome, endpoint };
}

// One page's DevTools session: `send` a command, wait for its answer.
async function session(endpoint) {
  const port = new URL(endpoint).port;
  const target = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: "PUT" })).json();
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
  let next = 0;
  const waiting = new Map();
  const listeners = new Map();
  socket.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.id && waiting.has(message.id)) {
      const { resolve, reject } = waiting.get(message.id);
      waiting.delete(message.id);
      message.error ? reject(new Error(message.error.message)) : resolve(message.result);
    } else if (message.method && listeners.has(message.method)) {
      listeners.get(message.method)(message.params);
      listeners.delete(message.method);
    }
  };
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++next;
    waiting.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params }));
  });
  const once = (method) => new Promise((resolve) => listeners.set(method, resolve));
  // Run an expression in the page and wait for it, surfacing a page error
  // as an error here rather than as a blank frame.
  const evaluate = async (expression) => {
    const result = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) {
      throw new Error(`the page threw: ${result.exceptionDetails.exception?.description || result.exceptionDetails.text}`);
    }
    return result.result.value;
  };
  return { send, once, evaluate, close: () => socket.close() };
}

async function main() {
  const o = options();
  if (!CHROME) throw new Error("no Chrome or Chromium found; set CHROME to its path");
  const server = await serve();
  const profile = await mkdtemp(path.join(tmpdir(), "cohors-studio-"));
  const { chrome, endpoint } = await launch(profile, o.width, o.height);
  try {
    const page = await session(endpoint);
    await page.send("Page.enable");
    await page.send("Emulation.setDeviceMetricsOverride", { width: o.width, height: o.height, deviceScaleFactor: o.scale, mobile: false });
    const loaded = page.once("Page.loadEventFired");
    await page.send("Page.navigate", { url: o.url || `http://127.0.0.1:${server.address().port}/${o.page}` });
    await loaded;
    // A studio page says when it is ready and draws itself at a given time.
    // Any other page is simply given a moment and photographed as it stands.
    await page.evaluate("window.ready");
    if (o.wait) await new Promise((resolve) => setTimeout(resolve, Number(o.wait)));
    const seek = (ms) => page.evaluate(`window.seek ? window.seek(${ms}) : undefined`);
    const shot = async () => Buffer.from((await page.send("Page.captureScreenshot", { format: "png" })).data, "base64");

    if (o.still) {
      await seek(o.at);
      await mkdir(path.dirname(path.resolve(ROOT, o.still)), { recursive: true });
      await writeFile(path.resolve(ROOT, o.still), await shot());
      console.log(`${o.still}  (${o.width * o.scale}x${o.height * o.scale})`);
      return;
    }

    const duration = await page.evaluate("window.duration || 0");
    const from = Number(o.from ?? 0), to = Number(o.to ?? duration);
    const count = Math.max(1, Math.round(((to - from) / 1000) * o.fps));
    const dir = path.resolve(ROOT, o.frames);
    await rm(dir, { recursive: true, force: true });
    await mkdir(dir, { recursive: true });
    for (let i = 0; i < count; i++) {
      await seek(from + (i * 1000) / o.fps);
      await writeFile(path.join(dir, `${String(i + 1).padStart(6, "0")}.png`), await shot());
      if (i % 30 === 0) process.stdout.write(`\r${o.frames}: frame ${i + 1} of ${count}`);
    }
    console.log(`\r${o.frames}: ${count} frames at ${o.fps} fps (${((to - from) / 1000).toFixed(1)} s)`);
  } finally {
    chrome.kill();
    server.close();
    await rm(profile, { recursive: true, force: true }).catch(() => {});
  }
}

main().catch((error) => { console.error(error.message); process.exit(1); });
