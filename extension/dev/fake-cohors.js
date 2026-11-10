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
  case "fetch": case "pull": case "push": case "commit": case "stash":
    print(`\n  cohors (fixtures)\n  ${command} ${rest.join(" ")}\n\n  This stand-in does not touch any repository.\n\n`);
    break;
  default:
    unknown(command);
}
