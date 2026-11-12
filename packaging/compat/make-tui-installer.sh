#!/bin/sh
# Writes cohors-tui-installer.sh, attached to every release beside the real
# cohors-installer.sh.
#
# Up to 0.7.2 the binary's crate was named `cohors-tui`, so its installer was
# `cohors-tui-installer.sh`. Those copies of cohors fetch that name to update
# themselves, and older instructions point at it, so each release carries a
# small script under the old name that runs the real installer of the same
# release, carrying the old settings over to their new names.
set -eu
cd "$(dirname "$0")/../.."
version=$(sed -n 's/^version = "\([0-9][^"]*\)"/\1/p' Cargo.toml | head -n 1)
test -n "$version" || { echo "no workspace version in Cargo.toml" >&2; exit 1; }
cat > cohors-tui-installer.sh <<SCRIPT
#!/bin/sh
# cohors' installer is now named cohors-installer.sh. This runs it, for copies
# of cohors and instructions that still use the old name.
set -u
if [ -n "\${COHORS_TUI_INSTALL_DIR:-}" ]; then export COHORS_INSTALL_DIR="\$COHORS_TUI_INSTALL_DIR"; fi
if [ -n "\${COHORS_TUI_NO_MODIFY_PATH:-}" ]; then export COHORS_NO_MODIFY_PATH="\$COHORS_TUI_NO_MODIFY_PATH"; fi
url="https://github.com/rushirb2001/cohors/releases/download/v${version}/cohors-installer.sh"
if command -v curl >/dev/null 2>&1; then
  script=\$(curl --proto '=https' --tlsv1.2 -fsSL "\$url") || { echo "could not download \$url" >&2; exit 1; }
elif command -v wget >/dev/null 2>&1; then
  script=\$(wget -qO- "\$url") || { echo "could not download \$url" >&2; exit 1; }
else
  echo "cohors needs curl or wget to install" >&2; exit 1
fi
printf '%s\n' "\$script" | sh -s -- "\$@"
SCRIPT
chmod +x cohors-tui-installer.sh
