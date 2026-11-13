# cohors as a container, mainly to run its MCP server for an agent or a
# registry (Glama) that starts servers in Docker.
#
# It installs the released Linux binary rather than compiling, so a build takes
# seconds and the image matches what every other channel ships. The release
# checksum is verified before anything is installed.
#
#   docker run -i --rm -v "$HOME/code:/repos" ghcr.io/rushirb2001/cohors
#
# serves every repo under ~/code over MCP (stdio), read-only. Any other cohors
# command works too: `docker run -it --rm -v "$HOME/code:/repos" \
#   ghcr.io/rushirb2001/cohors --root /repos` opens the dashboard.
FROM debian:bookworm-slim

# A release version without the "v" (e.g. 0.7.6), or "latest".
ARG COHORS_VERSION=latest
# Set by `docker buildx` for each platform; a plain `docker build` leaves it
# empty, and the machine's own architecture is used instead.
ARG TARGETARCH

LABEL org.opencontainers.image.title="cohors" \
      org.opencontainers.image.description="Every git repo you own, on one screen: a terminal dashboard and an MCP server for your coding agent" \
      org.opencontainers.image.source="https://github.com/rushirb2001/cohors" \
      org.opencontainers.image.licenses="MIT" \
      io.modelcontextprotocol.server.name="io.github.rushirb2001/cohors"

# git: cohors's write actions (fetch, pull, push) run the git CLI.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl git xz-utils \
 && rm -rf /var/lib/apt/lists/*

RUN set -eu; \
    arch="${TARGETARCH:-$(dpkg --print-architecture)}"; \
    case "$arch" in \
      amd64) target=x86_64-unknown-linux-gnu ;; \
      arm64) target=aarch64-unknown-linux-gnu ;; \
      *) echo "cohors has no Linux build for $arch" >&2; exit 1 ;; \
    esac; \
    if [ "$COHORS_VERSION" = latest ]; then \
      base=https://github.com/rushirb2001/cohors/releases/latest/download; \
    else \
      base="https://github.com/rushirb2001/cohors/releases/download/v$COHORS_VERSION"; \
    fi; \
    cd /tmp; \
    curl -fsSLO "$base/cohors-$target.tar.xz"; \
    curl -fsSLO "$base/cohors-$target.tar.xz.sha256"; \
    sha256sum -c "cohors-$target.tar.xz.sha256"; \
    tar -xJf "cohors-$target.tar.xz"; \
    install -m 0755 "cohors-$target/cohors" /usr/local/bin/cohors; \
    rm -rf /tmp/cohors-*; \
    cohors --version

# Repositories mounted from the host belong to another user, and git refuses
# to work in a repository it doesn't own unless told it's safe.
RUN git config --system --add safe.directory '*'

# Mount the folder that holds your repositories here.
RUN mkdir /repos
WORKDIR /repos

ENTRYPOINT ["cohors"]
CMD ["mcp", "--root", "/repos"]
