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

// Single-quote for POSIX shells. Inside single quotes nothing expands, so this
// is the only safe way to put a path we did not author into a shell string.
// The path reaches us from a repository's committed repo.json, which on a cloned
// repo is attacker-controlled.
function shq(s) {
  return "'" + String(s).replace(/'/g, "'\\''") + "'";
}

// Every mapping must do BOTH things: submit the prompt as the first user turn,
// and leave the user in an interactive session. Getting only the first gives a
// headless run that prints and exits; getting only the second drops the user
// into an idle session with nothing asked, which is what
// `--append-system-prompt` alone did here: it appends to the SYSTEM prompt and
// never submits a turn, so the agent just sat there waiting to be told
// something. A prompt file is still written and cat'd in, so the text is
// identical across runners and never depends on shell history or arg parsing.
// `start` launches a prompt. `startWith` launches it under a session id we
// choose, and `resume` re-enters that exact session later — together they let a
// task be picked up in the conversation it came from. A runner that cannot do
// one of those declares null and is offered the plain launch instead; nothing
// here pretends a capability a CLI does not have.
const RUNNERS = [
  {
    id: "claude-code",
    label: "Claude Code",
    bin: "claude",
    // The path is single-quoted; $(...) output is passed as one argument and is
    // never re-expanded, so prompt content cannot break out either.
    // `claude [prompt]`: positional prompt starts an interactive session with
    // that prompt already submitted. -p/--print would be headless instead.
    command: (p) => `claude "$(cat ${shq(p)})"`,
    startWith: (p, id) => `claude --session-id ${shq(id)} "$(cat ${shq(p)})"`,
    resume: (id) => `claude --resume ${shq(id)}`,
  },
  {
    id: "codex",
    label: "Codex",
    bin: "codex",
    // `codex [PROMPT]`: positional, forwarded to the interactive CLI. There is
    // no --prompt-file flag.
    command: (p) => `codex "$(cat ${shq(p)})"`,
    // Codex has no flag to choose a new session's id, so Principia cannot link
    // one at launch. It can still resume an id recorded some other way.
    startWith: null,
    resume: (id) => `codex resume ${shq(id)}`,
  },
  {
    id: "gemini-cli",
    label: "Gemini CLI",
    bin: "gemini",
    // -i/--prompt-interactive runs the prompt and stays interactive.
    // -p/--prompt is explicitly the non-interactive/headless mode.
    command: (p) => `gemini -i "$(cat ${shq(p)})"`,
    startWith: (p, id) => `gemini --session-id ${shq(id)} -i "$(cat ${shq(p)})"`,
    // `gemini --resume` takes "latest" or an index, not a UUID, so a recorded
    // id cannot be reopened directly. Claimed only where it is real.
    resume: null,
  },
  {
    id: "agy",
    label: "agy",
    bin: "agy",
    // --prompt is an alias for --print (headless), and there is no `run`
    // subcommand; --prompt-interactive is the one that keeps the session.
    command: (p) => `agy --prompt-interactive "$(cat ${shq(p)})"`,
    startWith: null,
    resume: (id) => `agy --conversation ${shq(id)}`,
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
  // JSON.stringify is NOT shell quoting. Each runner shell-quotes via shq().
  return r.command(promptFile);
}

// Launch under a session id we chose, so the task and the conversation it
// starts are linked without having to guess afterwards. Falls back to a plain
// launch when the runner cannot be told which id to use.
function startWithSession(runnerId, promptFile, sessionId) {
  const r = byId(runnerId);
  if (!r) return null;
  if (!r.startWith) return { cmd: r.command(promptFile), linked: false };
  return { cmd: r.startWith(promptFile, sessionId), linked: true };
}

function resumeCommand(runnerId, sessionId) {
  const r = byId(runnerId);
  return r && r.resume ? r.resume(sessionId) : null;
}

const canLinkSessions = (runnerId) => !!(byId(runnerId) || {}).startWith;
const canResume = (runnerId) => !!(byId(runnerId) || {}).resume;

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

module.exports = { RUNNERS, detect, resetDetection, commandFor, startWithSession,
  resumeCommand, canLinkSessions, canResume, recordLaunch, shq };
