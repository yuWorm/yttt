#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import struct
import sys
import tempfile
import unittest
import zlib
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("ui_visual_diff.py")
SPEC = importlib.util.spec_from_file_location("ui_visual_diff", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
visual_diff = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = visual_diff
SPEC.loader.exec_module(visual_diff)

WHITE = (255, 255, 255)
BLACK = (0, 0, 0)


def solid(width: int, height: int, color: tuple[int, int, int]) -> "visual_diff.Image":
    return visual_diff.Image(width, height, bytearray(bytes(color) * (width * height)))


def set_pixel(image: "visual_diff.Image", x: int, y: int, color: tuple[int, int, int]) -> None:
    offset = (y * image.width + x) * 3
    image.pixels[offset : offset + 3] = bytes(color)


def region(name: str, x: int, y: int, width: int, height: int) -> "visual_diff.Region":
    return visual_diff.Region(name, x, y, width, height)


class ImageCodecTests(unittest.TestCase):
    def test_png_round_trip_preserves_pixels(self) -> None:
        image = solid(3, 2, WHITE)
        set_pixel(image, 1, 1, (10, 20, 30))
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "image.png"
            visual_diff.write_png(path, image)
            decoded = visual_diff.read_png(path)
        self.assertEqual((decoded.width, decoded.height), (3, 2))
        self.assertEqual(decoded.pixels, image.pixels)

    def test_reader_applies_every_png_filter_and_drops_alpha(self) -> None:
        # One RGBA scanline per filter type; each decodes to (10, 20, 30, 255) then +1 per row.
        raw = bytearray()
        raw += bytes([0, 10, 20, 30, 255, 10, 20, 30, 255])  # None
        raw += bytes([1, 11, 21, 31, 255, 0, 0, 0, 0])  # Sub
        raw += bytes([2, 1, 1, 1, 0, 1, 1, 1, 0])  # Up
        raw += bytes([3, 7, 12, 17, 128, 1, 1, 1, 0])  # Average
        raw += bytes([4, 1, 1, 1, 0, 0, 0, 0, 0])  # Paeth
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "filters.png"
            path.write_bytes(encode_png(2, 5, 8, 6, bytes(raw)))
            decoded = visual_diff.read_png(path)
        rows = [bytes(decoded.pixels[y * 6 : y * 6 + 6]) for y in range(5)]
        self.assertEqual(
            rows,
            [
                bytes([10, 20, 30] * 2),
                bytes([11, 21, 31] * 2),
                bytes([12, 22, 32] * 2),
                bytes([13, 23, 33] * 2),
                bytes([14, 24, 34] * 2),
            ],
        )

    def test_reader_rejects_interlaced_png(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "interlaced.png"
            path.write_bytes(encode_png(1, 1, 8, 2, bytes([0, 1, 2, 3]), interlace=1))
            with self.assertRaises(visual_diff.ConfigError):
                visual_diff.read_png(path)


class CompareTests(unittest.TestCase):
    def test_identical_images_pass(self) -> None:
        report = visual_diff.compare(
            solid(4, 4, WHITE),
            solid(4, 4, WHITE),
            [region("all", 0, 0, 4, 4)],
            {},
            visual_diff.Thresholds(0, 0.0),
        )
        self.assertTrue(report.passed)
        self.assertEqual(report.results[0].changed_pixels, 0)
        self.assertEqual(report.results[0].status, "pass")

    def test_single_pixel_difference_fails_at_zero_tolerance(self) -> None:
        actual = solid(4, 4, WHITE)
        set_pixel(actual, 2, 1, BLACK)
        report = visual_diff.compare(
            solid(4, 4, WHITE),
            actual,
            [region("all", 0, 0, 4, 4)],
            {},
            visual_diff.Thresholds(0, 0.0),
        )
        self.assertFalse(report.passed)
        self.assertEqual(report.results[0].changed_pixels, 1)
        self.assertEqual(report.results[0].max_channel_delta, 255)
        self.assertAlmostEqual(report.results[0].changed_ratio, 1 / 16)

    def test_channel_delta_at_threshold_is_not_a_change(self) -> None:
        actual = solid(2, 2, (100, 100, 100))
        set_pixel(actual, 0, 0, (116, 100, 100))
        report = visual_diff.compare(
            solid(2, 2, (100, 100, 100)),
            actual,
            [region("all", 0, 0, 2, 2)],
            {},
            visual_diff.Thresholds(16, 0.0),
        )
        self.assertTrue(report.passed)
        self.assertEqual(report.results[0].max_channel_delta, 16)

    def test_ratio_at_threshold_passes_and_above_fails(self) -> None:
        actual = solid(10, 10, WHITE)
        set_pixel(actual, 0, 0, BLACK)
        names = [region("all", 0, 0, 10, 10)]
        at_limit = visual_diff.compare(
            solid(10, 10, WHITE), actual, names, {}, visual_diff.Thresholds(16, 0.01)
        )
        self.assertTrue(at_limit.passed)
        set_pixel(actual, 1, 0, BLACK)
        above_limit = visual_diff.compare(
            solid(10, 10, WHITE), actual, names, {}, visual_diff.Thresholds(16, 0.01)
        )
        self.assertFalse(above_limit.passed)

    def test_region_crop_ignores_differences_outside_it(self) -> None:
        actual = solid(4, 4, WHITE)
        set_pixel(actual, 3, 3, BLACK)
        report = visual_diff.compare(
            solid(4, 4, WHITE),
            actual,
            [region("top-left", 0, 0, 2, 2), region("bottom-right", 2, 2, 2, 2)],
            {},
            visual_diff.Thresholds(0, 0.0),
        )
        statuses = {result.name: result.status for result in report.results}
        self.assertEqual(statuses, {"top-left": "pass", "bottom-right": "fail"})

    def test_size_mismatch_is_a_config_error(self) -> None:
        with self.assertRaises(visual_diff.ConfigError):
            visual_diff.compare(
                solid(4, 4, WHITE),
                solid(4, 5, WHITE),
                [region("all", 0, 0, 4, 4)],
                {},
                visual_diff.Thresholds(0, 0.0),
            )

    def test_region_outside_image_is_a_config_error(self) -> None:
        with self.assertRaises(visual_diff.ConfigError):
            visual_diff.compare(
                solid(4, 4, WHITE),
                solid(4, 4, WHITE),
                [region("wide", 2, 0, 3, 1)],
                {},
                visual_diff.Thresholds(0, 0.0),
            )

    def test_exception_turns_failure_into_excepted_and_keeps_counts(self) -> None:
        actual = solid(4, 4, WHITE)
        set_pixel(actual, 0, 0, BLACK)
        exception = visual_diff.ExceptionEntry("all", "font hinting", "yuWorm")
        report = visual_diff.compare(
            solid(4, 4, WHITE),
            actual,
            [region("all", 0, 0, 4, 4)],
            {"all": exception},
            visual_diff.Thresholds(0, 0.0),
        )
        self.assertTrue(report.passed)
        self.assertEqual(report.results[0].status, "excepted")
        self.assertEqual(report.results[0].changed_pixels, 1)
        self.assertEqual(report.results[0].exception_reason, "font hinting")

    def test_passing_region_with_exception_reports_stale_exception(self) -> None:
        exception = visual_diff.ExceptionEntry("all", "font hinting", "yuWorm")
        report = visual_diff.compare(
            solid(4, 4, WHITE),
            solid(4, 4, WHITE),
            [region("all", 0, 0, 4, 4)],
            {"all": exception},
            visual_diff.Thresholds(0, 0.0),
        )
        self.assertEqual(report.stale_exceptions, ["all"])


class ConfigTests(unittest.TestCase):
    def write(self, name: str, content: str) -> Path:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / name
        path.write_text(content, encoding="utf-8")
        return path

    def test_regions_load_and_reject_duplicates(self) -> None:
        path = self.write(
            "regions.toml",
            '[[region]]\nname = "button"\nx = 1\ny = 2\nwidth = 3\nheight = 4\n',
        )
        self.assertEqual(visual_diff.load_regions(path), [region("button", 1, 2, 3, 4)])
        duplicate = self.write(
            "regions.toml",
            '[[region]]\nname = "a"\nx = 0\ny = 0\nwidth = 1\nheight = 1\n'
            '[[region]]\nname = "a"\nx = 1\ny = 0\nwidth = 1\nheight = 1\n',
        )
        with self.assertRaises(visual_diff.ConfigError):
            visual_diff.load_regions(duplicate)

    def test_exception_requires_reason_and_approver(self) -> None:
        valid = self.write(
            "exceptions.toml",
            '[[exception]]\nregion = "a"\nreason = "r"\napproved_by = "p"\n',
        )
        self.assertEqual(
            visual_diff.load_exceptions(valid),
            {"a": visual_diff.ExceptionEntry("a", "r", "p")},
        )
        for body in (
            '[[exception]]\nregion = "a"\napproved_by = "p"\n',
            '[[exception]]\nregion = "a"\nreason = "r"\n',
            '[[exception]]\nregion = "a"\nreason = " "\napproved_by = "p"\n',
        ):
            with self.subTest(body=body), self.assertRaises(visual_diff.ConfigError):
                visual_diff.load_exceptions(self.write("exceptions.toml", body))

    def test_manifest_thresholds_load(self) -> None:
        path = self.write(
            "manifest.toml", "[thresholds]\nchannel_delta = 8\nchanged_ratio = 0.002\n"
        )
        self.assertEqual(visual_diff.load_thresholds(path), visual_diff.Thresholds(8, 0.002))


class CommandLineTests(unittest.TestCase):
    def test_writes_report_and_diff_image_and_exit_code_reflects_result(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            baseline = root / "baseline.png"
            actual = root / "actual.png"
            regions = root / "regions.toml"
            manifest = root / "manifest.toml"
            output = root / "out"
            changed = solid(4, 4, WHITE)
            set_pixel(changed, 1, 1, BLACK)
            visual_diff.write_png(baseline, solid(4, 4, WHITE))
            visual_diff.write_png(actual, changed)
            regions.write_text(
                '[[region]]\nname = "all"\nx = 0\ny = 0\nwidth = 4\nheight = 4\n',
                encoding="utf-8",
            )
            manifest.write_text(
                "[thresholds]\nchannel_delta = 0\nchanged_ratio = 0.0\n", encoding="utf-8"
            )
            arguments = [
                "--baseline",
                str(baseline),
                "--actual",
                str(actual),
                "--regions",
                str(regions),
                "--manifest",
                str(manifest),
                "--output-dir",
                str(output),
            ]

            self.assertEqual(visual_diff.main(arguments), 1)
            report = json.loads((output / "report.json").read_text(encoding="utf-8"))
            self.assertFalse(report["passed"])
            self.assertEqual(report["regions"][0]["changed_pixels"], 1)
            diff = visual_diff.read_png(output / "diff.png")
            self.assertEqual(bytes(diff.pixels[(1 * 4 + 1) * 3 :][:3]), bytes((255, 0, 0)))

            arguments[3] = str(baseline)
            self.assertEqual(visual_diff.main(arguments), 0)

    def test_config_error_exits_two(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = visual_diff.main(
                [
                    "--baseline",
                    str(root / "missing.png"),
                    "--actual",
                    str(root / "missing.png"),
                    "--regions",
                    str(root / "missing.toml"),
                    "--manifest",
                    str(root / "missing.toml"),
                    "--output-dir",
                    str(root / "out"),
                ]
            )
        self.assertEqual(result, 2)


def encode_png(
    width: int, height: int, depth: int, color_type: int, raw: bytes, interlace: int = 0
) -> bytes:
    def chunk(kind: bytes, payload: bytes) -> bytes:
        checksum = zlib.crc32(kind + payload)
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", checksum)

    header = struct.pack(">IIBBBBB", width, height, depth, color_type, 0, 0, interlace)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


if __name__ == "__main__":
    unittest.main()
