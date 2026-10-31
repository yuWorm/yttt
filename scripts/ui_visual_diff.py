#!/usr/bin/env python3
"""Compare two UI screenshots region by region.

Exit status: 0 when every region passes or has an approved exception, 1 when
any region fails, 2 when the inputs or configuration are invalid. Uses only the
standard library so it runs unchanged in CI and on developer machines.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import tomllib
import zlib
from dataclasses import dataclass
from pathlib import Path


PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
CHANNELS_BY_COLOR_TYPE = {0: 1, 2: 3, 4: 2, 6: 4}


class ConfigError(Exception):
    """The comparison cannot run; distinct from a comparison that fails."""


@dataclass
class Image:
    width: int
    height: int
    pixels: bytearray  # Tightly packed RGB rows.


@dataclass(frozen=True)
class Region:
    name: str
    x: int
    y: int
    width: int
    height: int


@dataclass(frozen=True)
class ExceptionEntry:
    region: str
    reason: str
    approved_by: str


@dataclass(frozen=True)
class Thresholds:
    channel_delta: int  # A pixel changed when any channel differs by more than this.
    changed_ratio: float  # A region fails when its changed share exceeds this.


@dataclass
class RegionResult:
    name: str
    status: str  # pass, fail, or excepted.
    changed_pixels: int
    total_pixels: int
    changed_ratio: float
    max_channel_delta: int
    exception_reason: str | None = None


@dataclass
class Report:
    results: list[RegionResult]
    stale_exceptions: list[str]
    diff: Image

    @property
    def passed(self) -> bool:
        return all(result.status != "fail" for result in self.results)


def read_png(path: Path) -> Image:
    try:
        data = path.read_bytes()
    except OSError as error:
        raise ConfigError(f"cannot read {path}: {error}") from error
    if not data.startswith(PNG_SIGNATURE):
        raise ConfigError(f"{path} is not a PNG file")

    header: tuple[int, ...] | None = None
    compressed = bytearray()
    position = len(PNG_SIGNATURE)
    while position < len(data):
        length, kind = struct.unpack(">I4s", data[position : position + 8])
        payload = data[position + 8 : position + 8 + length]
        position += 12 + length
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", payload)
        elif kind == b"IDAT":
            compressed += payload
        elif kind == b"IEND":
            break
    if header is None:
        raise ConfigError(f"{path} has no IHDR chunk")

    width, height, depth, color_type, _, _, interlace = header
    if interlace != 0:
        raise ConfigError(f"{path} is interlaced, which is unsupported")
    if depth not in (8, 16) or color_type not in CHANNELS_BY_COLOR_TYPE:
        raise ConfigError(f"{path} uses unsupported depth {depth} or color type {color_type}")

    channels = CHANNELS_BY_COLOR_TYPE[color_type]
    sample_bytes = depth // 8
    rows = unfilter(zlib.decompress(bytes(compressed)), width, height, channels * sample_bytes)
    return Image(width, height, to_rgb(rows, width, channels, sample_bytes))


def unfilter(raw: bytes, width: int, height: int, pixel_bytes: int) -> list[bytearray]:
    stride = width * pixel_bytes
    previous = bytearray(stride)
    rows: list[bytearray] = []
    for row_index in range(height):
        start = row_index * (stride + 1)
        filter_type = raw[start]
        row = bytearray(raw[start + 1 : start + 1 + stride])
        if filter_type == 1:
            for i in range(pixel_bytes, stride):
                row[i] = (row[i] + row[i - pixel_bytes]) & 0xFF
        elif filter_type == 2:
            for i in range(stride):
                row[i] = (row[i] + previous[i]) & 0xFF
        elif filter_type == 3:
            for i in range(stride):
                left = row[i - pixel_bytes] if i >= pixel_bytes else 0
                row[i] = (row[i] + ((left + previous[i]) >> 1)) & 0xFF
        elif filter_type == 4:
            for i in range(stride):
                left = row[i - pixel_bytes] if i >= pixel_bytes else 0
                upper_left = previous[i - pixel_bytes] if i >= pixel_bytes else 0
                row[i] = (row[i] + paeth(left, previous[i], upper_left)) & 0xFF
        elif filter_type != 0:
            raise ConfigError(f"unknown PNG filter {filter_type}")
        rows.append(row)
        previous = row
    return rows


def paeth(left: int, up: int, upper_left: int) -> int:
    estimate = left + up - upper_left
    left_distance = abs(estimate - left)
    up_distance = abs(estimate - up)
    upper_left_distance = abs(estimate - upper_left)
    if left_distance <= up_distance and left_distance <= upper_left_distance:
        return left
    return up if up_distance <= upper_left_distance else upper_left


def to_rgb(rows: list[bytearray], width: int, channels: int, sample_bytes: int) -> bytearray:
    # Sixteen-bit samples are big-endian, so slicing every other byte keeps the high byte.
    pixels = bytearray(width * len(rows) * 3)
    for y, row in enumerate(rows):
        samples = row[::sample_bytes]
        target = y * width * 3
        if channels >= 3:
            pixels[target : target + width * 3 : 3] = samples[0::channels]
            pixels[target + 1 : target + width * 3 : 3] = samples[1::channels]
            pixels[target + 2 : target + width * 3 : 3] = samples[2::channels]
        else:
            gray = samples[0::channels]
            pixels[target : target + width * 3 : 3] = gray
            pixels[target + 1 : target + width * 3 : 3] = gray
            pixels[target + 2 : target + width * 3 : 3] = gray
    return pixels


def write_png(path: Path, image: Image) -> None:
    def chunk(kind: bytes, payload: bytes) -> bytes:
        checksum = zlib.crc32(kind + payload)
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", checksum)

    stride = image.width * 3
    raw = bytearray()
    for y in range(image.height):
        raw.append(0)
        raw += image.pixels[y * stride : (y + 1) * stride]
    header = struct.pack(">IIBBBBB", image.width, image.height, 8, 2, 0, 0, 0)
    path.write_bytes(
        PNG_SIGNATURE
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(bytes(raw)))
        + chunk(b"IEND", b"")
    )


def compare(
    baseline: Image,
    actual: Image,
    regions: list[Region],
    exceptions: dict[str, ExceptionEntry],
    thresholds: Thresholds,
) -> Report:
    if (baseline.width, baseline.height) != (actual.width, actual.height):
        raise ConfigError(
            f"image sizes differ: {baseline.width}x{baseline.height} "
            f"vs {actual.width}x{actual.height}"
        )

    diff = Image(baseline.width, baseline.height, bytearray(baseline.pixels))
    results = []
    stale_exceptions = []
    for region in regions:
        if (
            region.x < 0
            or region.y < 0
            or region.width <= 0
            or region.height <= 0
            or region.x + region.width > baseline.width
            or region.y + region.height > baseline.height
        ):
            raise ConfigError(f"region {region.name} is outside the image")

        result = compare_region(baseline, actual, region, thresholds, diff)
        exception = exceptions.get(region.name)
        if exception is not None:
            if result.status == "fail":
                result.status = "excepted"
                result.exception_reason = exception.reason
            else:
                stale_exceptions.append(region.name)
        results.append(result)
    return Report(results, stale_exceptions, diff)


def compare_region(
    baseline: Image, actual: Image, region: Region, thresholds: Thresholds, diff: Image
) -> RegionResult:
    stride = baseline.width * 3
    span = region.width * 3
    changed = 0
    max_delta = 0
    for y in range(region.y, region.y + region.height):
        start = y * stride + region.x * 3
        expected_row = baseline.pixels[start : start + span]
        actual_row = actual.pixels[start : start + span]
        if expected_row == actual_row:
            continue
        for offset in range(0, span, 3):
            delta = max(
                abs(expected_row[offset] - actual_row[offset]),
                abs(expected_row[offset + 1] - actual_row[offset + 1]),
                abs(expected_row[offset + 2] - actual_row[offset + 2]),
            )
            max_delta = max(max_delta, delta)
            if delta > thresholds.channel_delta:
                changed += 1
                diff.pixels[start + offset : start + offset + 3] = b"\xff\x00\x00"

    total = region.width * region.height
    ratio = changed / total
    return RegionResult(
        name=region.name,
        status="fail" if ratio > thresholds.changed_ratio else "pass",
        changed_pixels=changed,
        total_pixels=total,
        changed_ratio=ratio,
        max_channel_delta=max_delta,
    )


def load_toml(path: Path) -> dict:
    try:
        with path.open("rb") as file:
            return tomllib.load(file)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ConfigError(f"cannot load {path}: {error}") from error


def load_regions(path: Path) -> list[Region]:
    regions = []
    names = set()
    for entry in load_toml(path).get("region", []):
        try:
            region = Region(
                entry["name"], entry["x"], entry["y"], entry["width"], entry["height"]
            )
        except KeyError as error:
            raise ConfigError(f"{path}: region is missing {error}") from error
        if region.name in names:
            raise ConfigError(f"{path}: duplicate region {region.name}")
        names.add(region.name)
        regions.append(region)
    if not regions:
        raise ConfigError(f"{path} defines no regions")
    return regions


def load_exceptions(path: Path) -> dict[str, ExceptionEntry]:
    exceptions = {}
    for entry in load_toml(path).get("exception", []):
        region = str(entry.get("region", "")).strip()
        reason = str(entry.get("reason", "")).strip()
        approved_by = str(entry.get("approved_by", "")).strip()
        if not region or not reason or not approved_by:
            raise ConfigError(f"{path}: exception needs region, reason, and approved_by")
        exceptions[region] = ExceptionEntry(region, reason, approved_by)
    return exceptions


def load_thresholds(path: Path) -> Thresholds:
    try:
        section = load_toml(path)["thresholds"]
        return Thresholds(section["channel_delta"], section["changed_ratio"])
    except KeyError as error:
        raise ConfigError(f"{path}: missing threshold {error}") from error


def parse_arguments(arguments: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Compare UI screenshots by region.")
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--actual", type=Path, required=True)
    parser.add_argument("--regions", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True, help="provides [thresholds]")
    parser.add_argument("--exceptions", type=Path)
    parser.add_argument("--output-dir", type=Path, required=True)
    return parser.parse_args(arguments)


def main(arguments: list[str] | None = None) -> int:
    options = parse_arguments(arguments)
    try:
        report = compare(
            read_png(options.baseline),
            read_png(options.actual),
            load_regions(options.regions),
            load_exceptions(options.exceptions) if options.exceptions else {},
            load_thresholds(options.manifest),
        )
    except ConfigError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    options.output_dir.mkdir(parents=True, exist_ok=True)
    write_png(options.output_dir / "diff.png", report.diff)
    (options.output_dir / "report.json").write_text(
        json.dumps(
            {
                "passed": report.passed,
                "stale_exceptions": report.stale_exceptions,
                "regions": [vars(result) for result in report.results],
            },
            indent=2,
            ensure_ascii=False,
        )
        + "\n",
        encoding="utf-8",
    )
    for result in report.results:
        print(
            f"{result.status:8} {result.name}: {result.changed_pixels}/{result.total_pixels} "
            f"changed, max channel delta {result.max_channel_delta}"
        )
    return 0 if report.passed else 1


if __name__ == "__main__":
    sys.exit(main())
