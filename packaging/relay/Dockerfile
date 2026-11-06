# The published control-plane image, assembled from binaries the release
# workflow already built natively for each architecture: nothing compiles
# here. The image serves the console and hands both static relay binaries to
# new nodes through its install script; it can also run a node itself.
#
#   image/control-<amd64|arm64>   static yttt-control
#   image/relay-<amd64|arm64>     static yttt-relay
#   image/bin/                    yttt-relay-linux-{x86_64,aarch64}, SHA256SUMS
#   image/web/                    the built console
FROM debian:bookworm-slim
ARG TARGETARCH
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY image/control-${TARGETARCH} /usr/local/bin/yttt-control
COPY image/relay-${TARGETARCH} /usr/local/bin/yttt-relay
COPY image/bin/ /usr/share/yttt-control/bin/
COPY image/web/ /usr/share/yttt-control/web/
USER nobody
EXPOSE 8080
ENTRYPOINT ["yttt-control"]
CMD ["--config", "/etc/yttt/control.toml"]
