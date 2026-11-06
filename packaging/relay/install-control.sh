#!/bin/sh
# Installs the yttt relay control plane: the console, accounts and billing
# that relay nodes report to.
#
#   curl -fsSL https://github.com/yuWorm/yttt/releases/download/v__VERSION__/install-control.sh -o install-control.sh
#   sudo sh install-control.sh
#
# It asks how to install (Docker or straight onto this Linux server), which
# database (SQLite or PostgreSQL), and how browsers reach the console
# (Caddy on ports 80/443 with automatic certificates, or only the service
# for an existing reverse proxy). Every question has an option, and --yes
# takes the defaults, for running it unattended:
#
#   --method docker|native     --database sqlite|postgres
#   --expose caddy|service     --domain NAME  --email ADDRESS
#   --listen ADDRESS:PORT      --public-url URL  --behind-proxy yes|no
#   --admin-email ADDRESS      (password from YTTT_ADMIN_PASSWORD)
#   --signup open|closed       --dir DIR (Docker; default /opt/yttt-control)
#
# On a machine where it already ran it offers to upgrade, reconfigure or
# uninstall; --upgrade, --reconfigure and --uninstall [--purge] skip the
# question. Reconfiguring keeps the generated secrets: new ones would
# invalidate every relay ticket and node.
#
# Downloads are this release's own files, checked against its SHA256SUMS.
# For testing: --asset-dir DIR installs from local copies of the release
# files, --image NAME uses another Docker image, and
# YTTT_CADDY_TEST="https_port http_port" makes Caddy use its own CA on those
# ports instead of Let's Encrypt.

set -eu

VERSION='__VERSION__'
REPO='yuWorm/yttt'
CADDY_VERSION='2.10.0'

NATIVE_CONFIG_DIR=/etc/yttt-control
NATIVE_CONFIG=$NATIVE_CONFIG_DIR/control.toml
NATIVE_DATA=/var/lib/yttt-control
NATIVE_SHARE=/usr/share/yttt-control
NATIVE_BIN=/usr/local/bin/yttt-control
NATIVE_UNIT=/etc/systemd/system/yttt-control.service
CADDY_UNIT=/etc/systemd/system/caddy.service
SERVICE_USER=yttt-control
# The image runs as nobody.
IMAGE_UID=65534

die() {
    echo "install-control: $*" >&2
    exit 1
}

say() {
    echo "==> $*"
}

note() {
    echo "    $*"
}

# --- Questions -------------------------------------------------------------

tty_ready() {
    [ "$YES" = 1 ] && return 0
    { : </dev/tty; } 2>/dev/null || die "no terminal to ask on; pass the options and --yes"
}

# ask VAR "Question" DEFAULT: keeps a value already set by an option.
ask() {
    eval "current=\${$1:-}"
    if [ -n "$current" ]; then
        return
    fi
    if [ "$YES" = 1 ]; then
        eval "$1=\$3"
        return
    fi
    if [ -n "$3" ]; then
        printf '%s [%s]: ' "$2" "$3" >/dev/tty
    else
        printf '%s: ' "$2" >/dev/tty
    fi
    read -r answer </dev/tty || answer=''
    [ -n "$answer" ] || answer=$3
    eval "$1=\$answer"
}

# choose VAR "Question" DEFAULT "value|Description" ...
choose() {
    var=$1
    question=$2
    default=$3
    shift 3
    eval "current=\${$var:-}"
    if [ -n "$current" ]; then
        return
    fi
    if [ "$YES" = 1 ]; then
        eval "$var=\$default"
        return
    fi
    echo "$question" >/dev/tty
    i=1
    for option in "$@"; do
        value=${option%%|*}
        marker=' '
        [ "$value" = "$default" ] && marker='*'
        printf '  %s %d) %s\n' "$marker" "$i" "${option#*|}" >/dev/tty
        i=$((i + 1))
    done
    while :; do
        printf 'Choose [default %s]: ' "$default" >/dev/tty
        read -r answer </dev/tty || answer=''
        if [ -z "$answer" ]; then
            eval "$var=\$default"
            return
        fi
        i=1
        for option in "$@"; do
            if [ "$answer" = "$i" ] || [ "$answer" = "${option%%|*}" ]; then
                value=${option%%|*}
                eval "$var=\$value"
                return
            fi
            i=$((i + 1))
        done
        echo "Enter a number from the list." >/dev/tty
    done
}

