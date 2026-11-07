#!/bin/sh
# Installs a yttt relay node and enrolls it with the control plane at
# __CONTROL_URL__.
#
# The deploy command checked this script against the hash it was generated
# with. The relay binary is checked against SHA256SUMS from the same server,
# which only catches a truncated download: the trust in both comes from
# HTTPS to the control plane.
#
#   YTTT_ENROLL=<token> sh relay.sh [--method native|docker] [--domain relay.example.com]
#                                   [--public-url URL] [--binary-url URL] [--image IMAGE]
#                                   [--no-systemd] [--yes]
#                                   [--behind-proxy [--listen ADDRESS:PORT]]
#   With --behind-proxy, a reverse proxy on this machine (the control plane
#   installer's Caddy) serves --domain over HTTPS: the relay listens on
#   127.0.0.1:3340, or --listen ADDRESS:PORT for a proxy that refuses
#   loopback upstreams, and QUIC uses the certificate in /etc/yttt-relay/quic.
#
#   yttt-relay-install --upgrade      replace the binary or image, keep the enrollment
#   yttt-relay-install --uninstall    stop and remove the node from this machine
#
# On a terminal it asks how to run the node (directly with systemd, or in
# Docker) and for its domain; options answer those, and --yes or no terminal
# takes the defaults: directly, without a domain.
#
# Installing keeps a copy of this script as yttt-relay-install, since the
# downloaded one may not survive a reboot.
#
# The relay only forwards traffic that is end-to-end encrypted between a
# user's devices; nothing installed here can reach those devices.

set -eu

CONTROL='__CONTROL_URL__'
IMAGE_DEFAULT='__IMAGE__'
PREFIX=/usr/local/bin
CONFIG_DIR=/etc/yttt-relay
STATE_DIR=/var/lib/yttt-relay
UNIT=/etc/systemd/system/yttt-relay.service
CONTAINER=yttt-relay
# For testing beside a real node: every path under one directory and a
# container of its own. Docker only.
if [ -n "${YTTT_RELAY_ROOT:-}" ]; then
    PREFIX=$YTTT_RELAY_ROOT/bin
    CONFIG_DIR=$YTTT_RELAY_ROOT/etc
    STATE_DIR=$YTTT_RELAY_ROOT/state
    CONTAINER=yttt-relay-test
    mkdir -p "$PREFIX"
fi

die() {
    echo "yttt-relay install: $*" >&2
    exit 1
}

say() {
    echo "==> $*"
}

# Whether to ask on the terminal.
interactive() {
    [ "$yes" = 0 ] && { : </dev/tty; } 2>/dev/null
}

ask_method() {
    [ -n "$method" ] && return
    method=native
    interactive || return 0
    echo "How should the relay run?" >/dev/tty
    echo "  * 1) Directly on this server, as a systemd service" >/dev/tty
    echo "    2) In Docker (host networking)" >/dev/tty
    printf 'Choose [1]: ' >/dev/tty
    read -r answer </dev/tty || answer=''
    case "$answer" in
        2 | docker) method=docker ;;
    esac
}

ask_domain() {
    [ -n "$domain" ] && return
    interactive || return 0
    echo "A domain gives the node HTTPS with a Let's Encrypt certificate, and lets" >/dev/tty
    echo "devices find direct paths. Without one it only relays." >/dev/tty
    printf 'Domain pointing at this server (empty for none): ' >/dev/tty
    read -r domain </dev/tty || domain=''
}

# The program listening on TCP port $1: empty when the port is free, or
# when there is no ss to tell.
port_owner() {
    command -v ss >/dev/null || return 0
    line=$(ss -ltnpH "sport = :$1" 2>/dev/null | head -n 1)
    [ -n "$line" ] || return 0
    name=$(echo "$line" | sed -n 's/.*users:(("\([^"]*\)".*/\1/p')
    echo "${name:-another program}"
}

# Docker's bridge address, which only this machine reaches.
bridge_address() {
    ip -4 -o addr show docker0 2>/dev/null | awk '{ split($4, a, "/"); print a[1]; exit }'
}

