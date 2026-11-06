#!/bin/sh
# Installs a yttt relay node and enrolls it with the control plane at
# __CONTROL_URL__.
#
# The deploy command checked this script against the hash it was generated
# with. The relay binary is checked against SHA256SUMS from the same server,
# which only catches a truncated download: the trust in both comes from
# HTTPS to the control plane.
#
#   YTTT_ENROLL=<token> sh relay.sh [--domain relay.example.com] [--public-url URL]
#                                   [--binary-url URL] [--no-systemd]
#   sh relay.sh --upgrade      replace the binary, keep the enrollment
#   sh relay.sh --uninstall    stop and remove the node from this machine
#
# The relay only forwards traffic that is end-to-end encrypted between a
# user's devices; nothing installed here can reach those devices.

set -eu

CONTROL='__CONTROL_URL__'
PREFIX=/usr/local/bin
CONFIG_DIR=/etc/yttt-relay
UNIT=/etc/systemd/system/yttt-relay.service

die() {
    echo "yttt-relay install: $*" >&2
    exit 1
}

say() {
    echo "==> $*"
}

main() {
    domain=''
    public_url=''
    binary_url=''
    no_systemd=0
    mode=install
    while [ $# -gt 0 ]; do
        case "$1" in
            --domain) [ $# -ge 2 ] || die "--domain needs a value"; domain=$2; shift 2 ;;
            --public-url) [ $# -ge 2 ] || die "--public-url needs a value"; public_url=$2; shift 2 ;;
            --binary-url) [ $# -ge 2 ] || die "--binary-url needs a value"; binary_url=$2; shift 2 ;;
            --no-systemd) no_systemd=1; shift ;;
            --upgrade) mode=upgrade; shift ;;
            --uninstall) mode=uninstall; shift ;;
            *) die "unknown option $1" ;;
        esac
    done

    [ "$(id -u)" = 0 ] || die "run as root, e.g. with sudo"
    [ "$(uname -s)" = Linux ] || die "relay nodes run on Linux"
    case "$(uname -m)" in
        x86_64 | amd64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) die "unsupported architecture $(uname -m)" ;;
    esac

    if [ "$mode" = uninstall ]; then
        uninstall
        return
    fi
    command -v curl >/dev/null || die "curl is required"
    command -v sha256sum >/dev/null || die "sha256sum is required"

    fetch_binary "$arch" "$binary_url"
    if [ "$mode" = upgrade ]; then
        [ -f "$CONFIG_DIR/relay.toml" ] || die "this machine has no enrolled node to upgrade"
        if [ -f "$UNIT" ]; then
            systemctl restart yttt-relay
        fi
        say "upgraded to $("$PREFIX/yttt-relay" --version 2>/dev/null || echo 'the new version')"
        return
    fi

    [ -n "${YTTT_ENROLL:-}" ] || die "YTTT_ENROLL is not set; copy the deploy command from the console again"
    if [ -n "$domain" ]; then
        if command -v getent >/dev/null && ! getent hosts "$domain" >/dev/null; then
            die "$domain does not resolve; point its DNS at this machine first"
        fi
    fi

    mkdir -p "$CONFIG_DIR"
    chmod 755 "$CONFIG_DIR"
    say "enrolling with $CONTROL"
    # The token goes over stdin, never onto a command line.
    set -- --control "$CONTROL" --config-dir "$CONFIG_DIR"
    [ -n "$domain" ] && set -- "$@" --domain "$domain"
    [ -n "$public_url" ] && set -- "$@" --public-url "$public_url"
    printf '%s' "$YTTT_ENROLL" | "$PREFIX/yttt-relay" enroll "$@"
    unset YTTT_ENROLL

    if [ "$no_systemd" = 1 ]; then
        say "running in the foreground (--no-systemd)"
        exec "$PREFIX/yttt-relay" --config "$CONFIG_DIR/relay.toml"
    fi
    command -v systemctl >/dev/null || die "systemd is required; rerun with --no-systemd to run in the foreground"
    install_unit
    systemctl daemon-reload
    systemctl enable --now yttt-relay
    wait_healthy "$domain"
}

fetch_binary() {
    arch=$1
    url=$2
    temporary=$(mktemp -d)
    trap 'rm -rf "$temporary"' EXIT
    if [ -n "$url" ]; then
        say "downloading the relay from $url"
        curl -fsSL "$url" -o "$temporary/yttt-relay"
    else
        name="yttt-relay-linux-$arch"
        say "downloading $name"
        curl -fsSL "$CONTROL/install/bin/$name" -o "$temporary/yttt-relay"
        curl -fsSL "$CONTROL/install/bin/SHA256SUMS" -o "$temporary/SHA256SUMS"
        expected=$(awk -v name="$name" '$2 == name { print $1 }' "$temporary/SHA256SUMS")
        [ -n "$expected" ] || die "the control plane lists no checksum for $name"
        actual=$(sha256sum "$temporary/yttt-relay" | awk '{ print $1 }')
        [ "$expected" = "$actual" ] || die "the downloaded relay does not match its checksum"
    fi
    install -m 755 "$temporary/yttt-relay" "$PREFIX/yttt-relay"
}

install_unit() {
    cat >"$UNIT" <<'UNIT_EOF'
[Unit]
Description=yttt relay node
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/usr/local/bin/yttt-relay --config /etc/yttt-relay/relay.toml
# The node token is root-only on disk and handed to the service as a credential.
LoadCredential=node-token:/etc/yttt-relay/node-token
# Certificates from Let's Encrypt persist here across restarts.
StateDirectory=yttt-relay
DynamicUser=yes
AmbientCapabilities=CAP_NET_BIND_SERVICE
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
UNIT_EOF
}

wait_healthy() {
    domain=$1
    port=$(awk -F'[:"]' '/^listen/ { print $(NF-1) }' "$CONFIG_DIR/relay.toml")
    say "waiting for the relay to answer on port $port"
    i=0
    while [ $i -lt 30 ]; do
        if curl -fsS -k "http://127.0.0.1:$port/healthz" >/dev/null 2>&1 \
            || curl -fsS -k "https://127.0.0.1:$port/healthz" >/dev/null 2>&1; then
            break
        fi
        i=$((i + 1))
        sleep 1
    done
    [ $i -lt 30 ] || echo "warning: the relay did not answer yet; see: journalctl -u yttt-relay" >&2
    url=$(awk -F'"' '/^# url/ { print $2 }' "$CONFIG_DIR/relay.toml")
    say "the node is enrolled as ${url:-this machine}"
    if [ -n "$domain" ]; then
        echo "    Open TCP 443 and 80 and UDP 7842 to this machine."
        echo "    The certificate is requested in the background; the node serves once it arrives."
    else
        echo "    Open TCP $port to this machine."
    fi
    echo "    The console shows the node online within a minute."
}

uninstall() {
    if [ -f "$UNIT" ]; then
        systemctl disable --now yttt-relay || true
        rm -f "$UNIT"
        systemctl daemon-reload || true
    fi
    rm -f "$PREFIX/yttt-relay"
    rm -rf "$CONFIG_DIR" /var/lib/yttt-relay /var/lib/private/yttt-relay
    say "removed; disable or delete the node in the console too"
}

main "$@"
