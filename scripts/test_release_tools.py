#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import tempfile
import unittest
from pathlib import Path

from prepare_release import (
    archive_unreleased,
    repair_existing_release,
    replace_workspace_version,
)
from release_metadata import (
    build_update_manifest,
    changelog_section,
    validate_release_changelog,
    validate_version,
)


class ReleaseMetadataTests(unittest.TestCase):
    def test_stable_semver_validation_rejects_tags_and_prereleases(self) -> None:
        self.assertEqual(validate_version("1.2.3"), (1, 2, 3))
        for invalid in ("v1.2.3", "1.2", "1.2.3-beta.1", "01.2.3"):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                validate_version(invalid)

    def test_changelog_section_extracts_only_requested_release(self) -> None:
        changelog = """# Changelog

## Unreleased

### Added

- Future change.

## 1.2.3 - 2026-07-20

Release introduction.

### Fixed

- A regression.

## 1.2.2 - 2026-07-19

- Previous change.
"""
        self.assertEqual(
            changelog_section(changelog, "1.2.3"),
            "Release introduction.\n\n### Fixed\n\n- A regression.\n",
        )

    def test_release_changelog_requires_empty_unreleased_and_current_section(
        self,
    ) -> None:
        with self.assertRaisesRegex(ValueError, "Unreleased section is not empty"):
            validate_release_changelog(
                """# Changelog

## Unreleased

- A change that has not been archived.

## 1.2.3 - 2026-07-20

- Old release notes.
""",
                "1.2.3",
            )

        with self.assertRaisesRegex(
            ValueError, "must be the first release after Unreleased"
        ):
            validate_release_changelog(
                """# Changelog

## Unreleased

## 1.2.4 - 2026-07-21

- Newer release.

## 1.2.3 - 2026-07-20

- Requested old release.
""",
                "1.2.3",
            )

    def test_update_manifest_requires_and_hashes_every_platform_asset(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            dist = Path(temporary_directory)
            filenames = {
                "yttt-1.2.3-macos-aarch64.dmg": b"macos",
                "yttt-1.2.3-windows-x86_64-setup.exe": b"windows",
                "yttt-1.2.3-linux-x86_64.tar.gz": b"linux",
            }
            for filename, contents in filenames.items():
                (dist / filename).write_bytes(contents)

            manifest = build_update_manifest(
                "1.2.3",
                "v1.2.3",
                "yuWorm/yttt",
                dist,
                "Release notes.\n",
            )

            self.assertEqual(manifest["schema"], 1)
            self.assertEqual(manifest["version"], "1.2.3")
            self.assertEqual(
                manifest["releaseUrl"],
                "https://github.com/yuWorm/yttt/releases/tag/v1.2.3",
            )
            assets = manifest["assets"]
            self.assertEqual(
                assets["windows-x86_64"]["sha256"],
                hashlib.sha256(b"windows").hexdigest(),
            )
            self.assertTrue(
                assets["linux-x86_64"]["url"].endswith(
                    "/yttt-1.2.3-linux-x86_64.tar.gz"
                )
            )


class PrepareReleaseTests(unittest.TestCase):
    def test_workspace_version_is_replaced_only_in_workspace_package(self) -> None:
        manifest = """[workspace]
members = []

[workspace.package]
version = "1.0.0"
edition = "2024"

[package]
name = "yttt"
version.workspace = true
"""
        updated, previous = replace_workspace_version(manifest, "1.1.0")
        self.assertEqual(previous, "1.0.0")
        self.assertIn('version = "1.1.0"', updated)
        self.assertIn("version.workspace = true", updated)

    def test_archive_unreleased_preserves_entries_and_opens_next_section(self) -> None:
        changelog = """# Changelog

## Unreleased

Release introduction.

### Added

- Update checks.

## 1.0.0 - 2026-07-18

- Initial release.
"""
        updated = archive_unreleased(changelog, "1.1.0", "2026-07-20")
        self.assertIn(
            "## Unreleased\n\n## 1.1.0 - 2026-07-20\n\nRelease introduction.",
            updated,
        )
        self.assertEqual(
            changelog_section(updated, "1.1.0"),
            "Release introduction.\n\n### Added\n\n- Update checks.\n",
        )

    def test_archive_unreleased_rejects_empty_release_notes(self) -> None:
        with self.assertRaisesRegex(ValueError, "Unreleased section is empty"):
            archive_unreleased(
                "# Changelog\n\n## Unreleased\n\n## 1.0.0 - 2026-07-18\n",
                "1.1.0",
                "2026-07-20",
            )

    def test_archive_unreleased_rejects_an_existing_release(self) -> None:
        with self.assertRaisesRegex(ValueError, "already contains release 1.1.0"):
            archive_unreleased(
                """# Changelog

## Unreleased

- New change.

## 1.1.0 - 2026-07-19

- Existing notes.
""",
                "1.1.0",
                "2026-07-20",
            )

    def test_repair_existing_release_merges_unreleased_without_duplicate_heading(
        self,
    ) -> None:
        changelog = """# Changelog

## Unreleased

- New Host changes.

## 1.1.0 - 2026-07-19

- Existing SSH changes.

## 1.0.0 - 2026-07-18

- Initial release.
"""
        repaired = repair_existing_release(changelog, "1.1.0", "2026-07-20")
        self.assertEqual(repaired.count("## 1.1.0 - "), 1)
        self.assertIn(
            "## Unreleased\n\n## 1.1.0 - 2026-07-20\n\n"
            "- New Host changes.\n\n- Existing SSH changes.",
            repaired,
        )
        self.assertEqual(
            validate_release_changelog(repaired, "1.1.0"),
            "- New Host changes.\n\n- Existing SSH changes.\n",
        )


if __name__ == "__main__":
    unittest.main()
