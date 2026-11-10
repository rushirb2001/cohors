// Simulators, emulators and local dev servers. All optional, all best-effort,
// and only probed while the Devices page is on screen.
//
// cohors does not know about these, so the extension asks the platform tools
// directly, always with an argument array.

const cp = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const exists = (p) => { try { fs.accessSync(p); return true; } catch { return false; } };

function run(cmd, args, opts = {}) {
  return new Promise((resolve) => cp.execFile(cmd, args,
    { timeout: opts.timeout || 4000, encoding: "utf8", maxBuffer: 1 << 20, windowsHide: true },
    (err, stdout) => resolve(err ? "" : String(stdout))));
}

// The SDK location is whatever the user's environment says, then the platform
// defaults Android Studio installs to.
function androidSdk() {
  const home = os.homedir();
  const candidates = [
    process.env.ANDROID_HOME,
    process.env.ANDROID_SDK_ROOT,
    path.join(home, "Library", "Android", "sdk"),
    path.join(home, "Android", "Sdk"),
    path.join(home, "AppData", "Local", "Android", "Sdk"),
  ].filter(Boolean);
  return candidates.find((d) => exists(path.join(d, "emulator"))) || null;
}

function androidTools() {
  const sdk = androidSdk();
  if (!sdk) return null;
  const exe = process.platform === "win32" ? ".exe" : "";
  return {
    adb: path.join(sdk, "platform-tools", `adb${exe}`),
    emulator: path.join(sdk, "emulator", `emulator${exe}`),
  };
}

async function listeningPorts() {
  if (process.platform === "win32") return [];
  const out = await run("lsof", ["-iTCP", "-sTCP:LISTEN", "-P", "-n", "-Fpcn"]);
  if (!out) return [];
  const found = [];
  let pid = null, cmd = null;
  for (const line of out.split("\n")) {
    if (line[0] === "p") pid = parseInt(line.slice(1), 10);
    else if (line[0] === "c") cmd = line.slice(1);
    else if (line[0] === "n") {
      const m = line.match(/:(\d+)$/);
      if (m) found.push({ port: parseInt(m[1], 10), pid, cmd });
    }
  }
  const seen = new Map();
  for (const r of found) if (r.port >= 1024 && !seen.has(r.port)) seen.set(r.port, r);
  return [...seen.values()].sort((a, b) => a.port - b.port).slice(0, 40);
}

async function androidState() {
  const tools = androidTools();
  if (!tools) return { available: false, reason: "The Android SDK was not found.", booted: false, device: null, avds: [] };
  const [dev, avd] = await Promise.all([run(tools.adb, ["devices"]), run(tools.emulator, ["-list-avds"])]);
  const m = dev.match(/(emulator-\d+)\s+device/);
  return {
    available: true, reason: null,
    booted: !!m, device: m ? m[1] : null,
    avds: avd.split("\n").map((x) => x.trim()).filter((x) => x && !/\s/.test(x)),
  };
}

async function iosState() {
  if (process.platform !== "darwin") return { available: false, reason: "iOS simulators need macOS.", booted: [], all: [] };
  const out = await run("xcrun", ["simctl", "list", "devices", "available", "-j"], { timeout: 6000 });
  if (!out) return { available: false, reason: "The Xcode command line tools were not found.", booted: [], all: [] };
  let j;
  try { j = JSON.parse(out); } catch { return { available: false, reason: "simctl printed something unreadable.", booted: [], all: [] }; }
  const all = [];
  for (const [runtime, devs] of Object.entries(j.devices || {})) {
    if (!/iOS/.test(runtime)) continue;
    for (const d of devs) all.push({ name: d.name, udid: d.udid, state: d.state });
  }
  all.sort((a, b) => ((b.state === "Booted") - (a.state === "Booted")) || a.name.localeCompare(b.name));
  return { available: true, reason: null, booted: all.filter((d) => d.state === "Booted"), all: all.slice(0, 30) };
}

// Each probe shells out, and none of this changes on a one-second timescale.
let cache = { at: 0, value: null };
const TTL_MS = 10000;

async function state(force) {
  if (!force && cache.value && Date.now() - cache.at < TTL_MS) return cache.value;
  const [ports, android, ios] = await Promise.all([listeningPorts(), androidState(), iosState()]);
  cache = { at: Date.now(), value: { ports, android, ios, at: Math.floor(Date.now() / 1000) } };
  return cache.value;
}

// Headless on purpose: the point is to use it from the editor, not to open a
// second native window. Memory is capped for laptops.
async function bootAndroid() {
  const tools = androidTools();
  const st = await androidState();
  if (!tools || !st.available) return { ok: false, message: st.reason || "The Android SDK was not found." };
  if (st.booted) return { ok: true, message: `${st.device} is already running.` };
  if (!st.avds.length) return { ok: false, message: "No Android virtual devices are configured." };
  cp.spawn(tools.emulator,
    ["-avd", st.avds[0], "-no-window", "-no-audio", "-no-boot-anim", "-memory", "2048", "-no-snapshot-save"],
    { detached: true, stdio: "ignore" }).unref();
  return { ok: true, message: `Booting ${st.avds[0]} without a window.` };
}

async function stopAndroid() {
  const tools = androidTools();
  if (tools) await run(tools.adb, ["emu", "kill"]);
}

const UDID_RE = /^[0-9A-Fa-f-]{8,64}$/;

async function ios(action, udid) {
  if (!UDID_RE.test(String(udid || ""))) return;
  if (action === "boot") {
    await run("xcrun", ["simctl", "boot", udid], { timeout: 20000 });
    await run("open", ["-a", "Simulator"]);
  }
  if (action === "shutdown") await run("xcrun", ["simctl", "shutdown", udid], { timeout: 20000 });
}

module.exports = { state, bootAndroid, stopAndroid, ios };
