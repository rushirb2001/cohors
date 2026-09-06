#!/usr/bin/env node
// `principia init` - see steps.js for what each step actually does and what
// it requires to run at all. Nothing here assumes a step succeeded; every
// step's prerequisite is checked before it runs, and one step failing never
// stops the ones after it.

const path = require("path");
const { buildSteps, findContractRoot } = require("../src/steps");

function parseArgs(argv) {
  const out = { yes: false, contract: null, cmd: argv[0] || "help" };
  for (let i = 1; i < argv.length; i++) {
    if (argv[i] === "--yes" || argv[i] === "-y") out.yes = true;
    else if (argv[i] === "--contract") out.contract = argv[++i];
  }
  return out;
}

function printHelp() {
  console.log(`
principia - install and wire up the Principia launchpad

Usage:
  principia init [--yes] [--contract <path>]   Run every setup step
  principia status                             Show what is already in place, change nothing
  principia help                               This message

--yes        Skip Claude Code's own confirmation prompts (for scripted runs)
--contract   Path to a Principia checkout, if not found automatically
`.trim());
}

async function run() {
  const args = parseArgs(process.argv.slice(2));
  if (args.cmd === "help" || args.cmd === "--help" || args.cmd === "-h") return printHelp();

  const cwd = process.cwd();
  const contractRoot = findContractRoot(args.contract);
  const steps = buildSteps({ cwd, contractRoot, yes: args.yes });

  console.log(`Principia ${args.cmd === "status" ? "status" : "init"} — running in ${cwd}\n`);

  const results = [];
  for (const step of steps) {
    let check;
    try { check = step.check(); }
    catch (e) { check = { ok: false, reason: `check threw: ${e.message}` }; }

    if (!check.ok) {
      console.log(`  –  ${step.label}`);
      console.log(`     skipped: ${check.reason}`);
      results.push({ id: step.id, status: "skipped", reason: check.reason });
      continue;
    }

    if (args.cmd === "status" && !step.readonly) {
      console.log(`  ✔  ${step.label} (prerequisites met)`);
      results.push({ id: step.id, status: "ready" });
      continue;
    }

    try {
      const r = step.run();
      if (r.ok) {
        console.log(`  ✔  ${step.label}`);
        if (r.detail) console.log(`     ${r.detail}`);
        results.push({ id: step.id, status: args.cmd === "status" ? "checked" : "done", detail: r.detail });
      } else {
        console.log(`  ✖  ${step.label}`);
        if (r.detail) console.log(`     ${r.detail}`);
        results.push({ id: step.id, status: "failed", detail: r.detail });
      }
    } catch (e) {
      console.log(`  ✖  ${step.label}`);
      console.log(`     error: ${e.message}`);
      results.push({ id: step.id, status: "failed", reason: e.message });
    }
  }

  if (args.cmd === "status") return;

  const done = results.filter((r) => r.status === "done").length;
  const skipped = results.filter((r) => r.status === "skipped").length;
  const failed = results.filter((r) => r.status === "failed").length;
  console.log(`\n${done} done, ${skipped} skipped, ${failed} failed.`);

  if (done > 0) {
    console.log(`
Paste this into a fresh Claude Code session in this repo to get started:

  Set up this repo for the Principia launchpad, then plan my focus across
  every repo you can see. Follow the Principia contract at
  ${path.join(contractRoot || "<contract>", "spec/v1/SPEC.md")}.
`);
  }
}

run().catch((e) => { console.error("principia init failed:", e.message); process.exit(1); });
