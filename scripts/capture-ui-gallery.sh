#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/capture-ui-gallery.sh [--output-dir PATH] [--no-build] [--zed]

Renders the gallery offscreen twice and reports the renderer's self-noise.
With --zed, also captures the frontmost Zed window twice with screencapture,
which needs Screen Recording permission for the terminal running this script.
Set up Zed from tests/visual/zed/settings.json before capturing.
EOF
}

output_dir="tmp/ui-gallery/$(date +%Y%m%d-%H%M%S)"
build=1
zed=0
while (($#)); do
  case "$1" in
    --output-dir) output_dir="$2"; shift 2 ;;
    --no-build) build=0; shift ;;
    --zed) zed=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
mkdir -p "$output_dir"
if ((build)); then
  cargo build --locked --features ui-gallery --bin yttt-ui-gallery
fi

gallery="${CARGO_TARGET_DIR:-target}/debug/yttt-ui-gallery"
for run in 1 2; do
  "$gallery" --screenshot "$output_dir/yttt-$run.png" --dump-layout "$output_dir/layout-$run.json"
done
cmp "$output_dir/layout-1.json" "$output_dir/layout-2.json"

whole_image_regions() {
  python3 - "$1" "$2" <<'PY'
import sys
from pathlib import Path

sys.path.insert(0, "scripts")
import ui_visual_diff

image = ui_visual_diff.read_png(Path(sys.argv[1]))
Path(sys.argv[2]).write_text(
    f'[[region]]\nname = "window"\nx = 0\ny = 0\nwidth = {image.width}\nheight = {image.height}\n',
    encoding="utf-8",
)
PY
}

noise() {
  local name="$1"
  whole_image_regions "$output_dir/$name-1.png" "$output_dir/$name-regions.toml"
  python3 scripts/ui_visual_diff.py \
    --baseline "$output_dir/$name-1.png" \
    --actual "$output_dir/$name-2.png" \
    --regions "$output_dir/$name-regions.toml" \
    --manifest tests/visual/zed/manifest.toml \
    --output-dir "$output_dir/$name-noise"
}

noise yttt

if ((zed)); then
  window_id="$(swift -e '
import CoreGraphics
let options = CGWindowListOption(arrayLiteral: .optionOnScreenOnly, .excludeDesktopElements)
let windows = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] ?? []
let zed = windows.first { ($0["kCGWindowOwnerName"] as? String) == "Zed" && ($0["kCGWindowLayer"] as? Int) == 0 }
print(zed?["kCGWindowNumber"] as? Int ?? 0)
')"
  [[ "$window_id" != "0" ]] || { echo "error: no on-screen Zed window" >&2; exit 1; }
  for run in 1 2; do
    screencapture -x -o -l "$window_id" "$output_dir/zed-$run.png"
    sleep 1
  done
  noise zed
fi

printf 'Captured in %s\n' "$output_dir"
