// Every init step declares its own prerequisite check, separate from its
// action. The runner in bin/principia.js calls check() first; if it fails,
// the step is SKIPPED with a stated reason and the run continues with
// whatever comes next. One step failing must never take down the others -
// that was the explicit ask: verify prerequisites, don't just hope.

const fs = require("fs");
const os = require("os");
const path = require("path");
const cp = require("child_process");

const HOME = os.homedir();
const exists = (p) => { try { fs.accessSync(p); return true; } catch { return false; } };

function sh(cmd, args, opts = {}) {
  try {
    const out = cp.execFileSync(cmd, args, { encoding: "utf8", timeout: opts.timeout || 15000, stdio: ["ignore", "pipe", "pipe"] });
    return { ok: true, out };
  } catch (e) {
    return { ok: false, out: (e.stdout || "") + (e.stderr || ""), error: e };
  }
}

// Interactive by default (lets Claude Code show its own confirmation prompt,
// which is the honest default); --yes threads -y through for scripted runs.
function shInherit(cmd, args) {
  const r = cp.spawnSync(cmd, args, { stdio: "inherit" });
  return { ok: r.status === 0, status: r.status };
}

function findContractRoot(explicit) {
  const candidates = [
    explicit,
    process.env.PRINCIPIA_CONTRACT,
    path.join(__dirname, "..", ".."), // cli/src/.. /.. -> repo root, when run from a checkout
  ].filter(Boolean);
  for (const c of candidates) {
    if (exists(path.join(c, "spec", "v1", "SPEC.md"))) return path.resolve(c);
  }
  return null;
}

function gitRoot(cwd) {
  const r = sh("git", ["-C", cwd, "rev-parse", "--show-toplevel"]);
  return r.ok ? r.out.trim() : null;
}

