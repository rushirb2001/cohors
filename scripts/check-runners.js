#!/usr/bin/env node
// Every place that names the set of agent runners must name the same set.
//
// shared/runners.js is the one implementation, but four other files legitimately
// spell the list out again: the JSON Schema cannot import code, and the
// validator and MCP server are deliberately dependency-free so an agent can run
// them anywhere. Those copies are fine — silently disagreeing is not. Adding a
// runner in one place and forgetting the others is the failure this catches.
//
//   node scripts/check-runners.js
//
// Exits 0 when they agree, 1 when they do not.

const fs = require("fs");
const path = require("path");

const root = path.join(__dirname, "..");
const read = (rel) => fs.readFileSync(path.join(root, rel), "utf8");
const norm = (ids) => [...ids].sort().join(", ");

// Textual extraction on purpose: requiring scripts/validate.js would run it and
// call process.exit, and the schema is data, not code.
function fromArrayLiteral(rel, varName) {
  const m = read(rel).match(new RegExp(`const ${varName}\\s*=\\s*\\[([^\\]]*)\\]`));
  if (!m) return null;
  return [...m[1].matchAll(/"([a-z0-9-]+)"/g)].map((x) => x[1]);
}

function fromSchema(rel) {
  const doc = JSON.parse(read(rel));
  const items = (((doc.properties || {}).agents || {}).items || {});
  const supports = ((items.properties || {}).supports || {});
  return ((supports.items || {}).enum) || null;
}

// The server declares the same enum in more than one tool; every one must match.
function fromZodEnums(rel) {
  const body = read(rel);
  const found = [...body.matchAll(/z\.enum\(\[([^\]]*)\]\)/g)]
    .map((m) => [...m[1].matchAll(/"([a-z0-9-]+)"/g)].map((x) => x[1]))
    .filter((list) => list.includes("claude-code"));
  return found.length ? found : null;
}

const sources = [];
const add = (name, ids) => sources.push({ name, ids });

add("shared/runners.js", require("../shared/runners").RUNNERS.map((r) => r.id));
add("spec/v1/repo.schema.json", fromSchema("spec/v1/repo.schema.json"));
add("scripts/validate.js", fromArrayLiteral("scripts/validate.js", "RUNNERS"));
add("mcp/src/lib.js", fromArrayLiteral("mcp/src/lib.js", "RUNNERS"));
for (const [i, ids] of (fromZodEnums("mcp/src/server.js") || []).entries()) {
  add(`mcp/src/server.js (enum ${i + 1})`, ids);
}

const missing = sources.filter((s) => !s.ids);
const expected = norm(sources[0].ids);
const disagree = sources.filter((s) => s.ids && norm(s.ids) !== expected);

for (const s of sources) {
  const state = !s.ids ? "COULD NOT READ" : norm(s.ids) === expected ? "ok" : "DIFFERS";
  console.log(`  ${state.padEnd(14)} ${s.name}${s.ids ? `  [${norm(s.ids)}]` : ""}`);
}

if (missing.length || disagree.length) {
  console.log(`\nRunner lists disagree. Canonical is shared/runners.js: [${expected}]`);
  process.exit(1);
}
console.log(`\n${sources.length} sources agree: [${expected}]`);
