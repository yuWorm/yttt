# A disposable Linux machine with systemd, for rehearsing node deployment.
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends systemd systemd-sysv curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*
STOPSIGNAL SIGRTMIN+3
CMD ["/lib/systemd/systemd"]