function buildSteps({ cwd, contractRoot, yes }) {
  const state = {}; // steps stash results here for later steps to read

  return [
    {
      id: "contract",
      readonly: true,
      label: "Locate the Principia contract",
      check: () => contractRoot
        ? { ok: true }
        : { ok: false, reason: "spec/v1/SPEC.md not found. Pass --contract <path> or set PRINCIPIA_CONTRACT." },
      run: () => { state.contract = contractRoot; return { ok: true, detail: contractRoot }; },
    },

    {
      id: "runners",
      readonly: true,
      label: "Detect agent runners",
      check: () => ({ ok: true }),
      run: () => {
        const { detect } = require("./runners");
        const list = detect();
        state.runners = list;
        const avail = list.filter((r) => r.available).map((r) => r.label);
        return { ok: true, detail: avail.length ? avail.join(", ") : "none found on PATH" };
      },
    },

    {
      id: "git",
      readonly: true,
      label: "Confirm this is a git repository",
      check: () => {
        const root = gitRoot(cwd);
        state.gitRoot = root;
        return root ? { ok: true } : { ok: false, reason: `${cwd} is not inside a git repository` };
      },
      run: () => ({ ok: true, detail: state.gitRoot }),
    },

    {
      id: "claude-plugin",
      label: "Register the Claude Code plugin (marketplace + install)",
      check: () => {
        if (!state.runners || !state.runners.find((r) => r.id === "claude-code" && r.available)) {
          return { ok: false, reason: "claude was not detected on PATH" };
        }
        const v = sh(state.runners.find((r) => r.id === "claude-code").pathTo, ["plugin", "--help"]);
        return v.ok ? { ok: true } : { ok: false, reason: "this claude version does not support `plugin` subcommands" };
      },
      run: () => {
        const bin = state.runners.find((r) => r.id === "claude-code").pathTo;
        const list = sh(bin, ["plugin", "marketplace", "list"]);
        const haveMarketplace = list.ok && /^\s*[❯>-]?\s*principia\b/m.test(list.out);
        if (!haveMarketplace) {
          const add = shInherit(bin, ["plugin", "marketplace", "add", state.contract]);
          if (!add.ok) return { ok: false, detail: "marketplace add failed or was declined" };
        }
        const installed = sh(bin, ["plugin", "list"]);
        const havePlugin = installed.ok && /principia/.test(installed.out);
        if (havePlugin) return { ok: true, detail: "already installed" };
        const args = ["plugin", "install", "principia@principia"];
        if (yes) args.push("-y");
        const install = shInherit(bin, args);
        return install.ok ? { ok: true, detail: "installed" } : { ok: false, detail: "install failed or was declined" };
      },
    },

    {
      id: "project-skills",
      label: "Write project-local Claude Code skills (works even without the plugin)",
      check: () => state.gitRoot ? { ok: true } : { ok: false, reason: "not inside a git repository" },
      run: () => {
        const src = path.join(state.contract, "skills");
        const dest = path.join(state.gitRoot, ".claude", "skills");
        let n = 0;
        for (const name of fs.readdirSync(src)) {
          const file = path.join(src, name, "SKILL.md");
          if (!exists(file)) continue;
          fs.mkdirSync(path.join(dest, name), { recursive: true });
          fs.copyFileSync(file, path.join(dest, name, "SKILL.md"));
          n++;
        }
        return { ok: true, detail: `${n} skill file(s) -> .claude/skills/` };
      },
    },

    {
      id: "agents-md",
      label: "Point non-Claude runners (Codex, Gemini CLI, agy) at the contract",
      check: () => {
        if (!state.gitRoot) return { ok: false, reason: "not inside a git repository" };
        const others = (state.runners || []).filter((r) => r.id !== "claude-code" && r.available);
        return others.length ? { ok: true } : { ok: false, reason: "no other runner detected on PATH" };
      },
      run: () => {
        const file = path.join(state.gitRoot, "AGENTS.md");
        const block = [
          "## Principia",
          "",
          "This repository can describe itself to the Principia launchpad.",
          `See ${path.relative(state.gitRoot, path.join(state.contract, "prompts", "setup-repo.md")) || "prompts/setup-repo.md"} for how, and`,
          `${path.relative(state.gitRoot, path.join(state.contract, "prompts", "plan-day.md")) || "prompts/plan-day.md"} to plan cross-repo focus.`,
          "",
        ].join("\n");
        if (exists(file)) {
          const body = fs.readFileSync(file, "utf8");
          if (body.includes("## Principia")) return { ok: true, detail: "already present" };
          fs.writeFileSync(file, body.replace(/\n*$/, "\n\n") + block);
        } else {
          fs.writeFileSync(file, block);
        }
        return { ok: true, detail: "AGENTS.md" };
      },
    },

    {
      id: "home",
      label: "Set up ~/.principia (board + history)",
      check: () => ({ ok: true }),
      run: () => {
        const home = path.join(HOME, ".principia");
        fs.mkdirSync(path.join(home, "history"), { recursive: true });
        const board = path.join(home, "board.json");
        if (!exists(board)) fs.writeFileSync(board, JSON.stringify({ specVersion: 1, focus: [] }, null, 2) + "\n");
        return { ok: true, detail: home };
      },
    },

    {
      id: "vscode-extension",
      label: "Install the Launch pad extension (dev symlink)",
      check: () => {
        const targets = [
          { name: "VS Code", dir: path.join(HOME, ".vscode", "extensions") },
          { name: "Cursor", dir: path.join(HOME, ".cursor", "extensions") },
        ].filter((t) => exists(t.dir));
        if (!targets.length) return { ok: false, reason: "no VS Code or Cursor extensions directory found" };
        state.editorTargets = targets;
        return { ok: true };
      },
      run: () => {
        const src = path.join(state.contract, "extension");
        const pkg = JSON.parse(fs.readFileSync(path.join(src, "package.json"), "utf8"));
        const done = [];
        for (const t of state.editorTargets) {
          const dest = path.join(t.dir, `${pkg.publisher}.${pkg.name}-${pkg.version}`);
          if (exists(dest)) { done.push(`${t.name} (already linked)`); continue; }
          fs.symlinkSync(src, dest, "dir");
          done.push(t.name);
        }
        return { ok: true, detail: done.join(", ") + " - reload the window to activate" };
      },
    },

    {
      id: "gitignore",
      label: "Gitignore .principia/local.json",
      check: () => state.gitRoot ? { ok: true } : { ok: false, reason: "not inside a git repository" },
      run: () => {
        const file = path.join(state.gitRoot, ".gitignore");
        const line = ".principia/local.json";
        const body = exists(file) ? fs.readFileSync(file, "utf8") : "";
        if (body.includes(line)) return { ok: true, detail: "already present" };
        fs.writeFileSync(file, body + (body && !body.endsWith("\n") ? "\n" : "") + line + "\n");
        return { ok: true, detail: ".gitignore" };
      },
    },
  ];
}

module.exports = { buildSteps, findContractRoot, gitRoot, sh, exists, HOME };
