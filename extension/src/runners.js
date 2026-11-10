// Re-export only. The runner table lives at shared/runners.js so the extension
// and the CLI cannot drift apart; see the note at the top of that file.
module.exports = require("../../shared/runners");
