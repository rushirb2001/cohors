#!/bin/sh
# Start the installed `cohors` binary for the plugin.
#
# Why a launcher: an agent starts MCP servers with a stripped PATH, so
# a bare `cohors` often resolves to nothing and the agent silently ends up with
# no tools. This looks in the places every install channel puts the binary,
# each spelled out, and runs the first one found, cohors' own self-updating
# install first.
#
#   sh cohors-launch.sh mcp                  the MCP server (.mcp.json)
#   sh cohors-launch.sh hook session-start   the note at the start of a
#                                            session (hooks/hooks.json)
#
# For `mcp`, it also turns the plugin's settings into the server's flags.
# Claude Code passes each setting in a CLAUDE_PLUGIN_OPTION_* variable (see
# .mcp.json); only the word "true" switches one on, so an unset or unfilled
# value, such as from a Claude Code too old to fill it in, leaves the server
# read-only.
#
# `hook` must never get in the way of a session. Claude Code waits for a hook
# before it answers, and shows any non-zero exit as an error. So in hook mode
# this script prints nothing to stderr and always ends with exit 0, whatever
# the binary did: no cohors installed, or one too old to know `hook`, is
# silence, not an error. stdin and stdout go straight to the binary, which
# reads Claude Code's JSON and prints its answer. The setting "Session notes"
# turns the hook off; only the word "false" does, so an unset or unfilled
# value leaves it on, which is the default.

if [ "${1:-}" = mcp ]; then
  [ "${CLAUDE_PLUGIN_OPTION_GIT_ACTIONS:-}" = true ] && set -- "$@" --allow-writes
  [ "${CLAUDE_PLUGIN_OPTION_RUN_COMMANDS:-}" = true ] && set -- "$@" --allow-run
  [ "${CLAUDE_PLUGIN_OPTION_LAUNCHPAD_WRITES:-}" = true ] && set -- "$@" --allow-launchpad
  # Tells the server it was started by the plugin, so a refusal names the
  # plugin setting to turn on rather than a command-line flag.
  export COHORS_MCP_FROM_PLUGIN=1
fi

hook_mode=false
if [ "${1:-}" = hook ]; then
  hook_mode=true
  if [ "${2:-}" = session-start ] && [ "${CLAUDE_PLUGIN_OPTION_SESSION_NOTES:-}" = false ]; then
    exit 0
  fi
fi

# Runs the command it is given (the binary and its arguments) in place of this
# script. In hook mode it runs it as a child instead, with every message the
# binary or the shell would print to stderr thrown away, then exits 0. The
# braces matter: they also hide the shell's own "Killed" line, which a
# binary replaced while it starts can cause.
launch() {
  if [ "$hook_mode" = true ]; then
    { "$@"; } 2>/dev/null
    exit 0
  fi
  exec "$@"
}

# cohors' own install wins over every other copy, because it updates itself:
# `cohors install` keeps each release under ~/.local/share/cohors/versions
# and the link ~/.local/bin/cohors (or $XDG_BIN_HOME/cohors) on the active
# one. A Homebrew or Cargo copy found first on the PATH would be stuck on
# whatever version it was installed at. Only that link counts, not any file
# at that place: a pipx or uv tool link there is a copy like the others.
managed_bin="$HOME/.local/bin"
case "${XDG_BIN_HOME:-}" in
  /*) managed_bin="$XDG_BIN_HOME" ;; # only an absolute path counts, as in XDG
esac
if [ -L "$managed_bin/cohors" ] && [ -x "$managed_bin/cohors" ]; then
  case "$(readlink "$managed_bin/cohors")" in
    */cohors/versions/*) launch "$managed_bin/cohors" "$@" ;;
  esac
fi

# Otherwise the first install found wins: on the PATH, then each channel's
# own place.
if command -v cohors >/dev/null 2>&1; then launch cohors "$@"; fi
if [ -x "$HOME/.cargo/bin/cohors" ]; then launch "$HOME/.cargo/bin/cohors" "$@"; fi
if [ -x /opt/homebrew/bin/cohors ]; then launch /opt/homebrew/bin/cohors "$@"; fi
if [ -x /usr/local/bin/cohors ]; then launch /usr/local/bin/cohors "$@"; fi
if [ -x /home/linuxbrew/.linuxbrew/bin/cohors ]; then launch /home/linuxbrew/.linuxbrew/bin/cohors "$@"; fi
if [ -x "$HOME/.local/bin/cohors" ]; then launch "$HOME/.local/bin/cohors" "$@"; fi
if [ -x "$HOME/.nix-profile/bin/cohors" ]; then launch "$HOME/.nix-profile/bin/cohors" "$@"; fi

# None found. A hook says nothing; the message below is for the MCP server,
# where it is the only way to learn why there are no tools.
if [ "$hook_mode" = true ]; then exit 0; fi

cat >&2 <<'MSG'
cohors: the `cohors` program is not installed, so this plugin has no tools yet.

Install it with Homebrew (brew install rushirb2001/tap/cohors) or pip
(pip install cohors); every way to install it is listed at
https://github.com/rushirb2001/cohors#install

Then start a new session.
MSG
exit 127
