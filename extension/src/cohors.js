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

/**
 * @param {() => {binaryPath?: string, roots?: string[], configPath?: string}} getConfig
 *   Read on every call, so a settings change takes effect on the next refresh.
 */
function create(getConfig) {
  let located = null;   // { path } | { path: null, reason, configured }

  async function locate() {
    if (located) return located;
    const configured = String((getConfig().binaryPath || "")).trim();
    if (configured) {
      const p = expandHome(configured);
      located = isExecutable(p)
        ? { path: p, configured: true }
        : { path: null, configured: true, reason: `The cohors.binaryPath setting points at ${configured}, which is not an executable file.` };
      return located;
    }
    const sep = WINDOWS ? ";" : ":";
    const dirs = [
      ...(await loginPath()).split(":"),
      ...String(process.env.PATH || "").split(sep),
      ...usualDirs(),
    ].filter(Boolean);
    for (const d of dirs) {
      const p = path.join(d, BIN_NAME);
      if (isExecutable(p)) { located = { path: p, configured: false }; return located; }
    }
    located = { path: null, configured: false, reason: "cohors was not found on your PATH." };
    return located;
  }

  // Forget where the binary is (and the login PATH): the user installed it,
  // or changed the setting.
  function reset() { located = null; loginPathPromise = null; }

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
    return new Promise((resolve, reject) => {
      cp.execFile(bin.path, full,
        { encoding: "utf8", timeout: opts.timeout || 60000, maxBuffer: 64 << 20, cwd: opts.cwd, windowsHide: true },
        (err, stdout, stderr) => {
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
            return reject(new CohorsError("unsupported",
              `This version of cohors does not have \`cohors ${args.slice(0, 2).join(" ")}\`. Update cohors.`,
              { stderr: text, args: full }));
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

  // `cohors overview --json`: the shared Overview model plus `roots`.
  async function overview() {
    const o = await json(["overview", "--json"]);
    expect(isObj(o) && Array.isArray(o.repos) && Array.isArray(o.actions) && isObj(o.figures), "overview");
    o.roots = Array.isArray(o.roots) ? o.roots : [];
    o.work = Array.isArray(o.work) ? o.work : [];
    o.commits = Array.isArray(o.commits) ? o.commits : [];
    o.activity = Array.isArray(o.activity) ? o.activity : [];
    o.focus = Array.isArray(o.focus) ? o.focus : [];
    return o;
  }

  // `cohors launchpad --json`: flows, agents and tasks per repository, the
  // focus board and recent agent activity.
  async function launchpad() {
    const l = await json(["launchpad", "--json"]);
    expect(isObj(l) && Array.isArray(l.repos), "launchpad");
    l.board = isObj(l.board) ? l.board : {};
    l.board.focus = Array.isArray(l.board.focus) ? l.board.focus : [];
    l.recent_activity = Array.isArray(l.recent_activity) ? l.recent_activity : [];
    l.summary = isObj(l.summary) ? l.summary : {};
    return l;
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

  return { locate, reset, globalArgs, run, json, overview, launchpad, standup, standupMarkdown, contract, version, actionArgs };
}

module.exports = { create, CohorsError, VERBS, expandHome };
