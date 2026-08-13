FROM debian:bookworm-slim

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
        git \
        openssh-server \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /bin/bash yttt \
    && printf 'yttt:yttt-smoke\n' | chpasswd \
    && mkdir -p /run/sshd \
    && ssh-keygen -A

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
