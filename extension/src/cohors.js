// The cohors CLI adapter: the extension's only source of fleet data.
//
// Everything on screen comes from the `cohors` binary, the same one the
// terminal UI, the web dashboard and the MCP server are built on, so every
// surface shows the same numbers. This module finds the binary, runs it with
// an argument array (never a shell string) and parses what it prints.
//
// It does not require `vscode`, so it can be exercised from plain Node:
//   node dev/check.js --bin target/debug/cohors --root ~/code

const cp = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const HOME = os.homedir();
const WINDOWS = process.platform === "win32";
const BIN_NAME = WINDOWS ? "cohors.exe" : "cohors";

// The registry verbs the CLI exposes as bulk actions.
const VERBS = ["fetch", "pull", "push", "commit", "stash"];

// A failure with a `kind` the UI can turn into the right empty state:
//   missing      no binary found (or the configured path is wrong)
//   failed       the binary ran and exited non-zero
//   unsupported  the binary is too old to know this subcommand
//   parse        it printed something that is not the JSON we expect
//   timeout      it did not finish in time
class CohorsError extends Error {
  constructor(kind, message, extra) {
    super(message);
    this.name = "CohorsError";
    this.kind = kind;
    Object.assign(this, extra || {});
  }
}

function expandHome(p) {
  if (!p) return p;
  if (p === "~") return HOME;
  if (p.startsWith("~/") || p.startsWith("~\\")) return path.join(HOME, p.slice(2));
  return p;
}

function isDirectory(p) {
  try { return fs.statSync(p).isDirectory(); } catch { return false; }
}

function isExecutable(p) {
  try {
    if (!fs.statSync(p).isFile()) return false;
    fs.accessSync(p, WINDOWS ? fs.constants.F_OK : fs.constants.X_OK);
    return true;
  } catch { return false; }
}

// VS Code's process PATH is not the user's login PATH, so a binary installed
// by a shell profile (cargo, brew, pipx) is invisible from here. Ask a login
// shell once. The markers keep whatever a profile prints out of the answer.
let loginPathPromise = null;
function loginPath() {
  if (loginPathPromise) return loginPathPromise;
  loginPathPromise = new Promise((resolve) => {
    if (WINDOWS) return resolve("");
    const shell = process.env.SHELL || "/bin/sh";
    cp.execFile(shell, ["-lic", "printf '__COHORS_PATH__%s__END__' \"$PATH\""],
      { encoding: "utf8", timeout: 5000 },
      (_err, stdout) => {
        const m = /__COHORS_PATH__(.*?)__END__/s.exec(String(stdout || ""));
        resolve(m ? m[1].trim() : "");
      });
  });
  return loginPathPromise;
}

// Where installers put the binary, for when the login shell cannot be asked.
function usualDirs() {
  return [
    path.join(HOME, ".cargo", "bin"),
    path.join(HOME, ".local", "bin"),
    "/opt/homebrew/bin",
    "/usr/local/bin",
  ];
}

// Can this binary do what the dashboard needs? `overview` is the command the
// whole view is built from, and it did not exist in the first release. Asking
// for its help text is quick, scans nothing, and fails on a cohors too old to
// have it.
function supportsOverview(file) {
  return new Promise((resolve) => {
    cp.execFile(file, ["overview", "--help"], { timeout: 5000, windowsHide: true },
      (err) => resolve(!err));
  });
}

/**
 * @param {() => {binaryPath?: string, roots?: string[], configPath?: string, cwd?: string}} getConfig
 *   Read on every call, so a settings change takes effect on the next refresh.
 * @param {{searchDirs?: () => Promise<string[]>, log?: (line: string) => void}} [opts]
 *   `searchDirs` replaces where the binary is looked for (the checks use it).
 *   `log` is told which binary was chosen and what each call did.
 */
