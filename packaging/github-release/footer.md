<!-- cohors:install-footer (added by .github/workflows/release-notes.yml from packaging/github-release/footer.md) -->

## More ways to install cohors {version}

The installers above (shell, Homebrew, npm) and the downloads all come from this release. The same version is also on:

| Where | Install | How it updates |
|---|---|---|
| PyPI | `pip install cohors` | cohors updates itself, as with Homebrew and the shell installer |
| PyPI, with uv | `uvx cohors@{version}` | `uvx cohors@latest` always runs the newest |
| npm, as a command | `npm install -g @cohors/cli` | cohors tells you; run `npm install -g @cohors/cli@latest` |
| crates.io | `cargo install cohors --version {version}` | cohors tells you; run `cargo install cohors` |
| Docker | `docker pull ghcr.io/rushirb2001/cohors:{version}` | Pull `:latest` or the next version tag |
| VS Code, Cursor, Windsurf, VSCodium | Search "cohors" in Extensions, or install the `.vsix` attached here | The editor updates it |
| Claude Code | `/plugin marketplace add rushirb2001/cohors`, then `/plugin install cohors@cohors` | `claude plugin update cohors@cohors` |

The VS Code extension and the Claude Code plugin both drive the `cohors` command, so install it one of the ways above first.

**As an MCP server**, register it with the absolute path (agents start servers with a stripped `PATH`): `claude mcp add cohors -- "$(which cohors)" mcp`. It starts read-only; `--allow-writes`, `--allow-run` and `--allow-launchpad` turn on git actions, commands across repos and launchpad writes.