restore_echo() {
    { stty echo </dev/tty; } 2>/dev/null || true
}

cleanup() {
    restore_echo
    [ -n "${WORK:-}" ] && rm -rf "$WORK"
}

# ask_password VAR "Question": twice, hidden.
ask_password() {
    eval "current=\${$1:-}"
    if [ -n "$current" ]; then
        return
    fi
    [ "$YES" = 1 ] && die "set YTTT_ADMIN_PASSWORD for the administrator"
    while :; do
        printf '%s: ' "$2" >/dev/tty
        stty -echo </dev/tty
        read -r first </dev/tty || first=''
        printf '\nAgain: ' >/dev/tty
        read -r second </dev/tty || second=''
        restore_echo
        printf '\n' >/dev/tty
        if [ "$first" != "$second" ]; then
            echo "The passwords differ; try again." >/dev/tty
        elif [ ${#first} -lt 8 ]; then
            echo "Use at least 8 characters." >/dev/tty
        else
            break
        fi
    done
    eval "$1=\$first"
}

# --- Helpers ---------------------------------------------------------------

random_hex() {
    od -An -N32 -tx1 /dev/urandom | tr -d ' \n'
}

arch() {
    case "$(uname -m)" in
        x86_64 | amd64) echo x86_64 ;;
        aarch64 | arm64) echo aarch64 ;;
        *) die "unsupported architecture $(uname -m)" ;;
    esac
}

# A value from an existing control.toml: key = "value".
toml_value() {
    [ -f "$2" ] || return 0
    awk -F'"' -v key="$1" '$1 ~ "^" key "[ ]*=" { print $2; exit }' "$2"
}

# fetch NAME DEST: a file of this release, checked against its SHA256SUMS.
fetch() {
    if [ -n "$ASSET_DIR" ]; then
        cp "$ASSET_DIR/$1" "$2"
        sums="$ASSET_DIR/SHA256SUMS"
    else
        base="https://github.com/$REPO/releases/download/v$VERSION"
        curl -fsSL "$base/$1" -o "$2" || die "could not download $1"
        if [ ! -f "$WORK/SHA256SUMS" ]; then
            curl -fsSL "$base/SHA256SUMS" -o "$WORK/SHA256SUMS" || die "could not download SHA256SUMS"
        fi
        sums="$WORK/SHA256SUMS"
    fi
    expected=$(awk -v name="$1" '$2 == name { print $1 }' "$sums")
    [ -n "$expected" ] || die "SHA256SUMS lists no $1"
    actual=$(sha256sum "$2" | awk '{ print $1 }')
    [ "$expected" = "$actual" ] || die "$1 does not match its checksum"
}

has() {
    command -v "$1" >/dev/null 2>&1
}

# --- Install state ---------------------------------------------------------

# What an earlier run chose, so later runs do the same.
state_file() {
    if [ "$METHOD" = docker ]; then
        echo "$DIR/install.env"
    else
        echo "$NATIVE_CONFIG_DIR/install.env"
    fi
}

save_state() {
    cat >"$(state_file)" <<EOF
METHOD=$METHOD
DATABASE=$DATABASE
EXPOSE=$EXPOSE
DOMAIN=$DOMAIN
EMAIL=$EMAIL
LISTEN=$LISTEN
PUBLIC_URL=$PUBLIC_URL
BEHIND_PROXY=$BEHIND_PROXY
DIR=$DIR
INSTALLED_VERSION=$VERSION
EOF
    chmod 600 "$(state_file)"
}

# Finds an earlier install: Docker in --dir when it was given, otherwise
# native first, then Docker in the default directory.
find_install() {
    if [ "$DIR_GIVEN" = 1 ] && [ -f "$DIR/install.env" ]; then
        echo "$DIR/install.env"
    elif [ -f "$NATIVE_CONFIG_DIR/install.env" ]; then
        echo "$NATIVE_CONFIG_DIR/install.env"
    elif [ -f "$DIR/install.env" ]; then
        echo "$DIR/install.env"
    fi
}

