#!/usr/bin/env bash
set -euo pipefail

build=1
sign=1
print_dmg_path=0
binary_arg=""
output_arg="target/macos/yttt.dmg"

usage() {
  cat <<'EOF'
Usage: scripts/build-macos-dmg.sh [options]

Builds a release binary, packages it as a macOS application, and creates a DMG disk image.

Options:
  --no-build                 Package an existing release binary without running cargo build.
  --binary PATH              Package PATH instead of target/release/yttt (requires --no-build).
  --output PATH              DMG destination (default: target/macos/yttt.dmg).
  --no-sign                  Skip ad-hoc code signing of the application bundle.
  --print-dmg-path           Print the DMG path after a successful build.
  -h, --help                 Show this help.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-build) build=0; shift ;;
    --binary)
      binary_arg="${2:-}"
      [[ -n "$binary_arg" ]] || { echo "--binary requires a path." >&2; exit 2; }
      shift 2
      ;;
    --output)
      output_arg="${2:-}"
      [[ -n "$output_arg" ]] || { echo "--output requires a path." >&2; exit 2; }
      shift 2
      ;;
    --no-sign) sign=0; shift ;;
    --print-dmg-path) print_dmg_path=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "scripts/build-macos-dmg.sh is only supported on macOS." >&2
  exit 1
fi
if [[ -n "$binary_arg" && "$build" -eq 1 ]]; then
  echo "--binary requires --no-build so the selected artifact cannot be overwritten." >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "$output_arg" = /* ]]; then dmg_path="$output_arg"; else dmg_path="$repo_root/$output_arg"; fi

bundle_args=()
[[ "$build" -eq 0 ]] && bundle_args+=(--no-build)
[[ -n "$binary_arg" ]] && bundle_args+=(--binary "$binary_arg")
[[ "$sign" -eq 0 ]] && bundle_args+=(--no-sign)

staging_dir="$(mktemp -d "${TMPDIR:-/tmp}/yttt-dmg.XXXXXX")"
cleanup() { rm -rf "$staging_dir"; }
trap cleanup EXIT
bundle_args+=(--output "$staging_dir/yttt.app")

"$repo_root/scripts/build-macos-bundle.sh" "${bundle_args[@]}"
ln -s /Applications "$staging_dir/Applications"
mkdir -p "$(dirname "$dmg_path")"
rm -f "$dmg_path"

/usr/bin/hdiutil create \
  -volname "yttt" \
  -srcfolder "$staging_dir" \
  -ov \
  -format UDZO \
  "$dmg_path" >/dev/null

if [[ "$print_dmg_path" -eq 1 ]]; then
  printf '%s\n' "$output_arg"
fi
