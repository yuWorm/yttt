#!/usr/bin/env bash
set -euo pipefail

fixture="none"
build=1
open_app=1
print_bundle_path=0

usage() {
  cat <<'EOF'
Usage: scripts/run-dev-app.sh [--fixture dev|agent|none] [--no-build] [--no-open] [--print-bundle-path]

Creates target/dev-app/yttt.app with a stable macOS bundle id for GPUI smoke testing.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --fixture)
      fixture="${2:-}"
      shift 2
      ;;
    --no-build)
      build=0
      shift
      ;;
    --no-open)
      open_app=0
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

case "$(uname -s)" in
  Darwin) ;;
  *)
    echo "scripts/run-dev-app.sh is only supported on macOS." >&2
    exit 1
    ;;
esac

case "$fixture" in
  dev)
    fixture_env='export YTTT_DEV_FIXTURE=1'
    ;;
  agent)
    fixture_env='export YTTT_DEV_FIXTURE=agent-exit'
    ;;
  none|"")
    fixture_env='unset YTTT_DEV_FIXTURE'
    ;;
  *)
    echo "Invalid fixture: $fixture" >&2
    usage >&2
    exit 2
    ;;
esac

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bundle_rel="target/dev-app/yttt.app"
bundle_dir="$repo_root/$bundle_rel"
contents_dir="$bundle_dir/Contents"
macos_dir="$contents_dir/MacOS"
resources_dir="$contents_dir/Resources"
launcher="$macos_dir/yttt-launcher"
binary="$repo_root/target/debug/yttt"
bundle_binary="$macos_dir/yttt-bin"
system_icon="/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericApplicationIcon.icns"
bundle_icon="$resources_dir/AppIcon.icns"

if [[ "$build" -eq 1 ]]; then
  cargo build --manifest-path "$repo_root/Cargo.toml"
fi

mkdir -p "$macos_dir" "$resources_dir"

if [[ ! -x "$binary" ]]; then
  echo "Missing debug binary: $binary" >&2
  echo "Run scripts/run-dev-app.sh without --no-build, or run cargo build first." >&2
  exit 1
fi

cp "$binary" "$bundle_binary"
chmod +x "$bundle_binary"

cat > "$contents_dir/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>yttt-launcher</string>
  <key>CFBundleIdentifier</key>
  <string>com.yttt.dev</string>
  <key>CFBundleName</key>
  <string>yttt</string>
  <key>CFBundleDisplayName</key>
  <string>yttt</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0-dev</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF

if [[ -f "$system_icon" ]]; then
  cp "$system_icon" "$bundle_icon"
else
  echo "Missing system app icon: $system_icon" >&2
  exit 1
fi

cat > "$launcher" <<EOF
#!/usr/bin/env bash
set -euo pipefail

repo_root="$repo_root"
cd "\$repo_root"
$fixture_env
exec "\$repo_root/$bundle_rel/Contents/MacOS/yttt-bin"
EOF
chmod +x "$launcher"

if [[ "$print_bundle_path" -eq 1 ]]; then
  printf '%s\n' "$bundle_rel"
fi

if [[ "$open_app" -eq 1 ]]; then
  open -n "$bundle_dir"
fi
