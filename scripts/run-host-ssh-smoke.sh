#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image="yttt-ssh-smoke:local"
container="yttt-ssh-smoke-$$"
port="$(python3 -c 'import socket; sock = socket.socket(); sock.bind(("127.0.0.1", 0)); print(sock.getsockname()[1]); sock.close()')"

cleanup() {
  docker rm --force "$container" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

docker build --quiet --file "$repo_root/scripts/fixtures/ssh-smoke.Dockerfile" --tag "$image" "$repo_root"
docker run --detach --name "$container" --publish "127.0.0.1:$port:22" "$image" >/dev/null

python3 - "$port" <<'PY'
import socket
import sys
import time

port = int(sys.argv[1])
deadline = time.monotonic() + 30
while time.monotonic() < deadline:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=1):
            break
    except OSError:
        time.sleep(0.1)
else:
    raise SystemExit("SSH smoke container did not become ready")
PY

YTTT_SSH_SMOKE_HOST=127.0.0.1 \
YTTT_SSH_SMOKE_PORT="$port" \
YTTT_SSH_SMOKE_USERNAME=yttt \
YTTT_SSH_SMOKE_PASSWORD=yttt-smoke \
  cargo test --locked -p yttt-client-core --test host_roundtrip \
    host_ssh_product_smoke_covers_host_key_sftp_git_and_terminal -- \
    --exact --ignored --nocapture
