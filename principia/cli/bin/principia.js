#!/usr/bin/env node
// `principia init` - see steps.js for what each step actually does and what
// it requires to run at all. Nothing here assumes a step succeeded; every
// step's prerequisite is checked before it runs, and one step failing never
// stops the ones after it.

const path = require("path");
const { buildSteps, findContractRoot } = require("../src/steps");
const ui = require("../src/ui");

function parseArgs(argv) {
  const out = { yes: false, contract: null, cmd: argv[0] || "help" };
  for (let i = 1; i < argv.length; i++) {
    if (argv[i] === "--yes" || argv[i] === "-y") out.yes = true;
    else if (argv[i] === "--contract") out.contract = argv[++i];
  }
  return out;
}

function printHelp() {
  const { c, sym, line } = ui;
  line();
  line(`  ${c.bold("principia")} ${c.dim(sym.dot)} install and wire up the Principia launchpad`);
  line();
  line(`  ${c.bold("Usage")}`);
  line();
  const cmds = [
    ["init [options]", "Run every setup step"],
    ["status", "Show what is already in place, change nothing"],
    ["help", "This message"],
  ];
  for (const [cmd, desc] of cmds) line(`    ${c.cyan(cmd.padEnd(18))} ${c.dim(desc)}`);
  line();
  line(`  ${c.bold("Options")}`);
  line();
  const opts = [
    ["--yes, -y", "Skip a runner's own confirmation prompts (for scripted runs)"],
    ["--contract <path>", "Path to a Principia checkout, if not found automatically"],
  ];
  for (const [flag, desc] of opts) line(`    ${c.cyan(flag.padEnd(18))} ${c.dim(desc)}`);
  line();
  line(`  ${c.dim("Every step checks its own prerequisites and is skipped, not fatal,")}`);
  line(`  ${c.dim("if they are unmet. Re-running is safe.")}`);
  line();
}

async function run() {
  const args = parseArgs(process.argv.slice(2));
  if (args.cmd === "help" || args.cmd === "--help" || args.cmd === "-h") return printHelp();

  const cwd = process.cwd();
  const statusOnly = args.cmd === "status";
  const contractRoot = findContractRoot(args.contract);
  const steps = buildSteps({ cwd, contractRoot, yes: args.yes });
  const started = Date.now();

  ui.header(statusOnly ? "status" : "init", cwd);

  const results = [];
  for (const step of steps) {
    let check;
    try { check = step.check(); }
    catch (e) { check = { ok: false, reason: `check threw: ${e.message}` }; }

    if (!check.ok) {
      ui.step("skip", step.label, `skipped: ${check.reason}`);
      results.push({ id: step.id, status: "skipped", reason: check.reason });
      continue;
    }

    if (statusOnly && !step.readonly) {
      ui.step("ok", step.label, null, "ready");
      results.push({ id: step.id, status: "ready" });
      continue;
    }

    // A real terminal gets live feedback while a slow step (npm install, a
    // plugin install) runs; a pipe gets nothing extra and stays clean.
    const clear = ui.transient(`  ${ui.c.dim(ui.sym.work)}  ${ui.c.dim(step.label)}`);
    let r;
    try { r = step.run(); }
    catch (e) { r = { ok: false, detail: `error: ${e.message}` }; }
    clear();

    ui.step(r.ok ? "ok" : "fail", step.label, r.detail);
    results.push({
      id: step.id,
      status: r.ok ? (statusOnly ? "checked" : "done") : "failed",
      detail: r.detail,
    });
  }

  if (statusOnly) {
    const ready = results.filter((r) => r.status === "ready" || r.status === "checked").length;
    const blocked = results.filter((r) => r.status === "skipped").length;
    ui.summary({ done: ready, skipped: blocked }, Date.now() - started,
      { done: "ready", skipped: "blocked" });
    ui.note(blocked
      ? `${blocked} step(s) cannot run yet, each with its reason above. Run \`principia init\` to do the rest.`
      : `Everything is in place. Run \`principia init\` to re-apply it safely.`);
    ui.line();
    return;
  }

  const counts = {
    done: results.filter((r) => r.status === "done").length,
    skipped: results.filter((r) => r.status === "skipped").length,
    failed: results.filter((r) => r.status === "failed").length,
  };
  ui.summary(counts, Date.now() - started);

  if (counts.done > 0) {
    ui.section("Next", "Paste this into a fresh agent session in this repository:");
    ui.quote("Set up this repo for the Principia launchpad, then plan my focus across every repo you can see.");
    if (contractRoot) {
      ui.note(`The contract it follows: ${path.join(contractRoot, "spec/v1/SPEC.md")}`);
    }
  }
  if (counts.failed > 0) {
    ui.note(`${counts.failed} step(s) failed. Each is independent, so the rest still applied; re-run after fixing the cause.`);
  }
  ui.line();
}

run().catch((e) => {
  ui.line();
  ui.line(`  ${ui.c.red(ui.sym.fail)}  ${ui.c.bold("principia failed")}`);
  for (const l of ui.wrapText(e.message, 5)) ui.line(ui.c.dim(l));
  ui.line();
  process.exit(1);
});