load_state() {
    # Values from options win over the saved ones.
    saved_method=$(awk -F= '$1 == "METHOD" { print $2 }' "$1")
    for key in DATABASE EXPOSE DOMAIN EMAIL LISTEN PUBLIC_URL BEHIND_PROXY DIR; do
        value=$(awk -F= -v key="$key" '$1 == key { sub(/^[^=]*=/, ""); print; exit }' "$1")
        eval "current=\${$key:-}"
        [ -n "$current" ] || eval "$key=\$value"
    done
    METHOD=${METHOD:-$saved_method}
}

# --- Configuration ---------------------------------------------------------

write_config() {
    path=$1
    database_url=$2
    # Reuse secrets from an existing file: never generate new ones over them.
    ticket_secret=$(toml_value ticket_secret "$path")
    secrets_key=$(toml_value secrets_key "$path")
    [ -n "$ticket_secret" ] || ticket_secret=$(random_hex)
    [ -n "$secrets_key" ] || secrets_key=$(random_hex)
    if [ "$EXPOSE" = caddy ]; then
        listen='127.0.0.1:8080'
        public_url="https://$DOMAIN"
        trusted=true
    else
        listen=$LISTEN
        public_url=$PUBLIC_URL
        if [ "$BEHIND_PROXY" = yes ]; then trusted=true; else trusted=false; fi
    fi
    # The container listens on every interface inside its own network.
    [ "$METHOD" = docker ] && listen='0.0.0.0:8080'
    signup=true
    [ "$SIGNUP" = closed ] && signup=false
    umask 077
    cat >"$path" <<EOF
# The yttt control plane, written by install-control.sh. Keep it private: it
# holds the ticket signing key and the key that seals console secrets.
listen = "$listen"
public_url = "$public_url"
trusted_proxy = $trusted
web_root = "$WEB_ROOT"
relay_binaries_dir = "$BIN_ROOT"
database_url = "$database_url"
# Changing it invalidates every relay ticket and needs every node redeployed.
ticket_secret = "$ticket_secret"
# Seals the GitHub and Google client secrets set up in the console.
secrets_key = "$secrets_key"

# Seeds for an empty database only; afterwards plans, sign-up and nodes are
# managed in the console.
[signup]
enabled = $signup
plan = "free"
node = "node-1"

[[plans]]
id = "free"
rate_bps = 250_000              # bytes per second (2 Mbps)
burst_bytes = 250_000
monthly_bytes = 10_737_418_240  # 10 GiB
throttled_rate_bps = 16_000     # after the quota (128 Kbps)
max_devices = 5
EOF
    umask 022
}

write_caddyfile() {
    path=$1
    upstream=$2
    {
        echo "# Written by install-control.sh: certificates for the console's domain."
        echo "{"
        echo "    email $EMAIL"
        if [ -n "${YTTT_CADDY_TEST:-}" ]; then
            set -- $YTTT_CADDY_TEST
            echo "    local_certs"
            echo "    https_port $1"
            echo "    http_port $2"
        else
            echo "    cert_issuer acme"
        fi
        echo "}"
        echo ""
        echo "$DOMAIN {"
        echo "    encode gzip"
        echo "    reverse_proxy $upstream"
        echo "}"
    } >"$path"
}

# --- Questions for a new install ---------------------------------------------

