// Re-export only. The runner table lives at shared/runners.js so the CLI and
// the extension cannot drift apart; see the note at the top of that file.
// The CLI uses just detect(); the rest is here so both sides see one table.
module.exports = require("../../shared/runners");
