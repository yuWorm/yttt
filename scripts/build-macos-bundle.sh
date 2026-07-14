#!/usr/bin/env bash
set -euo pipefail

build=1
sign=1
print_bundle_path=0
binary_arg=""
output_arg="target/macos/yttt.app"

usage() {
  cat <<'EOF'
Usage: scripts/build-macos-bundle.sh [options]

Builds a release binary and packages it as a launchable macOS application bundle.

Options:
  --no-build                 Package an existing release binary without running cargo build.
  --binary PATH              Package PATH instead of target/release/yttt (requires --no-build).
  --output PATH              Bundle destination (default: target/macos/yttt.app).
  --no-sign                  Skip ad-hoc code signing.
  --print-bundle-path        Print the bundle path after a successful build.
  -h, --help                 Show this help.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-build)
      build=0
      shift
      ;;
    --binary)
      binary_arg="${2:-}"
      if [[ -z "$binary_arg" ]]; then
        echo "--binary requires a path." >&2
        exit 2
      fi
      shift 2
      ;;
    --output)
      output_arg="${2:-}"
      if [[ -z "$output_arg" ]]; then
        echo "--output requires a path." >&2
        exit 2
      fi
      shift 2
      ;;
    --no-sign)
      sign=0
      shift
      ;;
    --print-bundle-path)
      print_bundle_path=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "scripts/build-macos-bundle.sh is only supported on macOS." >&2
  exit 1
fi

if [[ -n "$binary_arg" && "$build" -eq 1 ]]; then
  echo "--binary requires --no-build so the selected artifact cannot be overwritten." >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/Cargo.toml"
app_icon="$repo_root/assets/app-icon/macos/AppIcon.icns"

if [[ "$output_arg" = /* ]]; then
  bundle_dir="$output_arg"
else
  bundle_dir="$repo_root/$output_arg"
fi

if [[ -n "$binary_arg" ]]; then
  if [[ "$binary_arg" = /* ]]; then
    binary="$binary_arg"
  else
    binary="$repo_root/$binary_arg"
  fi
else
  binary="$repo_root/target/release/yttt"
fi

if [[ ! -f "$app_icon" ]]; then
  echo "Missing app icon: $app_icon" >&2
  echo "Run scripts/build-app-icons.sh first." >&2
  exit 1
fi

if [[ "$build" -eq 1 ]]; then
  cargo build --release --locked --manifest-path "$manifest"
fi

if [[ ! -x "$binary" ]]; then
  echo "Missing executable binary: $binary" >&2
  if [[ "$build" -eq 0 ]]; then
    echo "Remove --no-build, or provide an executable with --binary PATH." >&2
  fi
  exit 1
fi

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
if [[ -z "$version" ]]; then
  echo "Unable to read the package version from $manifest" >&2
  exit 1
fi
bundle_version="${version%%-*}"

contents_dir="$bundle_dir/Contents"
macos_dir="$contents_dir/MacOS"
resources_dir="$contents_dir/Resources"
bundle_binary="$macos_dir/yttt"

rm -rf "$bundle_dir"
mkdir -p "$macos_dir" "$resources_dir"
install -m 755 "$binary" "$bundle_binary"
install -m 644 "$app_icon" "$resources_dir/AppIcon.icns"

cat > "$contents_dir/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>yttt</string>
  <key>CFBundleExecutable</key>
  <string>yttt</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon.icns</string>
  <key>CFBundleIdentifier</key>
  <string>com.yttt.app</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>yttt</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$version</string>
  <key>CFBundleVersion</key>
  <string>$bundle_version</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.developer-tools</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSPrincipalClass</key>
  <string>NSApplication</string>
</dict>
</plist>
EOF

printf 'APPL????' > "$contents_dir/PkgInfo"
/usr/bin/plutil -lint "$contents_dir/Info.plist" >/dev/null

if [[ "$sign" -eq 1 ]]; then
  /usr/bin/codesign --force --sign - "$bundle_dir"
  /usr/bin/codesign --verify --deep --strict "$bundle_dir"
fi

if [[ "$print_bundle_path" -eq 1 ]]; then
  printf '%s\n' "$output_arg"
fi