ask_everything() {
    choose METHOD "How should the control plane run?" docker \
        "docker|Docker (compose): easiest to upgrade and remove" \
        "native|Directly on this server (systemd): least memory"
    choose DATABASE "Which database?" sqlite \
        "sqlite|SQLite: a file, nothing else to run (personal use, small servers)" \
        "postgres|PostgreSQL: for many accounts and nodes"
    choose EXPOSE "How do browsers reach the console?" caddy \
        "caddy|Caddy on ports 80 and 443 with automatic HTTPS (needs a domain)" \
        "service|Only start the service; I run nginx or another proxy myself"
    if [ "$EXPOSE" = caddy ]; then
        while [ -z "$DOMAIN" ]; do
            ask DOMAIN "Domain for the console (its DNS points at this server)" ""
            [ -n "$DOMAIN" ] || [ "$YES" = 0 ] || die "--domain is required with --expose caddy"
        done
        ask EMAIL "Email for certificate notices" "admin@$DOMAIN"
        if has getent && ! getent hosts "$DOMAIN" >/dev/null; then
            echo "warning: $DOMAIN does not resolve yet; Caddy gets the certificate once it does." >&2
        fi
    else
        choose BIND "Listen on" loopback \
            "loopback|127.0.0.1 only: a proxy on this machine forwards to it" \
            "any|Every interface"
        if [ -z "$LISTEN" ]; then
            ask PORT "Port" 8080
            case "$BIND" in
                any) LISTEN="0.0.0.0:$PORT" ;;
                *) LISTEN="127.0.0.1:$PORT" ;;
            esac
        fi
        ask PUBLIC_URL "Address browsers and nodes use, e.g. https://control.example.com" "http://$(hostname -f 2>/dev/null || hostname):${LISTEN##*:}"
        choose BEHIND_PROXY "Is it behind a reverse proxy that sets X-Forwarded-For?" yes \
            "yes|Yes (nginx, Caddy, a load balancer)" \
            "no|No, browsers connect straight to it"
    fi
    choose SIGNUP "Can people sign up on their own?" open \
        "open|Yes, anyone can create an account" \
        "closed|No, administrators open accounts"
    ask ADMIN_EMAIL "Administrator email" ""
    [ -n "$ADMIN_EMAIL" ] || die "an administrator email is required"
    ask_password ADMIN_PASSWORD "Administrator password"
}

# --- Docker ------------------------------------------------------------------

docker_install() {
    has docker || die "Docker is not installed; see https://docs.docker.com/engine/install/"
    docker compose version >/dev/null 2>&1 || die "docker compose is not available"
    mkdir -p "$DIR/data"
    image=${IMAGE:-ghcr.io/yuworm/yttt-relay:$VERSION}
    WEB_ROOT=/usr/share/yttt-control/web
    BIN_ROOT=/usr/share/yttt-control/bin
    postgres_password=$(awk -F= '$1 == "POSTGRES_PASSWORD" { print $2 }' "$DIR/.env" 2>/dev/null || true)
    [ -n "$postgres_password" ] || postgres_password=$(random_hex)
    if [ "$DATABASE" = postgres ]; then
        database_url="postgres://postgres:$postgres_password@postgres:5432/postgres"
    else
        database_url="sqlite:///var/lib/yttt-control/control.db"
    fi
    write_config "$DIR/control.toml" "$database_url"
    umask 077
    {
        echo "YTTT_IMAGE=$image"
        echo "POSTGRES_PASSWORD=$postgres_password"
    } >"$DIR/.env"
    umask 022
    if [ "$EXPOSE" = caddy ]; then
        published='127.0.0.1:8080:8080'
        write_caddyfile "$DIR/Caddyfile" 127.0.0.1:8080
    else
        published="${LISTEN%:*}:${LISTEN##*:}:8080"
    fi
    {
        echo "# Written by install-control.sh; rerun it to change the setup."
        echo "services:"
        echo "  control:"
        echo "    image: \${YTTT_IMAGE}"
        echo "    restart: unless-stopped"
        echo "    volumes:"
        echo "      - ./control.toml:/etc/yttt/control.toml:ro"
        echo "      - ./data:/var/lib/yttt-control"
        echo "    ports:"
        echo "      - \"$published\""
        if [ "$DATABASE" = postgres ]; then
            echo "    depends_on:"
            echo "      postgres:"
            echo "        condition: service_healthy"
            echo "  postgres:"
            echo "    image: postgres:17-alpine"
            echo "    restart: unless-stopped"
            echo "    environment:"
            echo "      POSTGRES_PASSWORD: \${POSTGRES_PASSWORD}"
            echo "    volumes:"
            echo "      - pgdata:/var/lib/postgresql/data"
            echo "    healthcheck:"
            echo "      test: [\"CMD\", \"pg_isready\", \"-U\", \"postgres\"]"
            echo "      interval: 2s"
            echo "      retries: 30"
        fi
        if [ "$EXPOSE" = caddy ]; then
            echo "  caddy:"
            echo "    image: caddy:$CADDY_VERSION"
            echo "    restart: unless-stopped"
            # Host networking: it reaches the control plane on 127.0.0.1.
            echo "    network_mode: host"
            echo "    volumes:"
            echo "      - ./Caddyfile:/etc/caddy/Caddyfile:ro"
            echo "      - caddy_data:/data"
            echo "      - caddy_config:/config"
        fi
        if [ "$DATABASE" = postgres ] || [ "$EXPOSE" = caddy ]; then
            echo "volumes:"
            [ "$DATABASE" = postgres ] && echo "  pgdata:"
            if [ "$EXPOSE" = caddy ]; then
                echo "  caddy_data:"
                echo "  caddy_config:"
            fi
        fi
    } >"$DIR/compose.yml"
    # The image runs as nobody, which must read the config and own the data.
    chown "$IMAGE_UID:$IMAGE_UID" "$DIR/control.toml" "$DIR/data"
    chmod 600 "$DIR/control.toml"
    say "starting the containers"
    (cd "$DIR" && { [ -n "$IMAGE" ] || docker compose pull -q; } && docker compose up -d)
    wait_answering "$DIR"
}

