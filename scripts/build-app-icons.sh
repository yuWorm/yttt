#!/usr/bin/env bash
set -euo pipefail



repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_png="$repo_root/assets/app-icon/source/yttt-icon.png"
png_dir="$repo_root/assets/app-icon/png"
macos_dir="$repo_root/assets/app-icon/macos"
windows_dir="$repo_root/assets/app-icon/windows"
iconset="$repo_root/target/app-icons/AppIcon.iconset"
icns="$macos_dir/AppIcon.icns"
ico="$windows_dir/AppIcon.ico"

if [[ ! -f "$source_png" ]]; then
  echo "Missing app icon source PNG: $source_png" >&2
  exit 1
fi

if command -v magick >/dev/null 2>&1; then
  render_png() {
    local size="$1"
    local out="$2"
    magick "$source_png" -resize "${size}x${size}" "PNG32:$out"
  }
else
  echo "Missing ImageMagick (magick), required to build app icons from the PNG source." >&2
  exit 1
fi

mkdir -p "$png_dir" "$macos_dir" "$windows_dir" "$(dirname "$iconset")"

for size in 16 32 48 64 128 256 512 1024; do
  render_png "$size" "$png_dir/$size.png"
done

if command -v magick >/dev/null 2>&1; then
  magick "$png_dir/16.png" "$png_dir/32.png" "$png_dir/48.png" "$png_dir/64.png" \
    "$png_dir/128.png" "$png_dir/256.png" "$ico"
else
  echo "Missing ImageMagick (magick), required to build Windows .ico." >&2
  exit 1
fi

if [[ "$(uname -s)" == "Darwin" ]]; then
  rm -rf "$iconset"
  mkdir -p "$iconset"
  cp "$png_dir/16.png" "$iconset/icon_16x16.png"
  cp "$png_dir/32.png" "$iconset/icon_16x16@2x.png"
  cp "$png_dir/32.png" "$iconset/icon_32x32.png"
  cp "$png_dir/64.png" "$iconset/icon_32x32@2x.png"
  cp "$png_dir/128.png" "$iconset/icon_128x128.png"
  cp "$png_dir/256.png" "$iconset/icon_128x128@2x.png"
  cp "$png_dir/256.png" "$iconset/icon_256x256.png"
  cp "$png_dir/512.png" "$iconset/icon_256x256@2x.png"
  cp "$png_dir/512.png" "$iconset/icon_512x512.png"
  cp "$png_dir/1024.png" "$iconset/icon_512x512@2x.png"

  if command -v iconutil >/dev/null 2>&1 && iconutil --convert icns --output "$icns" "$iconset" 2>/dev/null; then
    :
  elif command -v ruby >/dev/null 2>&1; then
    ruby -e '
      pairs = [
        ["icp4", ARGV[0]],
        ["icp5", ARGV[1]],
        ["icp6", ARGV[2]],
        ["ic07", ARGV[3]],
        ["ic08", ARGV[4]],
        ["ic09", ARGV[5]],
        ["ic10", ARGV[6]],
      ]
      chunks = pairs.map do |code, path|
        data = File.binread(path)
        code + [data.bytesize + 8].pack("N") + data
      end
      total = 8 + chunks.sum(&:bytesize)
      File.binwrite(ARGV[7], "icns" + [total].pack("N") + chunks.join)
    ' "$png_dir/16.png" "$png_dir/32.png" "$png_dir/64.png" "$png_dir/128.png" \
      "$png_dir/256.png" "$png_dir/512.png" "$png_dir/1024.png" "$icns"
  else
    echo "Missing iconutil fallback: install Ruby or use a working iconutil." >&2
    exit 1
  fi
fi

printf '%s\n%s\n' "$icns" "$ico"
