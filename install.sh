#!/bin/sh
# cohors installer — downloads the prebuilt binary for this machine from the
# latest GitHub release and puts it on your PATH. No Rust required.
#
#   curl -fsSL https://raw.githubusercontent.com/rushirb2001/cohors/main/install.sh | sh
#
# Options (environment variables):
#   COHORS_VERSION   install a specific tag (e.g. v0.5.0) instead of the latest
#   COHORS_BIN_DIR   install directory (default: /usr/local/bin if writable,
#                    otherwise ~/.local/bin)
set -eu

repo="rushirb2001/cohors"

# ── Pick the release asset for this OS + architecture ────────────────────────
os=$(uname -s)
arch=$(uname -m)
case "$os" in
  Darwin)
    case "$arch" in
      arm64) target="aarch64-apple-darwin" ;;
      x86_64) target="x86_64-apple-darwin" ;;
      *) echo "unsupported macOS architecture: $arch" >&2; exit 1 ;;
    esac
    ;;
  Linux)
    case "$arch" in
      x86_64) target="x86_64-unknown-linux-gnu" ;;
      *) echo "unsupported Linux architecture: $arch (build from source: cargo install --git https://github.com/$repo cohors-tui)" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "unsupported OS: $os — on Windows use WSL, or build from source: cargo install --git https://github.com/$repo cohors-tui" >&2
    exit 1
    ;;
esac

version="${COHORS_VERSION:-latest}"
if [ "$version" = "latest" ]; then
  url="https://github.com/$repo/releases/latest/download/cohors-$target.tar.gz"
else
  url="https://github.com/$repo/releases/download/$version/cohors-$target.tar.gz"
fi

# ── Pick the install directory: /usr/local/bin when writable, else ~/.local ──
bin_dir="${COHORS_BIN_DIR:-}"
if [ -z "$bin_dir" ]; then
  if [ -w /usr/local/bin ]; then
    bin_dir="/usr/local/bin"
  else
    bin_dir="$HOME/.local/bin"
  fi
fi
mkdir -p "$bin_dir"

# ── Download and install ─────────────────────────────────────────────────────
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
echo "downloading cohors ($version, $target)…"
curl -fsSL "$url" -o "$tmp/cohors.tar.gz"
tar -xzf "$tmp/cohors.tar.gz" -C "$tmp"
install -m 755 "$tmp/cohors" "$bin_dir/cohors"

echo "installed: $("$bin_dir/cohors" --version) → $bin_dir/cohors"

# PATH hint when the chosen directory isn't reachable.
case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *)
    echo
    echo "note: $bin_dir is not on your PATH. Add this to your shell profile:"
    echo "  export PATH=\"$bin_dir:\$PATH\""
    ;;
esac

echo
echo "get started:"
echo "  cohors init   # auto-detect where your repos live"
echo "  cohors        # launch the dashboard"