docker_admin() {
    printf '%s\n' "$ADMIN_PASSWORD" | (cd "$DIR" && docker compose exec -T control \
        yttt-control create-admin --config /etc/yttt/control.toml "$ADMIN_EMAIL")
}

docker_upgrade() {
    image=${IMAGE:-ghcr.io/yuworm/yttt-relay:$VERSION}
    sed -i "s|^YTTT_IMAGE=.*|YTTT_IMAGE=$image|" "$DIR/.env"
    (cd "$DIR" && { [ -n "$IMAGE" ] || docker compose pull -q; } && docker compose up -d)
    wait_answering "$DIR"
}

docker_uninstall() {
    if [ "$PURGE" = 1 ]; then
        (cd "$DIR" && docker compose down -v) || true
        rm -rf "$DIR"
        say "removed the containers, their data and $DIR"
    else
        (cd "$DIR" && docker compose down) || true
        rm -f "$DIR/install.env"
        say "stopped and removed the containers; data and settings stay in $DIR (--purge deletes them)"
    fi
}

# --- Directly on the server ----------------------------------------------------

native_install() {
    has systemctl || die "systemd is required to install directly; use --method docker"
    WEB_ROOT=$NATIVE_SHARE/web
    BIN_ROOT=$NATIVE_SHARE/bin
    if ! id "$SERVICE_USER" >/dev/null 2>&1; then
        useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin "$SERVICE_USER"
    fi
    native_binaries
    mkdir -p "$NATIVE_CONFIG_DIR"
    if [ "$DATABASE" = postgres ]; then
        native_postgres
        database_url="postgres:///yttt_control?host=/var/run/postgresql&user=$SERVICE_USER"
    else
        database_url="sqlite://$NATIVE_DATA/control.db"
    fi
    write_config "$NATIVE_CONFIG" "$database_url"
    chown "root:$SERVICE_USER" "$NATIVE_CONFIG_DIR" "$NATIVE_CONFIG"
    chmod 750 "$NATIVE_CONFIG_DIR"
    chmod 640 "$NATIVE_CONFIG"
    after=network-online.target
    [ "$DATABASE" = postgres ] && after="$after postgresql.service"
    cat >"$NATIVE_UNIT" <<EOF
[Unit]
Description=yttt relay control plane
After=$after
Wants=network-online.target

[Service]
ExecStart=$NATIVE_BIN --config $NATIVE_CONFIG
User=$SERVICE_USER
Group=$SERVICE_USER
StateDirectory=yttt-control
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ProtectHome=yes
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    systemctl enable yttt-control >/dev/null
    systemctl restart yttt-control
    [ "$EXPOSE" = caddy ] && native_caddy
    wait_answering ""
}

