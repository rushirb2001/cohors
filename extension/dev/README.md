# Working on the extension

Nothing here ships: `.vscodeignore` keeps `dev/` out of the packaged extension.

## See the dashboard without launching VS Code

```sh
cd extension
python3 -m http.server 8000
# open http://localhost:8000/dev/preview.html
```

`preview.html` loads the real `media/main.css`, `media/view.js` and
`media/main.js`, defines the `--vscode-*` variables a theme would, stubs
`acquireVsCodeApi`, and answers the webview's messages from `fixtures/` the way
the extension host does. The bar at the top is the harness, not the extension:

- **Theme** cycles Dark Modern, Light Modern and an approximate Dark High
  Contrast.
- **Scenario** switches the state the host reports: the demo fleet, no
  repositories, cohors not installed, a wrong `cohors.binaryPath`, a failed
  scan, a cohors with no `agent` command, no runner installed, an untrusted
  workspace, and still loading.
- The grey text shows the last message the webview sent to the host.

Query parameters set the starting state, which is handy for screenshots:
`?theme=light&page=tasks`, `?page=repos&sel=demo/payments`,
`?scenario=missing&bare` (`bare` hides the harness bar).

## Check it from Node

```sh
node dev/check.js                                   # fixtures and the stand-in binary
node dev/check.js --bin ../target/debug/cohors --root ~/code   # also a real cohors
```

It checks the wiring (every control has a handler, every message a receiver,
`package.json` matches the code), renders every page in every scenario and
verifies the HTML, and runs `src/cohors.js` and `src/agent.js` against
`fake-cohors.js`. With `--bin` it also runs the adapter against a real binary
and prints counts only.

## Run it in VS Code

```sh
code --extensionDevelopmentPath="$PWD/extension"
```

To drive it with the demo fleet instead of your own repositories, set
`cohors.binaryPath` to the absolute path of `dev/fake-cohors.js`.

## Where the fixtures come from

`fixtures/` is cohors's built-in demo fleet, so no real repository is in this
directory. The CLI has no demo mode for `overview --json`, so it is captured
from the demo web server, which serves the same model, and rebuilt with:

```sh
cohors web --demo --no-open --port 8787            # in the cohors checkout
curl -s localhost:8787/api/overview  > /tmp/overview.json
curl -s localhost:8787/api/launchpad > /tmp/launchpad.json
node dev/capture-fixtures.js /tmp/overview.json /tmp/launchpad.json
```

`capture-fixtures.js` then does two things, both spelled out in its comments.

**Normalised**, so the files look like real `cohors … --json` output:

- repository paths are absolute under `/home/dev/code`, and `roots` is added;
- launchpad entries use the same ids as the overview (the demo launchpad says
  `payments` where the demo fleet says `demo/payments`; real output always
  agrees), which also fills in each repository's `tasks_open`, `tasks_high`
  and `configured`;
- each commit gets its own short id and each remote a fetch time (the demo
  fleet repeats one id and has no fetch times);
- the launchpad is reshaped from the web server's `{entries, board}` to the
  CLI's `{summary, repos, board, recent_activity}`, with an entry for every
  repository.

**Invented**, because the demo fleet has nothing there and the UI needs
something to draw:

- one declared agent, one link, one finished task, task notes, a second flow
  and a port;
- detected scripts for the repositories with no launchpad file;
- six agent-activity events;
- both standup files (the demo fleet has no history to read);
- `agent.json` (runners and prompts, in the shape of the `cohors agent`
  contract), `local.json` (a task's linked conversation and two sessions),
  `devices.json` and `host.json` (what the extension host adds).

## The mark

`node dev/make-mark.js` rewrites `media/cohors*.svg` from the block rows in
`media/view.js`. `check.js` fails if they drift apart.
