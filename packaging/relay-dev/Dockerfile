# Builds the control plane with its web console, and the relay node: a
# dynamically linked one for this image, and static musl ones for x86_64 and
# aarch64 that the control plane hands to new nodes through its install
# script.
#   docker build -f packaging/relay-dev/Dockerfile -t yttt-relay-dev .
FROM node:24.18.0-bookworm-slim AS console
WORKDIR /console
COPY web/control-console/package.json web/control-console/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/control-console/ ./
RUN npm run build

FROM rust:1.98-bookworm AS build
# The static relay for this machine's architecture links with musl-gcc; Zig
# cross-links the other one, so one build serves x86_64 and aarch64 nodes.
ARG ZIG_VERSION=0.13.0
RUN apt-get update \
    && apt-get install -y --no-install-recommends xz-utils musl-tools \
    && rm -rf /var/lib/apt/lists/* \
    && curl -fsSL "https://ziglang.org/download/${ZIG_VERSION}/zig-linux-$(uname -m)-${ZIG_VERSION}.tar.xz" \
        | tar -xJ -C /opt \
    && ln -s "/opt/zig-linux-$(uname -m)-${ZIG_VERSION}/zig" /usr/local/bin/zig \
    && cargo install --locked cargo-zigbuild@0.19.8
WORKDIR /src
COPY . .
# Targets go to the toolchain rust-toolchain.toml selects inside the source.
RUN rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl \
    && cargo build --locked --release -p yttt-relay -p yttt-control \
    && mkdir -p /out/bin \
    && native="$(uname -m)" \
    && for arch in x86_64 aarch64; do \
        if [ "$arch" = "$native" ]; then \
            CC=musl-gcc cargo build --locked --release -p yttt-relay --target "$arch-unknown-linux-musl"; \
        else \
            cargo zigbuild --locked --release -p yttt-relay --target "$arch-unknown-linux-musl"; \
        fi \
        && cp "target/$arch-unknown-linux-musl/release/yttt-relay" "/out/bin/yttt-relay-linux-$arch" \
        || exit 1; \
    done \
    && cd /out/bin && sha256sum yttt-relay-linux-* > SHA256SUMS

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/yttt-relay /src/target/release/yttt-control /usr/local/bin/
COPY --from=build /out/bin/ /usr/share/yttt-control/bin/
COPY --from=console /console/dist/ /usr/share/yttt-control/web/
USER nobody