native_binaries() {
    a=$(arch)
    say "installing yttt-control $VERSION"
    fetch "yttt-control-$VERSION-linux-$a" "$WORK/yttt-control"
    install -m 755 "$WORK/yttt-control" "$NATIVE_BIN"
    fetch "yttt-control-web-$VERSION.tar.gz" "$WORK/web.tar.gz"
    rm -rf "$WORK/web" && mkdir -p "$WORK/web"
    tar -xzf "$WORK/web.tar.gz" -C "$WORK/web"
    mkdir -p "$NATIVE_SHARE"
    rm -rf "$NATIVE_SHARE/web.new" && mv "$WORK/web" "$NATIVE_SHARE/web.new"
    rm -rf "$NATIVE_SHARE/web" && mv "$NATIVE_SHARE/web.new" "$NATIVE_SHARE/web"
    # The relay binaries new nodes download from this control plane.
    mkdir -p "$NATIVE_SHARE/bin"
    for node_arch in x86_64 aarch64; do
        fetch "yttt-relay-$VERSION-linux-$node_arch" "$NATIVE_SHARE/bin/yttt-relay-linux-$node_arch"
        chmod 755 "$NATIVE_SHARE/bin/yttt-relay-linux-$node_arch"
    done
    (cd "$NATIVE_SHARE/bin" && sha256sum yttt-relay-linux-x86_64 yttt-relay-linux-aarch64 >SHA256SUMS)
}

native_postgres() {
    if ! has psql; then
        say "installing PostgreSQL"
        if has apt-get; then
            DEBIAN_FRONTEND=noninteractive apt-get install -y -q postgresql >/dev/null
        elif has dnf; then
            dnf install -y -q postgresql-server >/dev/null
            postgresql-setup --initdb >/dev/null 2>&1 || true
        else
            die "install PostgreSQL yourself, or choose SQLite"
        fi
    fi
    systemctl enable --now postgresql >/dev/null 2>&1 || true
    # The service connects over the local socket as its own user (peer
    # authentication), so there is no database password to keep.
    exists=$(su postgres -s /bin/sh -c "psql -tAc \"SELECT 1 FROM pg_roles WHERE rolname = '$SERVICE_USER'\"")
    if [ "$exists" != 1 ]; then
        su postgres -s /bin/sh -c "psql -qc 'CREATE ROLE \"$SERVICE_USER\" LOGIN'"
    fi
    exists=$(su postgres -s /bin/sh -c "psql -tAc \"SELECT 1 FROM pg_database WHERE datname = 'yttt_control'\"")
    if [ "$exists" != 1 ]; then
        su postgres -s /bin/sh -c "createdb -O '$SERVICE_USER' yttt_control"
    fi
}

native_caddy() {
    if ! has caddy; then
        say "installing Caddy $CADDY_VERSION"
        case "$(arch)" in
            x86_64) caddy_arch=amd64 ;;
            aarch64) caddy_arch=arm64 ;;
        esac
        name="caddy_${CADDY_VERSION}_linux_$caddy_arch.tar.gz"
        base="https://github.com/caddyserver/caddy/releases/download/v$CADDY_VERSION"
        curl -fsSL "$base/$name" -o "$WORK/$name" || die "could not download Caddy"
        curl -fsSL "$base/caddy_${CADDY_VERSION}_checksums.txt" -o "$WORK/caddy.sums" || die "could not download Caddy's checksums"
        expected=$(awk -v name="$name" '$2 == name { print $1 }' "$WORK/caddy.sums")
        actual=$(sha512sum "$WORK/$name" | awk '{ print $1 }')
        [ -n "$expected" ] && [ "$expected" = "$actual" ] || die "Caddy does not match its checksum"
        tar -xzf "$WORK/$name" -C "$WORK" caddy
        install -m 755 "$WORK/caddy" /usr/local/bin/caddy
        touch "$NATIVE_CONFIG_DIR/caddy-installed"
    fi
    if ! id caddy >/dev/null 2>&1; then
        useradd --system --home-dir /var/lib/caddy --create-home --shell /usr/sbin/nologin caddy
    fi
    mkdir -p /etc/caddy
    write_caddyfile /etc/caddy/Caddyfile 127.0.0.1:8080
    cat >"$CADDY_UNIT" <<EOF
[Unit]
Description=Caddy, in front of the yttt control plane
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=$(command -v caddy) run --environ --config /etc/caddy/Caddyfile
ExecReload=$(command -v caddy) reload --config /etc/caddy/Caddyfile --force
User=caddy
Group=caddy
Environment=HOME=/var/lib/caddy
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=full
LimitNOFILE=1048576
Restart=on-failure

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    systemctl enable caddy >/dev/null
    systemctl restart caddy
}

