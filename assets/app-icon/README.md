# App Icon Assets

The production icon source is `source/yttt-icon.svg`: an angular Y with a separate
terminal prompt and underscore cursor on a dark rounded tile.

Install Python 3, librsvg (`rsvg-convert`), and ImageMagick (`magick`); on macOS,
use `brew install python librsvg imagemagick`.
Run `scripts/build-app-icons.sh` after editing the SVG. The script generates:

- `png/*.png` for fixed-size app and UI usage.
- `macos/AppIcon.icns` for the macOS app bundle.
- `windows/AppIcon.ico` for Windows packaging.
- `tray/*.rgba`: 36 × 36, 8-bit RGBA foreground-only tray icons. macOS uses the black
  template; Windows uses a neutral gray foreground.

The SVG is the source of truth. Its intermediate raster is written to
`target/app-icons/yttt-icon.png`; generated platform assets are checked in.
macOS `.icns` generation requires macOS with `iconutil` or the Ruby fallback.

`scripts/build-tray-icons.py` can regenerate only the tray assets. It extracts the foreground
paths from the same SVG and removes the tile and gradients; previews are written to
`target/app-icons/tray-{template,color}.png`. No runtime image decoder is required.
