// Terminal presentation for the CLI. Zero dependencies, on purpose: this
// package has none and an installer is the last place to want a dependency
// tree.
//
// Two rules:
//  1. Colour is an enhancement, never the message. NO_COLOR, a dumb terminal,
//     or a pipe all degrade to plain text that still reads correctly — the
//     glyph and the words carry the meaning, not the escape codes.
//  2. Nothing is written that a non-TTY cannot represent. The live "working…"
//     line only exists when there is a real terminal to rewrite.

const stream = process.stdout;

function colorEnabled() {
  if (process.env.NO_COLOR) return false;          // no-color.org
  if (process.env.FORCE_COLOR) return true;
  if (process.env.TERM === "dumb") return false;
  return stream.isTTY === true;
}

const ON = colorEnabled();
const wrapCode = (open, close) => (s) => (ON ? `\x1b[${open}m${s}\x1b[${close}m` : String(s));

const c = {
  bold: wrapCode(1, 22),
  dim: wrapCode(2, 22),
  red: wrapCode(31, 39),
  green: wrapCode(32, 39),
  yellow: wrapCode(33, 39),
  cyan: wrapCode(36, 39),
};

// Unicode is fine in every terminal this targets, but a dumb terminal gets
// ASCII so the output never turns into replacement characters.
const fancy = ON || process.env.TERM !== "dumb";
const sym = {
  ok: fancy ? "✔" : "+",
  skip: fancy ? "–" : "-",
  fail: fancy ? "✖" : "x",
  work: fancy ? "⋯" : ".",
  dot: fancy ? "·" : "-",
  arrow: fancy ? "›" : ">",
};

const width = () => Math.max(40, Math.min(stream.columns || 80, 100));

// Visible length: escape codes occupy no columns.
const visible = (s) => String(s).replace(/\x1b\[[0-9;]*m/g, "").length;

// Wrap on spaces, then hang every continued line under `indent`. Long detail
// strings are the main thing that made this output look ragged.
function wrapText(text, indent, max) {
  const limit = (max || width()) - indent;
  const out = [];
  for (const paragraph of String(text).split("\n")) {
    let line = "";
    for (const word of paragraph.split(/\s+/).filter(Boolean)) {
      if (!line.length) line = word;
      else if (visible(line) + 1 + visible(word) <= limit) line += " " + word;
      else { out.push(line); line = word; }
    }
    out.push(line);
  }
  return out.map((l) => " ".repeat(indent) + l);
}

const write = (s) => stream.write(s);
const line = (s = "") => write(s + "\n");

// A transient line, only when a real terminal can erase it again.
function transient(s) {
  if (!stream.isTTY) return () => {};
  write(s);
  return () => write("\r\x1b[2K");
}

function header(command, cwd) {
  line();
  line(`  ${c.bold("principia")} ${c.dim(sym.dot)} ${c.cyan(command)}`);
  line(`  ${c.dim(cwd)}`);
  line();
}

// `hint` sits inline after the label. A one-word status ("ready") repeated down
// a column of ten steps is noise as its own line, but useful next to the label.
function step(status, label, detail, hint) {
  const glyph = status === "ok" ? c.green(sym.ok)
    : status === "skip" ? c.yellow(sym.skip)
    : c.red(sym.fail);
  const tail = hint ? `  ${c.dim(hint)}` : "";
  // The label wraps too. Details always did, but a long label used to run
  // straight off the right edge.
  const [first, ...rest] = wrapText(label, 0, width() - 5 - (hint ? hint.length + 2 : 0));
  const paint = (s) => (status === "skip" ? c.dim(s) : s);
  line(`  ${glyph}  ${paint(first)}${rest.length ? "" : tail}`);
  rest.forEach((l, i) => line(`     ${paint(l)}${i === rest.length - 1 ? tail : ""}`));
  if (detail) for (const l of wrapText(detail, 5)) line(c.dim(l));
}

function summary(counts, ms, labels) {
  const l = labels || { done: "done", skipped: "skipped", failed: "failed" };
  const parts = [
    c.green(`${counts.done} ${l.done}`),
    counts.skipped ? c.yellow(`${counts.skipped} ${l.skipped}`) : c.dim(`0 ${l.skipped}`),
  ];
  if (counts.failed != null) {
    parts.push(counts.failed ? c.red(`${counts.failed} ${l.failed}`) : c.dim(`0 ${l.failed}`));
  }
  line();
  const took = ms == null ? "" : c.dim(`  ${sym.dot}  ${(ms / 1000).toFixed(1)}s`);
  line(`  ${parts.join(c.dim(`  ${sym.dot}  `))}${took}`);
}

function section(title, body) {
  line();
  line(`  ${c.bold(title)}`);
  line();
  for (const l of wrapText(body, 2)) line(c.dim(l));
}

// A block meant to be copied. The gutter marks where it starts and ends
// without relying on colour, which a pipe or NO_COLOR would strip.
function quote(text) {
  const bar = c.dim(fancy ? "│" : "|");
  line();
  for (const l of wrapText(text, 0, width() - 6)) line(`    ${bar}  ${c.bold(l)}`);
}

function note(text) {
  line();
  for (const l of wrapText(text, 2)) line(c.dim(l));
}

module.exports = { c, sym, line, write, header, step, summary, section, quote, note, transient, wrapText, width };