function create(getConfig, opts = {}) {
  let located = null;   // { path, ... } | { path: null, reason, configured }
  const log = (line) => { try { if (opts.log) opts.log(line); } catch { /* a log must never break a call */ } };

  // Every directory a cohors could be in, most likely first.
  async function searchDirs() {
    if (opts.searchDirs) return opts.searchDirs();
    const sep = WINDOWS ? ";" : ":";
    return [
      ...(await loginPath()).split(":"),
      ...String(process.env.PATH || "").split(sep),
      ...usualDirs(),
    ].filter(Boolean);
  }

  async function locate() {
    if (located) return located;
    await find();
    if (located.path) {
      log(`using ${located.path}${located.configured ? " (the cohors.binaryPath setting)" : ""}`
        + (located.passedOver && located.passedOver.length ? `; passed over ${located.passedOver.join(", ")}: too old to have \`cohors overview\`` : "")
        + (located.outdated ? "; it is too old to have `cohors overview`, and no newer cohors was found" : ""));
    } else {
      log(`no cohors to run: ${located.reason}`);
    }
    return located;
  }

  async function find() {
    const configured = String((getConfig().binaryPath || "")).trim();
    if (configured) {
      const p = expandHome(configured);
      located = isExecutable(p)
        ? { path: p, configured: true }
        : { path: null, configured: true, reason: `The cohors.binaryPath setting points at ${configured}, which is not an executable file.` };
      return located;
    }
    // Every cohors on the machine, each once (a directory can be listed twice,
    // and two directories can link to the same file).
    const found = [];
    const seen = new Set();
    for (const d of await searchDirs()) {
      const p = path.join(d, BIN_NAME);
      if (!isExecutable(p)) continue;
      let real = p;
      try { real = fs.realpathSync(p); } catch { /* keep the path as given */ }
      if (seen.has(real)) continue;
      seen.add(real);
      found.push(p);
    }
    if (!found.length) {
      located = { path: null, configured: false, reason: "cohors was not found on your PATH." };
      return located;
    }
    // The first one on the PATH is not always the one to use: a machine can
    // have an old cohors from one installer ahead of a new one from another,
    // both reporting the same version. Take the first that can do the job.
    for (const p of found) {
      if (await supportsOverview(p)) {
        located = { path: p, configured: false, passedOver: found.slice(0, found.indexOf(p)) };
        return located;
      }
    }
    // None can: use the first, and let the error say which file is too old.
    located = { path: found[0], configured: false, outdated: true };
    return located;
  }

  // Forget where the binary is (and the login PATH): the user installed it,
  // or changed the setting.
  function reset() { located = null; loginPathPromise = null; }

  // Where cohors is run: the folder the caller says the user is in, else
  // their home directory. Never the process's own directory.
  function workingDir() {
    const dir = String(getConfig().cwd || "").trim();
    return dir && isDirectory(dir) ? dir : HOME;
  }

  // Global flags go before the subcommand.
  function globalArgs() {
    const c = getConfig();
    const out = [];
    if (c.configPath) out.push("--config", expandHome(String(c.configPath)));
    for (const r of c.roots || []) {
      if (typeof r === "string" && r.trim()) out.push("--root", expandHome(r.trim()));
    }
    return out;
  }

  async function run(args, opts = {}) {
    const bin = await locate();
    if (!bin.path) throw new CohorsError("missing", bin.reason, { configured: bin.configured });
    const full = [...globalArgs(), ...args];
    const started = Date.now();
    return new Promise((resolve, reject) => {
      cp.execFile(bin.path, full,
        // With no roots configured, cohors looks around the directory it is
        // run in. An editor's own working directory is `/`, which says
        // nothing; the open folder is where the user is working.
        { encoding: "utf8", timeout: opts.timeout || 60000, maxBuffer: 64 << 20, cwd: opts.cwd || workingDir(), windowsHide: true },
        (err, stdout, stderr) => {
          log(`cohors ${full.join(" ")}: ${err ? `failed (${String(stderr || err.message).trim().split("\n")[0]})` : `ok, ${String(stdout).length} bytes`} in ${Date.now() - started} ms`);
          if (!err) return resolve(String(stdout));
          const text = String(stderr || "").trim();
          if (err.code === "ENOENT") {
            located = null;
            return reject(new CohorsError("missing", `${bin.path} can no longer be run.`, { configured: bin.configured }));
          }
          if (err.killed || err.signal === "SIGTERM") {
            return reject(new CohorsError("timeout", `cohors ${args[0]} did not finish in time.`, { args: full }));
          }
          if (/unrecognized subcommand|unexpected argument/.test(text)) {
            // Name the file: with two installs on one machine, "update cohors"
            // does not say which one is being run.
            return reject(new CohorsError("unsupported",
              `${bin.path} is an older cohors without \`cohors ${args.slice(0, 2).join(" ")}\`. Update it, or point the cohors.binaryPath setting at a newer one.`,
              { stderr: text, args: full, path: bin.path }));
          }
          // The first line is the reason; the rest is usage noise.
          const first = text.split("\n").map((l) => l.trim()).find(Boolean) || `cohors ${args[0]} exited with code ${err.code}.`;
          reject(new CohorsError("failed", first.replace(/^error:\s*/i, ""), { stderr: text, code: err.code, args: full }));
        });
    });
  }

  async function json(args, opts) {
    const out = await run(args, opts);
    try { return JSON.parse(out); }
    catch {
      throw new CohorsError("parse", `cohors ${args[0]} printed something that is not JSON.`, { stdout: out.slice(0, 400) });
    }
  }

  const isObj = (v) => !!v && typeof v === "object" && !Array.isArray(v);
  function expect(ok, what) {
    if (!ok) throw new CohorsError("parse", `cohors ${what} printed JSON in a shape this extension does not understand. Update cohors or the extension.`);
  }

  // The shared Overview model plus `roots`, as `cohors overview --json` and
  // each `cohors watch --json` line carry it.
  function checkOverview(o, what) {
    expect(isObj(o) && Array.isArray(o.repos) && Array.isArray(o.actions) && isObj(o.figures), what);
    o.roots = Array.isArray(o.roots) ? o.roots : [];
    o.work = Array.isArray(o.work) ? o.work : [];
    o.commits = Array.isArray(o.commits) ? o.commits : [];
    o.activity = Array.isArray(o.activity) ? o.activity : [];
    o.focus = Array.isArray(o.focus) ? o.focus : [];
    return o;
  }

  // Flows, agents and tasks per repository, the focus board and recent agent
  // activity, as `cohors launchpad --json` and each watch line carry them.
  function checkLaunchpad(l, what) {
    expect(isObj(l) && Array.isArray(l.repos), what);
    l.board = isObj(l.board) ? l.board : {};
    l.board.focus = Array.isArray(l.board.focus) ? l.board.focus : [];
    l.recent_activity = Array.isArray(l.recent_activity) ? l.recent_activity : [];
    l.summary = isObj(l.summary) ? l.summary : {};
    return l;
  }

  // What to pick up next. Each has a kind, a title, why and the command
  // that carries it out; a resume also names its session. Anything else is
  // dropped, so the view never meets a shape it does not know.
  function checkSuggestions(list) {
    const str = (v) => typeof v === "string";
    return (Array.isArray(list) ? list : []).filter((s) => isObj(s) && str(s.kind) && str(s.title) && str(s.why) && str(s.command)
      && (s.kind !== "resume" || (isObj(s.session) && str(s.session.id) && str(s.session.repo) && str(s.session.runner))));
  }

  const overview = async () => checkOverview(await json(["overview", "--json"]), "overview");
  const launchpad = async () => checkLaunchpad(await json(["launchpad", "--json"]), "launchpad");

  /**
   * Follow the fleet: `cohors watch --json`, one long-lived process that
   * prints the fleet once, then one line per change. Returns at once.
   *   onLine({ seq, reason, changed, overview, launchpad, suggestions })
   *   onExit({ code, signal, stderr, unsupported })  unsupported: this cohors
   *     has no `watch` (older than 0.8), so the caller should poll instead.
   * The child's input stays open; `stop()` closes it, which ends the child,
   * and so does this process going away (the pipe closes with it).
   */
  async function watch({ onLine, onExit }) {
    const bin = await locate();
    if (!bin.path) throw new CohorsError("missing", bin.reason, { configured: bin.configured });
    const full = [...globalArgs(), "watch", "--json"];
    const child = cp.spawn(bin.path, full, { cwd: workingDir(), stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
    log(`cohors ${full.join(" ")}: started (pid ${child.pid})`);
    let pending = "", stderr = "", ended = false;
    const end = (info) => {
      if (ended) return;
      ended = true;
      const text = stderr.trim();
      const unsupported = /unrecognized subcommand|unexpected argument/.test(text);
      log(`cohors watch ended (${info.signal || `code ${info.code}`})${text ? `: ${text.split("\n")[0]}` : ""}`);
      try { onExit({ ...info, stderr: text, unsupported, path: bin.path }); } catch { /* the caller's problem */ }
    };
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      pending += chunk;
      let at;
      while ((at = pending.indexOf("\n")) >= 0) {
        const text = pending.slice(0, at);
        pending = pending.slice(at + 1);
        if (!text.trim()) continue;
        let line;
        try {
          const v = JSON.parse(text);
          if (!isObj(v) || v.type !== "fleet") continue;   // a kind of line this extension does not know
          line = {
            seq: v.seq, reason: String(v.reason || ""), changed: Array.isArray(v.changed) ? v.changed : [],
            overview: checkOverview(v.overview, "watch"), launchpad: checkLaunchpad(v.launchpad, "watch"),
            suggestions: checkSuggestions(v.suggestions),
          };
        } catch (e) {
          log(`cohors watch printed a line this extension cannot read: ${e.message}`);
          continue;
        }
        try { onLine(line); } catch { /* the caller's problem */ }
      }
    });
    // Keep the end of stderr only: it says why the process stopped.
    child.stderr.on("data", (chunk) => { stderr = (stderr + chunk).slice(-4000); });
    child.stdin.on("error", () => { /* it may already be gone */ });
    child.on("error", (e) => end({ code: null, signal: null, error: e.message }));
    child.on("close", (code, signal) => end({ code, signal }));
    return {
      pid: child.pid,
      stop() {
        try { child.stdin.end(); } catch { /* already closed */ }
        // Closing its input ends it; if it does not, end it.
        setTimeout(() => { if (!ended) child.kill(); }, 2000).unref();
      },
      kill() { if (!ended) child.kill(); },
    };
  }

  // `cohors standup --json [--today]`: the configured author's commits.
  async function standup(today) {
    const s = await json(["standup", ...(today ? ["--today"] : []), "--json"]);
    expect(isObj(s) && Array.isArray(s.groups), "standup");
    return s;
  }

  // The same digest as markdown, exactly as the terminal UI copies it.
  const standupMarkdown = (today) => run(["standup", ...(today ? ["--today"] : [])]);

  const contract = () => run(["launchpad", "contract"]);

  async function version() {
    try { return (await run(["--version"], { timeout: 5000 })).trim(); }
    catch { return ""; }
  }

  /**
   * The argument array for a bulk action, e.g.
   *   actionArgs("fetch", { ids: ["/code/a"] }, { dryRun: true })
   *   -> [..global, "fetch", "--select", '{"ids":["/code/a"]}', "--dry-run"]
   * The selector travels as one argv element, so nothing is ever shell-quoted.
   */
  function actionArgs(verb, selector, opts = {}) {
    if (!VERBS.includes(verb)) throw new CohorsError("failed", `Unknown action "${verb}".`);
    const sel = selector && selector.all
      ? { all: true }
      : { ids: ((selector && selector.ids) || []).map(String) };
    if (!sel.all && !sel.ids.length) throw new CohorsError("failed", "No repositories selected.");
    const args = [...globalArgs(), verb, "--select", JSON.stringify(sel)];
    if (verb === "commit") {
      if (!opts.message || !String(opts.message).trim()) throw new CohorsError("failed", "A commit needs a message.");
      args.push("-m", String(opts.message));
    }
    if (opts.dryRun) args.push("--dry-run");
    return args;
  }

  return { locate, reset, globalArgs, run, json, overview, launchpad, watch, standup, standupMarkdown, contract, version, actionArgs };
}

module.exports = { create, CohorsError, VERBS, expandHome };