# A node with a domain serves HTTPS on 443 and answers certificate
# challenges on 80. When another program holds them, the node goes behind
# it or does without the domain; its own port must be free either way.
check_ports() {
    proxy='the proxy'
    if [ -n "$domain" ] && [ "$behind_proxy" = 0 ]; then
        owner=$(port_owner 443)
        [ -n "$owner" ] || owner=$(port_owner 80)
        case "$owner" in
            '' | yttt-relay) ;;
            *)
                interactive || die "ports 443 and 80 are in use by $owner; rerun with --behind-proxy [--listen ADDRESS:PORT] to run behind it, or without --domain"
                proxy=$owner
                echo "Ports 443 and 80 are in use by $owner, so the node cannot serve $domain itself." >/dev/tty
                echo "  * 1) Behind $owner: it serves $domain over HTTPS and forwards to the node" >/dev/tty
                echo "    2) Without a domain, on port 3341: the node only relays and devices find no direct paths" >/dev/tty
                printf 'Choose [1]: ' >/dev/tty
                read -r answer </dev/tty || answer=''
                case "$answer" in
                    2) domain='' ;;
                    *) behind_proxy=1 ;;
                esac
                ;;
        esac
    fi
    if [ "$behind_proxy" = 1 ] && [ -z "$listen" ]; then
        # SafeLine refuses 127.0.0.1 as an upstream; Docker's bridge is as private.
        bridge=$(bridge_address)
        listen=127.0.0.1:3340
        if [ -n "$bridge" ] && docker ps --format '{{.Names}}' 2>/dev/null | grep -q '^safeline-'; then
            listen=$bridge:3340
        fi
        if interactive; then
            echo "The node listens where $proxy forwards to. SafeLine and some other proxies" >/dev/tty
            [ -z "$bridge" ] || echo "refuse 127.0.0.1; for them use Docker's bridge, $bridge:3340." >/dev/tty
            printf 'Listen on [%s]: ' "$listen" >/dev/tty
            read -r answer </dev/tty || answer=''
            [ -z "$answer" ] || listen=$answer
        fi
    fi
    if [ "$behind_proxy" = 1 ]; then
        port=${listen##*:}
    elif [ -n "$domain" ]; then
        return 0
    else
        port=3341
    fi
    owner=$(port_owner "$port")
    case "$owner" in
        '' | yttt-relay) ;;
        *) die "port $port is in use by $owner" ;;
    esac
}

main() {
    domain=''
    public_url=''
    binary_url=''
    image=''
    method=''
    no_systemd=0
    behind_proxy=0
    listen=''
    yes=0
    mode=install
    while [ $# -gt 0 ]; do
        case "$1" in
            --domain) [ $# -ge 2 ] || die "--domain needs a value"; domain=$2; shift 2 ;;
            --public-url) [ $# -ge 2 ] || die "--public-url needs a value"; public_url=$2; shift 2 ;;
            --binary-url) [ $# -ge 2 ] || die "--binary-url needs a value"; binary_url=$2; shift 2 ;;
            --image) [ $# -ge 2 ] || die "--image needs a value"; image=$2; shift 2 ;;
            --method) [ $# -ge 2 ] || die "--method needs a value"; method=$2; shift 2 ;;
            --no-systemd) no_systemd=1; shift ;;
            --behind-proxy) behind_proxy=1; shift ;;
            --listen) [ $# -ge 2 ] || die "--listen needs a value"; listen=$2; shift 2 ;;
            --yes) yes=1; shift ;;
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

    # An existing node remembers how it was installed.
    installed=$(awk -F= '$1 == "METHOD" { print $2 }' "$CONFIG_DIR/install.env" 2>/dev/null || true)
    if [ "$mode" = uninstall ]; then
        if [ "$installed" = docker ]; then docker_uninstall; else uninstall; fi
        return
    fi
    if [ "$mode" = upgrade ]; then
        [ -f "$CONFIG_DIR/relay.toml" ] || die "this machine has no enrolled node to upgrade"
        if [ "$installed" = docker ]; then docker_upgrade; else upgrade; fi
        return
    fi

    [ -n "${YTTT_ENROLL:-}" ] || die "YTTT_ENROLL is not set; copy the deploy command from the console again"
    ask_method
    case "$method" in native | docker) ;; *) die "--method is native or docker" ;; esac
    ask_domain
    [ "$behind_proxy" = 0 ] || [ -n "$domain" ] || die "--behind-proxy needs the --domain the proxy serves"
    [ -z "$listen" ] || [ "$behind_proxy" = 1 ] || die "--listen is for --behind-proxy"
    check_ports
    if [ -n "$domain" ]; then
        if command -v getent >/dev/null && ! getent hosts "$domain" >/dev/null; then
            die "$domain does not resolve; point its DNS at this machine first"
        fi
    fi
    command -v curl >/dev/null || die "curl is required"
    mkdir -p "$CONFIG_DIR"
    chmod 755 "$CONFIG_DIR"
    if [ "$method" = docker ]; then
        docker_install
    else
        native_install
    fi
    echo "METHOD=$method" >"$CONFIG_DIR/install.env"
    wait_healthy "$domain"
}

