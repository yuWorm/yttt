#!/usr/bin/env bash
set -euo pipefail

build=1
binary_arg=""
target_triple="x86_64-unknown-linux-gnu"
output_arg=""

usage() {
  cat <<'EOF'
Usage: scripts/build-linux-tar.sh [options]

Builds a release binary and packages it with Linux desktop integration in a tar.gz archive.

Options:
  --no-build                 Package an existing binary without running cargo build.
  --binary PATH              Package PATH instead of target/<target>/release/yttt.
  --target TRIPLE            Rust target triple (default: x86_64-unknown-linux-gnu).
  --output PATH              Archive destination; defaults to a versioned target/linux path.
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
    --target)
      target_triple="${2:-}"
      [[ -n "$target_triple" ]] || { echo "--target requires a Rust target triple." >&2; exit 2; }
      shift 2
      ;;
    --output)
      output_arg="${2:-}"
      [[ -n "$output_arg" ]] || { echo "--output requires a path." >&2; exit 2; }
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ -n "$binary_arg" && "$build" -eq 1 ]]; then
  echo "--binary requires --no-build so the selected artifact cannot be overwritten." >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/Cargo.toml"
desktop_file="$repo_root/packaging/linux/com.yttt.app.desktop"

version="$(/usr/bin/awk '
  /^\[package\]$/ { in_package = 1; next }
  /^\[/ { if (in_package) exit }
  in_package && $1 == "version" {
    value = $3
    gsub(/\"/, "", value)
    print value
    exit
  }
' "$manifest")"
[[ -n "$version" ]] || { echo "Unable to read package version from $manifest" >&2; exit 1; }

architecture="${target_triple%%-*}"
case "$architecture" in
  x86_64) package_arch="x86_64" ;;
  aarch64) package_arch="aarch64" ;;
  *) package_arch="$architecture" ;;
esac
package_name="yttt-$version-linux-$package_arch"
if [[ -z "$output_arg" ]]; then
  output_arg="target/linux/$package_name.tar.gz"
fi
if [[ "$output_arg" = /* ]]; then output_path="$output_arg"; else output_path="$repo_root/$output_arg"; fi

if [[ -n "$binary_arg" ]]; then
  if [[ "$binary_arg" = /* ]]; then binary="$binary_arg"; else binary="$repo_root/$binary_arg"; fi
else
  binary="$repo_root/target/$target_triple/release/yttt"
fi

if [[ "$build" -eq 1 ]]; then
  cargo build --release --locked --manifest-path "$manifest" --target "$target_triple" --bin yttt
fi
[[ -f "$binary" ]] || { echo "Missing executable binary: $binary" >&2; exit 1; }
[[ -f "$desktop_file" ]] || { echo "Missing desktop file: $desktop_file" >&2; exit 1; }

staging_dir="$(mktemp -d "${TMPDIR:-/tmp}/yttt-linux.XXXXXX")"
cleanup() { rm -rf "$staging_dir"; }
trap cleanup EXIT
package_root="$staging_dir/$package_name"
install -d \
  "$package_root/bin" \
  "$package_root/share/applications" \
  "$package_root/share/doc/yttt"
install -m 755 "$binary" "$package_root/bin/yttt"
install -m 644 "$desktop_file" "$package_root/share/applications/com.yttt.app.desktop"
install -m 644 "$repo_root/README.md" "$package_root/share/doc/yttt/README.md"

for size in 16 32 48 64 128 256 512; do
  icon="$repo_root/assets/app-icon/png/$size.png"
  [[ -f "$icon" ]] || { echo "Missing Linux icon: $icon" >&2; exit 1; }
  icon_dir="$package_root/share/icons/hicolor/${size}x${size}/apps"
  install -d "$icon_dir"
  install -m 644 "$icon" "$icon_dir/com.yttt.app.png"
done

mkdir -p "$(dirname "$output_path")"
rm -f "$output_path"
tar -C "$staging_dir" -czf "$output_path" "$package_name"
printf '%s\n' "$output_path"
