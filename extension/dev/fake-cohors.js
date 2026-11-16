#!/usr/bin/env node
// A stand-in for the cohors binary that answers from dev/fixtures/.
//
// Two uses:
//   1. dev/check.js runs the extension's CLI adapter against it, including
//      the `cohors agent …` contract.
//   2. Point the `cohors.binaryPath` setting at this file to drive the real
//      extension in VS Code with the demo fleet instead of your own repos.
//
// It implements only what the extension calls. Bulk verbs just say what they
// would have done; nothing here touches a repository.

const fs = require("fs");
const path = require("path");

const fixture = (name) => JSON.parse(fs.readFileSync(path.join(__dirname, "fixtures", `${name}.json`), "utf8"));
const print = (value) => process.stdout.write(typeof value === "string" ? value : JSON.stringify(value, null, 2) + "\n");
// The real binary reports a failure as one `Error: <reason>` line on stderr.
function fail(message) { process.stderr.write(`Error: ${message}\n`); process.exit(1); }
function unknown(name) { process.stderr.write(`error: unrecognized subcommand '${name}'\n`); process.exit(2); }

// Global flags come before the subcommand.
const argv = process.argv.slice(2);
const roots = [];
while (argv[0] === "--root" || argv[0] === "--config") {
  const flag = argv.shift();
  const value = argv.shift();
  if (flag === "--root") roots.push(value);
}
const [command, ...rest] = argv;
const flag = (name) => { const i = rest.indexOf(name); return i >= 0 ? rest[i + 1] : undefined; };

switch (command) {
  case "--version":
    print("cohors 0.6.0 (fixtures)\n");
    break;
  case "overview": {
    const o = fixture("overview");
    if (roots.length) o.roots = roots;
    print(o);
    break;
  }
  case "launchpad":
    if (rest[0] === "contract") print({ specVersion: 1, note: "The real cohors prints the full contract here." });
    else print(fixture("launchpad"));
    break;
  case "standup": {
    const s = fixture(rest.includes("--today") ? "standup-today" : "standup-week");
    if (rest.includes("--json")) { print(s); break; }
    const lines = [`## Standup — ${s.window}`, "", `_${s.total} commits across ${s.groups.length} repos_`, ""];
    for (const g of s.groups) {
      lines.push(`### ${g.repo} (${g.commits.length})`);
      for (const c of g.commits) lines.push(`- \`${c.short_id}\` ${c.summary}`);
      lines.push("");
    }
    print(lines.join("\n"));
    break;
  }
  case "agent": {
    const a = fixture("agent");
    if (rest[0] === "runners") { print({ runners: a.runners }); break; }
    if (rest[0] === "prompts") { print({ prompts: a.prompts }); break; }
    if (rest[0] !== "command") unknown(rest[0]);
    const kind = rest[1];
    const wanted = flag("--runner");
    const runner = wanted ? a.runners.find((r) => r.id === wanted) : a.runners.find((r) => r.available);
    if (!runner) fail(wanted ? `unknown runner "${wanted}"` : "no agent runner is installed");
    if (!runner.available) fail(`${runner.label} is not installed: ${runner.reason}`);
    const repo = flag("--repo");
    if (kind !== "plan-day" && !repo) fail(`\`cohors agent command ${kind}\` needs --repo`);
    if (kind === "task" && flag("--task") === "no-such-task") fail(`no task "no-such-task" in ${path.basename(repo)}`);
    const session = flag("--session") || (kind === "task" && runner.can_link ? "11111111-2222-4333-8444-555555555555" : null);
    const what = kind === "agent" ? flag("--agent") : kind === "task" ? flag("--task") : kind;
    print({
      command: kind === "resume" ? `${runner.bin} --resume '${session}'` : `${runner.bin} "$(cat '/tmp/cohors/${what}.md')"`,
      cwd: repo || process.cwd(),
      runner: runner.id,
      label: `${runner.label}: ${what}`,
      prompt_file: kind === "resume" || kind === "start" ? null : `/tmp/cohors/${what}.md`,
      // The exact text the runner would receive; null where there is none.
      prompt: kind === "resume" || kind === "start" ? null : `Stand-in prompt for ${what}.\nThe real cohors puts the full instructions here.`,
      linked: !!session && kind !== "resume",
      session,
    });
    break;
  }
  case "watch": {
    // Like the real one: the fleet once, then a line per change, until its
    // input closes. Here the "change" is a second line a moment later, and
    // with it news of a newer cohors (the real stream finds that in the
    // background and carries it the same way).
    if (!rest.includes("--json")) unknown("watch");
    const o = fixture("overview");
    if (roots.length) o.roots = roots;
    const update = {
      version: "0.9.0",
      notice: "Update available: 0.9.0 — run brew upgrade cohors",
      detail: { state: "available", version: "0.9.0", command: "brew upgrade cohors" },
    };
    const line = (seq, reason, changed, news) => process.stdout.write(JSON.stringify({
      type: "fleet", seq, reason, changed, overview: o, launchpad: fixture("launchpad"), suggestions: fixture("suggestions"), update: news,
    }) + "\n");
    line(1, "start", o.repos.map((r) => r.id), null);
    setTimeout(() => line(2, "git", [o.repos[0].id], update), 50);
    process.stdin.on("end", () => process.exit(0));
    process.stdin.resume();
    break;
  }
  case "show": {
    // `cohors show --json -- <repo>`: the one fixture, as the repository named
    // (by its id, name or path), so every row of the demo fleet has a detail.
    const key = rest.filter((a) => a !== "--json" && a !== "--").pop();
    const repo = fixture("overview").repos.find((r) => r.id === key || r.name === key || r.path === key);
    if (!repo) fail(`no repository matches \`${key}\``);
    print(Object.assign(fixture("show"), { id: repo.id, name: repo.name, path: repo.path }));
    break;
  }
  case "fetch": case "pull": case "push": case "commit": case "stash": case "run":
    print(`\n  cohors (fixtures)\n  ${command} ${rest.join(" ")}\n\n  This stand-in does not touch any repository.\n\n`);
    break;
  default:
    unknown(command);
}
