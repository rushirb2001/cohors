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
| `0.7` | the newest 0.7.x |
| `0.7.7` | exactly that release |

The container never updates itself. `docker pull ghcr.io/rushirb2001/cohors` gets the newest `latest`; pin an exact tag for a setup that should not move.

## New in 0.7.7

From 0.7.7 the image is published with every release, under the tags above. cohors 0.7.7 also finds repos nested inside other repos, runs git actions on every selected repo at once, and refuses a commit that would include a file that looks like a secret. [Release notes](https://github.com/rushirb2001/cohors/releases)

[Documentation](https://github.com/rushirb2001/cohors#readme) · [Dockerfile](https://github.com/rushirb2001/cohors/blob/main/Dockerfile) · [Issues](https://github.com/rushirb2001/cohors/issues) · MIT licensed
