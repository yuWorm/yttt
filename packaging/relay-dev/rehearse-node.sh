#!/bin/sh
# Runs a deploy command from the console on a throwaway systemd machine and
# checks the whole life of a node: install, service, restart, upgrade,
# uninstall. The control plane's public_url must be reachable from a
# container (e.g. http://host.docker.internal:<port>).
#
#   packaging/relay-dev/rehearse-node.sh '<deploy command>' [linux/amd64|linux/arm64]
set -eu

command=$1
platform=${2:-}
name=yttt-node-rehearsal
here=$(cd "$(dirname "$0")" && pwd)
platform_flag=${platform:+--platform $platform}

step() { echo "==> $*"; }
fail() {
    echo "rehearsal failed: $*" >&2
    docker exec "$name" journalctl -u yttt-relay --no-pager 2>&1 | tail -20 >&2
    exit 1
}

# shellcheck disable=SC2086
docker build -q $platform_flag -t yttt-node-rehearsal "$here/rehearsal" >/dev/null
docker rm -f "$name" >/dev/null 2>&1 || true
# shellcheck disable=SC2086
docker run -d --name "$name" $platform_flag --privileged --cgroupns=host \
    -v /sys/fs/cgroup:/sys/fs/cgroup:rw --tmpfs /run --tmpfs /run/lock \
    --add-host host.docker.internal:host-gateway yttt-node-rehearsal >/dev/null
trap 'docker rm -f "$name" >/dev/null 2>&1 || true' EXIT
for _ in $(seq 1 30); do
    state=$(docker exec "$name" systemctl is-system-running 2>/dev/null || true)
    case "$state" in running | degraded) break ;; esac
    sleep 1
done

step "deploying"
# The container runs as root; sudo is not installed.
docker exec "$name" sh -c "$(printf '%s' "$command" | sed 's/sudo //')" || fail "the deploy command failed"
docker exec "$name" systemctl is-active --quiet yttt-relay || fail "the service is not active"
docker exec "$name" sh -c 'test "$(stat -c %a:%U:%G /etc/yttt-relay/node-token)" = 640:root:yttt-relay' \
    || fail "the node token is not readable by the service alone"

step "restarting the machine"
docker restart "$name" >/dev/null
for _ in $(seq 1 30); do
    docker exec "$name" systemctl is-active --quiet yttt-relay && break
    sleep 1
done
docker exec "$name" systemctl is-active --quiet yttt-relay || fail "the service did not come back"
docker exec "$name" sh -c 'curl -fsS http://127.0.0.1:3341/healthz >/dev/null 2>&1 || curl -fsSk https://127.0.0.1/healthz >/dev/null' \
    || fail "the relay does not answer"

step "upgrading"
docker exec "$name" yttt-relay-install --upgrade || fail "upgrade failed"
docker exec "$name" systemctl is-active --quiet yttt-relay || fail "the service is not active after upgrading"

step "uninstalling"
docker exec "$name" yttt-relay-install --uninstall || fail "uninstall failed"
docker exec "$name" sh -c '! test -e /usr/local/bin/yttt-relay && ! test -e /usr/local/bin/yttt-relay-install && ! test -e /etc/yttt-relay && ! id yttt-relay 2>/dev/null' \
    || fail "files or the service user were left behind"

step "the node deployed, survived a restart, upgraded and uninstalled"
