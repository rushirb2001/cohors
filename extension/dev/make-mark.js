#!/usr/bin/env node
// Writes the icon files from the block rows in media/view.js, so the mark in
// the activity bar, on the editor tab and inside the dashboard is one drawing.
//
//   node dev/make-mark.js

const fs = require("fs");
const path = require("path");
const V = require("../media/view.js");

const media = path.join(__dirname, "..", "media");
const head = "<!-- The cohors mark. Generated from the block rows in media/view.js by dev/make-mark.js. -->\n";
// 18x12 drawing centred on a square canvas, which is what an icon slot wants.
const square = "0 -3 18 18";
const write = (name, fill) =>
  fs.writeFileSync(path.join(media, name), head + V.markSvg({ xmlns: true, cls: false, viewBox: square, fill }) + "\n");

write("cohors.svg", undefined);          // activity bar: VS Code masks it, so one colour
write("cohors-dark.svg", "#c3a2bb");     // editor tab, dark themes
write("cohors-light.svg", "#7d5a75");    // editor tab, light themes
console.log("wrote media/cohors.svg, cohors-dark.svg, cohors-light.svg");