native_install() {
    command -v sha256sum >/dev/null || die "sha256sum is required"
    fetch_binary "$arch" "$binary_url"
    keep_installer
    say "enrolling with $CONTROL"
    # The token goes over stdin, never onto a command line.
    set -- --control "$CONTROL" --config-dir "$CONFIG_DIR"
    [ -n "$domain" ] && set -- "$@" --domain "$domain"
    [ -n "$public_url" ] && set -- "$@" --public-url "$public_url"
    [ "$behind_proxy" = 1 ] && set -- "$@" --behind-proxy \
        --quic-cert "$CONFIG_DIR/quic/cert.pem" --quic-key "$CONFIG_DIR/quic/key.pem"
    [ -n "$listen" ] && set -- "$@" --listen "$listen"
    printf '%s' "$YTTT_ENROLL" | "$PREFIX/yttt-relay" enroll "$@"
    unset YTTT_ENROLL

    if [ "$no_systemd" = 1 ]; then
        say "running in the foreground (--no-systemd)"
        exec "$PREFIX/yttt-relay" --config "$CONFIG_DIR/relay.toml"
    fi
    command -v systemctl >/dev/null || die "systemd is required; rerun with --no-systemd to run in the foreground"
    # The service runs as its own user, which may read the token but not change it.
    if ! id yttt-relay >/dev/null 2>&1; then
        useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin yttt-relay
    fi
    chown root:yttt-relay "$CONFIG_DIR/node-token"
    chmod 640 "$CONFIG_DIR/node-token"
    install_unit
    systemctl daemon-reload
    systemctl enable yttt-relay
    # A machine deployed again keeps its service running; restart it so the
    # new enrollment takes effect.
    systemctl restart yttt-relay
}

docker_install() {
    command -v docker >/dev/null || die "Docker is not installed; see https://docs.docker.com/engine/install/"
    image=${image:-$IMAGE_DEFAULT}
    keep_installer
    say "pulling $image"
    pull "$image"
    mkdir -p "$STATE_DIR"
    say "enrolling with $CONTROL"
    set -- --control "$CONTROL" --config-dir "$CONFIG_DIR"
    [ -n "$domain" ] && set -- "$@" --domain "$domain"
    [ -n "$public_url" ] && set -- "$@" --public-url "$public_url"
    [ "$behind_proxy" = 1 ] && set -- "$@" --behind-proxy \
        --quic-cert "$CONFIG_DIR/quic/cert.pem" --quic-key "$CONFIG_DIR/quic/key.pem"
    [ -n "$listen" ] && set -- "$@" --listen "$listen"
    # Host networking, like the node: the control plane may be on this machine.
    printf '%s' "$YTTT_ENROLL" | docker run --rm -i --user 0:0 --network host \
        -v "$CONFIG_DIR:$CONFIG_DIR" --entrypoint yttt-relay "$image" enroll "$@"
    unset YTTT_ENROLL
    chmod 600 "$CONFIG_DIR/node-token"
    echo "IMAGE=$image" >"$CONFIG_DIR/docker.env"
    run_container "$image"
}

# Fetches the image, or uses a copy already on this machine.
pull() {
    docker pull -q "$1" >/dev/null 2>&1 \
        || docker image inspect "$1" >/dev/null 2>&1 \
        || die "could not pull $1"
}

