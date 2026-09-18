# Tirith — MCP coordination server for parallel coding agents.
#
# Builds the `tirith` binary and runs the stdio shim, which is what an MCP
# client speaks to. The shim starts the repository daemon on first use and
# proxies to it, so the container answers `initialize` and `tools/list`
# with nothing else running.
#
#   docker build -t tirith .
#   docker run --rm -i tirith
#
# Mount a repository at /workspace to coordinate real work:
#   docker run --rm -i -v "$PWD:/workspace" tirith

FROM rust:slim-bookworm AS build

WORKDIR /src
COPY . .

# The `tray` default feature is the macOS menu bar icon and compiles to
# nothing off macOS; a container has no menu bar, so leave it out.
RUN cargo build --release --locked --no-default-features --bin tirith

FROM debian:bookworm-slim

# ca-certificates so `tirith update` and the HTTP client have a trust store.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/tirith /usr/local/bin/tirith

# Unprivileged, with a writable repository root and state directory.
RUN useradd --create-home --uid 10001 tirith \
 && mkdir -p /workspace /home/tirith/.local/state/tirith \
 && chown -R tirith:tirith /workspace /home/tirith

USER tirith

# `--root` defaults to the working directory; .tirith/ is written there.
WORKDIR /workspace

ENV TIRITH_NO_TRAY=1 \
    TIRITH_STATE_DIR=/home/tirith/.local/state/tirith

# stdio is the transport MCP clients and introspection use.
ENTRYPOINT ["tirith", "stdio"]
