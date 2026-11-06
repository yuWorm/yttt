# Builds yttt-relay and yttt-control from this repository.
#   docker build -f packaging/relay-dev/Dockerfile -t yttt-relay-dev .
FROM rust:1.98-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p yttt-relay -p yttt-control

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/yttt-relay /src/target/release/yttt-control /usr/local/bin/
USER nobody
