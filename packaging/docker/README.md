# cohors on Docker: `ghcr.io/rushirb2001/cohors`

The cohors MCP server in a container. Mount the folder that holds your git repositories at `/repos`, and your coding agent sees every repo's status, a search across all of them, recent commits and each repo's tasks, without installing anything else. Built for linux/amd64 and linux/arm64; it runs the release's own Linux binary, checked against the release checksum when the image is built.

## Run the MCP server

```sh
docker run -i --rm -v "$HOME/code:/repos" ghcr.io/rushirb2001/cohors
```

That is `cohors mcp --root /repos`: a stdio MCP server, read-only. Keep `-i` (the client talks to it over stdin) and leave out `-t`.

**Claude Code**

```sh
claude mcp add cohors -- "$(which docker)" run -i --rm -v "$HOME/code:/repos" ghcr.io/rushirb2001/cohors
```

**Claude Desktop, Cursor and other clients** (`mcpServers`; use absolute paths, since clients start servers with a stripped `PATH` and don't expand `~` or `$HOME`)

```json
{
  "mcpServers": {
    "cohors": {
      "command": "/usr/local/bin/docker",
      "args": ["run", "-i", "--rm", "-v", "/Users/you/code:/repos", "ghcr.io/rushirb2001/cohors"]
    }
  }
}
```

`which docker` prints the path for your machine.

## Options

Arguments after the image name replace the default `mcp --root /repos`, so repeat those when you add flags:

```sh
docker run -i --rm -v "$HOME/code:/repos" ghcr.io/rushirb2001/cohors mcp --root /repos --allow-launchpad
```

| Flag | Turns on (all off by default) |
|---|---|
| `--allow-writes` | `fetch`, `pull`, `push`, `commit`, `stash`. Pull is fast-forward only and push never forces. |
| `--allow-run` | One shell command in many repos at once, capped by the config's run allowlist. |
| `--allow-launchpad` | Writing a repo's `.cohors/repo.json`, the focus board and the activity log. Never git. |

The image has no SSH client and none of your git credentials, so `fetch`, `pull` and `push` reach only remotes that need neither. For git actions against your real remotes, run a native install (`brew install rushirb2001/tap/cohors`, `pip install cohors`).

**More than one folder.** Mount each and name each:

```sh
docker run -i --rm -v "$HOME/work:/repos/work" -v "$HOME/oss:/repos/oss" \
  ghcr.io/rushirb2001/cohors mcp --root /repos/work --root /repos/oss
```

**CI and pull-request status.** Pass a GitHub token through: add `-e GITHUB_TOKEN` to the `docker run` arguments, and set `GITHUB_TOKEN` in the client's `env`.

**The dashboard.** Any cohors command runs, with a terminal for the interactive ones:

```sh
docker run -it --rm -v "$HOME/code:/repos" ghcr.io/rushirb2001/cohors --root /repos
docker run --rm -v "$HOME/code:/repos" ghcr.io/rushirb2001/cohors overview --root /repos
```

Without `-t`, reports print JSON, ready for `jq`.

## Tags and updates

| Tag | Follows |
|---|---|
| `latest` | the newest release |
| `0.9` | the newest 0.9.x |
| `0.9.2` | exactly that release |

The container never updates itself. `docker pull ghcr.io/rushirb2001/cohors` gets the newest `latest`; pin an exact tag for a setup that should not move.

## New in 0.9

Answers are brief by default and several times smaller (`detail: "full"` gives every field). Work a coding agent left in a linked worktree is reported with its repository: it counts as work at risk and has a step of its own in `overview`. When the mounted folder holds a single repository, its worktrees are listed as rows themselves, each with a `linked` field.

## New in 0.8

The server answers "where did I leave off?" with a `next` tool (the coding-agent conversations to resume and the fleet's next steps), finds repositories nested inside others with a `discover` tool, accepts a repository by a unique prefix (and says "did you mean" for a typo), and briefs on part of the fleet with `overview` and a selector. The image is published with every release, under the tags above. [Release notes](https://github.com/rushirb2001/cohors/releases)

[Documentation](https://github.com/rushirb2001/cohors#readme) · [Dockerfile](https://github.com/rushirb2001/cohors/blob/main/Dockerfile) · [Issues](https://github.com/rushirb2001/cohors/issues) · MIT licensed