# Starts the node's container, replacing an earlier one.
run_container() {
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    # Host networking: QUIC address discovery needs the machine's own
    # addresses, and the node binds 80/443 when it has a domain. Docker gives
    # added capabilities only to root, so it runs as root with nothing else.
    docker run -d --name "$CONTAINER" --restart unless-stopped --network host \
        --user 0:0 --cap-drop ALL --cap-add NET_BIND_SERVICE \
        -v "$CONFIG_DIR:$CONFIG_DIR:ro" -v "$STATE_DIR:$STATE_DIR" \
        --entrypoint yttt-relay "$1" --config "$CONFIG_DIR/relay.toml" >/dev/null
}

keep_installer() {
    if [ "$0" != "$PREFIX/yttt-relay-install" ]; then
        install -m 755 "$0" "$PREFIX/yttt-relay-install"
    fi
}

upgrade() {
    command -v sha256sum >/dev/null || die "sha256sum is required"
    fetch_binary "$arch" "$binary_url"
    keep_installer
    if [ -f "$UNIT" ]; then
        systemctl restart yttt-relay
    fi
    say "upgraded to $("$PREFIX/yttt-relay" --version 2>/dev/null || echo 'the new version')"
}

docker_upgrade() {
    image=${image:-$IMAGE_DEFAULT}
    keep_installer
    pull "$image"
    echo "IMAGE=$image" >"$CONFIG_DIR/docker.env"
    run_container "$image"
    say "upgraded to $image"
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
User=yttt-relay
Group=yttt-relay
# Certificates from Let's Encrypt persist here across restarts.
StateDirectory=yttt-relay
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
PrivateTmp=yes
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
    # Behind a proxy the relay may listen on one address only.
    host=$(awk -F'"' '/^listen/ { print $2 }' "$CONFIG_DIR/relay.toml")
    host=${host%:*}
    case "$host" in 0.0.0.0 | '[::]' | '') host=127.0.0.1 ;; esac
    say "waiting for the relay to answer on port $port"
    i=0
    while [ $i -lt 30 ]; do
        # Another program may answer on the port; the relay says "ok".
        if [ "$(curl -fsS -k "http://$host:$port/healthz" 2>/dev/null)" = ok ] \
            || [ "$(curl -fsS -k "https://$host:$port/healthz" 2>/dev/null)" = ok ]; then
            break
        fi
        i=$((i + 1))
        sleep 1
    done
    if [ $i -ge 30 ]; then
        if [ "$method" = docker ]; then
            echo "warning: the relay did not answer yet; see: docker logs $CONTAINER" >&2
        else
            echo "warning: the relay did not answer yet; see: journalctl -u yttt-relay" >&2
        fi
    fi
    url=$(awk -F'"' '/^# url/ { print $2 }' "$CONFIG_DIR/relay.toml")
    say "the node is enrolled as ${url:-this machine}"
    if [ "$behind_proxy" = 1 ]; then
        echo "    Point the proxy for $domain at http://$host:$port, with WebSocket allowed."
        echo "    Open UDP 7842 to this machine for direct paths; QUIC reads $CONFIG_DIR/quic."
    elif [ -n "$domain" ]; then
        echo "    Open TCP 443 and 80 and UDP 7842 to this machine."
        echo "    The certificate is requested in the background; the node serves once it arrives."
    else
        echo "    Open TCP $port to this machine."
    fi
    echo "    The console shows the node online within a minute."
    echo "    Later: sudo yttt-relay-install --upgrade | --uninstall"
}

uninstall() {
    if [ -f "$UNIT" ]; then
        systemctl disable --now yttt-relay || true
        rm -f "$UNIT"
        systemctl daemon-reload || true
    fi
    rm -f "$PREFIX/yttt-relay" "$PREFIX/yttt-relay-install"
    rm -rf "$CONFIG_DIR" "$STATE_DIR"
    if id yttt-relay >/dev/null 2>&1; then
        userdel yttt-relay || true
    fi
    say "removed; disable or delete the node in the console too"
}

docker_uninstall() {
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    rm -f "$PREFIX/yttt-relay-install"
    rm -rf "$CONFIG_DIR" "$STATE_DIR"
    say "removed the container; disable or delete the node in the console too"
}

main "$@"