native_admin() {
    printf '%s\n' "$ADMIN_PASSWORD" | runuser -u "$SERVICE_USER" -- \
        "$NATIVE_BIN" create-admin --config "$NATIVE_CONFIG" "$ADMIN_EMAIL"
}

native_upgrade() {
    native_binaries
    systemctl restart yttt-control
    wait_answering ""
}

native_uninstall() {
    systemctl disable --now yttt-control >/dev/null 2>&1 || true
    rm -f "$NATIVE_UNIT" "$NATIVE_BIN"
    rm -rf "$NATIVE_SHARE"
    if [ -f "$NATIVE_CONFIG_DIR/caddy-installed" ]; then
        systemctl disable --now caddy >/dev/null 2>&1 || true
        rm -f "$CADDY_UNIT" /usr/local/bin/caddy /etc/caddy/Caddyfile
    fi
    systemctl daemon-reload || true
    if [ "$PURGE" = 1 ]; then
        rm -rf "$NATIVE_CONFIG_DIR" "$NATIVE_DATA"
        if has psql && [ "$DATABASE" = postgres ]; then
            su postgres -s /bin/sh -c "dropdb --if-exists yttt_control" || true
            su postgres -s /bin/sh -c "psql -qc 'DROP ROLE IF EXISTS \"$SERVICE_USER\"'" || true
        fi
        userdel "$SERVICE_USER" 2>/dev/null || true
        say "removed the control plane, its settings and its data"
    else
        rm -f "$NATIVE_CONFIG_DIR/install.env"
        say "removed the control plane; settings stay in $NATIVE_CONFIG_DIR and data in $NATIVE_DATA (--purge deletes them)"
    fi
}

# --- Shared ------------------------------------------------------------------

# Waits for the control plane to answer on its own port; $1 is the Docker
# directory, empty when installed directly.
wait_answering() {
    i=0
    while [ $i -lt 60 ]; do
        curl -fsS "http://$(local_target)/healthz" >/dev/null 2>&1 && return
        i=$((i + 1))
        sleep 1
    done
    if [ -n "$1" ]; then
        (cd "$1" && docker compose logs --tail 20 control) >&2 || true
    else
        journalctl -u yttt-control -n 20 --no-pager >&2 || true
    fi
    die "the control plane did not start"
}

