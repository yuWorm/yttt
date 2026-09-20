#!/usr/bin/env python3
"""Extract the app icon foreground and render transparent native tray assets."""
from pathlib import Path
import subprocess
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "assets/app-icon/source/yttt-icon.svg"
OUTPUT = ROOT / "assets/app-icon/tray"
TEMP = ROOT / "target/app-icons"
SIZE = 36


def main():
    source = ET.parse(SOURCE).getroot()
    ns = {"svg": "http://www.w3.org/2000/svg"}
    foreground = source.find("svg:g/svg:g", ns)
    if foreground is None:
        raise SystemExit("App icon foreground group is missing")
    paths = list(foreground.findall("svg:path", ns))
    if not paths:
        raise SystemExit("App icon foreground paths are missing")
    OUTPUT.mkdir(parents=True, exist_ok=True)
    TEMP.mkdir(parents=True, exist_ok=True)
    ET.register_namespace("", ns["svg"])
    # Original foreground coordinates; centered with breathing room for the menu bar.
    svg = ET.Element("{http://www.w3.org/2000/svg}svg", {
        "width": str(SIZE), "height": str(SIZE), "viewBox": "40 39 223 205",
    })
    for path in paths:
        svg.append(path)
    for name, color in [("template", "#000000"), ("color", "#747980")]:
        svg.set("fill", color)
        svg_path = TEMP / f"tray-{name}.svg"
        ET.ElementTree(svg).write(svg_path, encoding="unicode")
        png_path = TEMP / f"tray-{name}.png"
        subprocess.run(["rsvg-convert", "--output", str(png_path), str(svg_path)], check=True)
        subprocess.run([
            "magick", str(png_path), "-depth", "8", f"rgba:{OUTPUT / (name + '.rgba')}",
        ], check=True)
        if (OUTPUT / (name + ".rgba")).stat().st_size != SIZE * SIZE * 4:
            raise SystemExit(f"Invalid RGBA size: {name}")


if __name__ == "__main__":
    main()
