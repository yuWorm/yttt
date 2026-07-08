# App Icon Assets

The production icon source is `source/yttt-icon.png`.

Run `scripts/build-app-icons.sh` after replacing the source image. The script generates:

- `png/*.png` for fixed-size app and UI usage.
- `macos/AppIcon.icns` for the macOS app bundle.
- `windows/AppIcon.ico` for Windows packaging.

The source is intentionally raster-first so the icon can keep the subtle desktop-app
material, shadow, and split-pane details from the final visual direction.