# Where this machine reaches the control plane directly.
local_target() {
    if [ "$EXPOSE" = caddy ]; then
        echo 127.0.0.1:8080
    else
        port=${LISTEN##*:}
        echo "127.0.0.1:$port"
    fi
}

summary() {
    if [ "$EXPOSE" = caddy ]; then
        url="https://$DOMAIN"
    else
        url=$PUBLIC_URL
    fi
    say "the control plane is running: $url"
    if [ "$METHOD" = docker ]; then
        note "Settings and secrets: $DIR/control.toml and $DIR/.env (private)"
    else
        note "Settings and secrets: $NATIVE_CONFIG (private)"
    fi
    if [ "$EXPOSE" = caddy ]; then
        note "Open TCP 80 and 443 to this server; the certificate arrives within a minute."
    elif [ "$BEHIND_PROXY" = yes ]; then
        note "Point your reverse proxy at http://${LISTEN} and keep its X-Forwarded-For header."
    fi
    if [ "$ADMIN_EMAIL" != none ]; then
        note "Sign in as $ADMIN_EMAIL, then deploy a relay node from Nodes."
    fi
    note "Later: sudo sh install-control.sh --upgrade | --reconfigure | --uninstall"
}

main() {
    METHOD=''
    DATABASE=''
    EXPOSE=''
    DOMAIN=''
    EMAIL=''
    LISTEN=''
    PUBLIC_URL=''
    BEHIND_PROXY=''
    BIND=''
    PORT=''
    SIGNUP=''
    ADMIN_EMAIL=''
    ADMIN_PASSWORD=${YTTT_ADMIN_PASSWORD:-}
    DIR=/opt/yttt-control
    DIR_GIVEN=0
    ASSET_DIR=''
    IMAGE=''
    YES=0
    PURGE=0
    ACTION=''
    while [ $# -gt 0 ]; do
        case "$1" in
            --method | --database | --expose | --domain | --email | --listen | --public-url | \
                --behind-proxy | --admin-email | --signup | --dir | --asset-dir | --image)
                [ $# -ge 2 ] || die "$1 needs a value"
                case "$1" in
                    --method) METHOD=$2 ;;
                    --database) DATABASE=$2 ;;
                    --expose) EXPOSE=$2 ;;
                    --domain) DOMAIN=$2 ;;
                    --email) EMAIL=$2 ;;
                    --listen) LISTEN=$2 ;;
                    --public-url) PUBLIC_URL=$2 ;;
                    --behind-proxy) BEHIND_PROXY=$2 ;;
                    --admin-email) ADMIN_EMAIL=$2 ;;
                    --signup) SIGNUP=$2 ;;
                    --dir) DIR=$2; DIR_GIVEN=1 ;;
                    --asset-dir) ASSET_DIR=$2 ;;
                    --image) IMAGE=$2 ;;
                esac
                shift 2
                ;;
            --yes) YES=1; shift ;;
            --upgrade) ACTION=upgrade; shift ;;
            --reconfigure) ACTION=reconfigure; shift ;;
            --uninstall) ACTION=uninstall; shift ;;
            --purge) PURGE=1; shift ;;
            -h | --help) awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"; exit 0 ;;
            *) die "unknown option $1" ;;
        esac
    done
    # An unpublished copy has no release to download from.
    case "$VERSION" in
        __*) [ -n "$ASSET_DIR$IMAGE" ] || die "run the copy published with a release" ;;
    esac
    [ "$(id -u)" = 0 ] || die "run as root, e.g. with sudo"
    [ "$(uname -s)" = Linux ] || die "the control plane runs on Linux"
    has curl || die "curl is required"
    has sha256sum || die "sha256sum is required"
    WORK=$(mktemp -d)
    trap cleanup EXIT
    trap 'exit 130' INT TERM

    # What the options said, before the saved choices fill the gaps.
    given="EXPOSE=$EXPOSE DOMAIN=$DOMAIN EMAIL=$EMAIL LISTEN=$LISTEN PUBLIC_URL=$PUBLIC_URL BEHIND_PROXY=$BEHIND_PROXY"
    existing=$(find_install)
    if [ -n "$existing" ]; then
        load_state "$existing"
        if [ -z "$ACTION" ]; then
            tty_ready
            choose ACTION "yttt-control is already installed here ($METHOD). What now?" upgrade \
                "upgrade|Upgrade to $VERSION, keeping settings and data" \
                "reconfigure|Change how it is reached or set up, keeping secrets and data" \
                "uninstall|Remove it"
        fi
        case "$ACTION" in
            upgrade)
                "${METHOD}_upgrade"
                save_state
                say "upgraded to $VERSION"
                return
                ;;
            uninstall)
                "${METHOD}_uninstall"
                return
                ;;
            reconfigure)
                # On a terminal, ask again how it is reached, except what the
                # options say; --yes keeps the saved answers. Method and
                # database stay.
                if [ "$YES" = 0 ]; then
                    for pair in $given; do
                        eval "${pair%%=*}=\${pair#*=}"
                    done
                fi
                tty_ready
                SIGNUP=closed
                ADMIN_EMAIL=none
                ADMIN_PASSWORD=none
                ask_everything
                "${METHOD}_install"
                save_state
                summary
                return
                ;;
            *) die "unknown action $ACTION" ;;
        esac
    fi
    [ "$ACTION" = "" ] || [ "$ACTION" = reconfigure ] || die "yttt-control is not installed here"
    tty_ready
    ask_everything
    case "$METHOD" in docker | native) ;; *) die "--method is docker or native" ;; esac
    case "$DATABASE" in sqlite | postgres) ;; *) die "--database is sqlite or postgres" ;; esac
    case "$EXPOSE" in caddy | service) ;; *) die "--expose is caddy or service" ;; esac
    "${METHOD}_install"
    say "creating the administrator $ADMIN_EMAIL"
    "${METHOD}_admin"
    save_state
    summary
}

main "$@"
