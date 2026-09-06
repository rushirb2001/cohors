// Agent runners. No runner is privileged: each is a row in this table, and the
// prompt text is identical across all of them (see prompts/ in the repo).
//
// A runner that is not installed is reported as unavailable WITH A REASON, so
// the UI can show it disabled rather than silently hiding an action.

const fs = require("fs");
const os = require("os");
const path = require("path");
const cp = require("child_process");

const HOME = os.homedir();

const RUNNERS = [
  {
    id: "claude-code",
    label: "Claude Code",
    bin: "claude",
    // $PROMPT is substituted with a shell-quoted absolute path to the prompt file.
    command: (p) => `claude --append-system-prompt "$(cat ${p})"`,
  },
  {
    id: "codex",
    label: "Codex",
    bin: "codex",
    command: (p) => `codex --prompt-file ${p}`,
  },
  {
    id: "gemini-cli",
    label: "Gemini CLI",
    bin: "gemini",
    command: (p) => `gemini -p "$(cat ${p})"`,
  },
  {
    id: "agy",
    label: "agy",
    bin: "agy",
    command: (p) => `agy run --prompt ${p}`,
  },
];

// VS Code's process PATH is not the user's login PATH, so `which` from here
// misses anything installed by a shell profile. Ask a login shell instead.
let cache = null;
function detect() {
  if (cache) return cache;
  let loginPath = process.env.PATH || "";
  try {
    const shell = process.env.SHELL || "/bin/zsh";
    loginPath = cp.execFileSync(shell, ["-lic", "echo -n $PATH"], {
      encoding: "utf8", timeout: 4000, stdio: ["ignore", "pipe", "ignore"],
    }).trim() || loginPath;
  } catch { /* fall back to the process PATH */ }

  const dirs = loginPath.split(":").filter(Boolean);
  cache = RUNNERS.map((r) => {
    let found = null;
    for (const d of dirs) {
      const p = path.join(d, r.bin);
      try { fs.accessSync(p, fs.constants.X_OK); found = p; break; } catch { /* keep looking */ }
    }
    return {
      id: r.id,
      label: r.label,
      bin: r.bin,
      available: !!found,
      pathTo: found,
      reason: found ? null : `${r.bin} was not found on your PATH`,
    };
  });
  return cache;
}

function resetDetection() { cache = null; }

function byId(id) { return RUNNERS.find((r) => r.id === id); }

function commandFor(runnerId, promptFile) {
  const r = byId(runnerId);
  if (!r) return null;
  return r.command(JSON.stringify(promptFile));
}

// One shape for every runner, so a cross-runner timeline is possible at all.
function recordLaunch({ runner, repo, branch, agent }) {
  try {
    const dir = path.join(HOME, ".principia", "history");
    fs.mkdirSync(dir, { recursive: true });
    const day = new Date().toISOString().slice(0, 10);
    const rec = {
      ts: new Date().toISOString(),
      phase: "launch",
      runner,
      repo: repo || "",
      branch: branch || "",
      agent: agent || "",
      source: "principia-extension",
    };
    fs.appendFileSync(path.join(dir, `${day}.jsonl`), JSON.stringify(rec) + "\n");
  } catch { /* history is best-effort; never block a launch */ }
}

module.exports = { RUNNERS, detect, resetDetection, commandFor, recordLaunch };
