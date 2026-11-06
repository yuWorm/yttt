# Builds the control plane with its web console, and the relay node: a
# dynamically linked one for this image and a static musl one that the
# control plane hands to new nodes through its install script.
#   docker build -f packaging/relay-dev/Dockerfile -t yttt-relay-dev .
FROM node:24.18.0-bookworm-slim AS console
WORKDIR /console
COPY web/control-console/package.json web/control-console/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/control-console/ ./
RUN npm run build

FROM rust:1.98-bookworm AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
# Static relay for this machine's architecture; build the image on each
# architecture you deploy nodes on, or cross-compile with cargo-zigbuild.
RUN arch="$(uname -m)" \
    && rustup target add "$arch-unknown-linux-musl" \
    && cargo build --locked --release -p yttt-relay -p yttt-control \
    && CC=musl-gcc cargo build --locked --release -p yttt-relay --target "$arch-unknown-linux-musl" \
    && mkdir -p /out/bin \
    && cp "target/$arch-unknown-linux-musl/release/yttt-relay" "/out/bin/yttt-relay-linux-$arch" \
    && cd /out/bin && sha256sum yttt-relay-linux-* > SHA256SUMS

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/yttt-relay /src/target/release/yttt-control /usr/local/bin/
COPY --from=build /out/bin/ /usr/share/yttt-control/bin/
COPY --from=console /console/dist/ /usr/share/yttt-control/web/
USER nobody
