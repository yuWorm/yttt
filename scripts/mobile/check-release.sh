#!/usr/bin/env bash
# Fails if a mobile build still carries the spike's launch hooks: the names
# only debug builds may read. Run it on a debug build first to see it find them.
#
# Usage: scripts/mobile/check-release.sh <app.apk | App.app>
set -euo pipefail

build="${1:?usage: check-release.sh <app.apk | App.app>}"

case "$build" in
  *.apk)
    # The debug source set's launch extra; release reads none.
    markers=(qrImage)
    dir="$(mktemp -d)"
    trap 'rm -rf "$dir"' EXIT
    unzip -q "$build" 'classes*.dex' -d "$dir"
    ;;
  *.app | *.app/)
    # The environment variables of the Debug configuration's hooks.
    markers=(YTTT_SPIKE)
    dir="$build"
    ;;
  *)
    echo "not an .apk or .app: $build" >&2
    exit 2
    ;;
esac

found=0
for marker in "${markers[@]}"; do
  if files="$(grep -rlaF -- "$marker" "$dir")"; then
    echo "found \"$marker\" in:" >&2
    sed "s|^$dir/*|  |" <<<"$files" >&2
    found=1
  fi
done
if ((found)); then
  echo "$build carries launch hooks" >&2
  exit 1
fi
echo "$build: no launch hooks"
