#!/usr/bin/env python3
"""Prepare a release commit without committing, tagging, or pushing it."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import subprocess
from pathlib import Path

from release_metadata import changelog_section, validate_version

WORKSPACE_PACKAGE_PATTERN = re.compile(
    r"(?ms)(^\[workspace\.package\]\s*$.*?^version\s*=\s*\")([^\"]+)(\"\s*$)"
)
UNRELEASED_HEADING = "## Unreleased"


def replace_workspace_version(manifest: str, version: str) -> tuple[str, str]:
    validate_version(version)
    match = WORKSPACE_PACKAGE_PATTERN.search(manifest)
    if match is None:
        raise ValueError("Cargo.toml has no version in [workspace.package]")
    current = match.group(2)
    updated = manifest[: match.start(2)] + version + manifest[match.end(2) :]
    return updated, current


def archive_unreleased(changelog: str, version: str, release_date: str) -> str:
    validate_version(version)
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", release_date):
        raise ValueError(f"invalid release date: {release_date!r}")
    if re.search(rf"(?m)^## {re.escape(version)} - ", changelog):
        raise ValueError(f"CHANGELOG.md already contains release {version}")

    lines = changelog.splitlines()
    try:
        heading = lines.index(UNRELEASED_HEADING)
    except ValueError as error:
        raise ValueError("CHANGELOG.md has no Unreleased section") from error

    end = len(lines)
    for index in range(heading + 1, len(lines)):
        if lines[index].startswith("## "):
            end = index
            break
    unreleased_body = "\n".join(lines[heading + 1 : end]).strip()
    if not unreleased_body:
        raise ValueError("CHANGELOG.md Unreleased section is empty")

    replacement = [
        UNRELEASED_HEADING,
        "",
        f"## {version} - {release_date}",
    ]
    updated = lines[:heading] + replacement + lines[heading + 1 :]
    return "\n".join(updated).rstrip() + "\n"


def run_cargo_metadata(repo_root: Path, *, locked: bool) -> dict[str, object]:
    command = ["cargo", "metadata", "--format-version", "1", "--no-deps"]
    if locked:
        command.append("--locked")
    completed = subprocess.run(
        command,
        cwd=repo_root,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(completed.stdout)


def verify_workspace_versions(metadata: dict[str, object], version: str) -> None:
    packages = metadata.get("packages")
    if not isinstance(packages, list):
        raise ValueError("cargo metadata returned no packages")
    mismatches = [
        f"{package['name']}={package['version']}"
        for package in packages
        if isinstance(package, dict)
        and str(package.get("name", "")).startswith("yttt")
        and package.get("version") != version
    ]
    if mismatches:
        raise ValueError("workspace version mismatch: " + ", ".join(mismatches))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("version", help="stable SemVer without a v prefix")
    parser.add_argument(
        "--date",
        default=dt.datetime.now(dt.timezone.utc).date().isoformat(),
        help="release date in YYYY-MM-DD form",
    )
    parser.add_argument(
        "--repo",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help=argparse.SUPPRESS,
    )
    args = parser.parse_args()

    target = validate_version(args.version)
    repo_root = args.repo.resolve()
    manifest_path = repo_root / "Cargo.toml"
    changelog_path = repo_root / "CHANGELOG.md"
    lock_path = repo_root / "Cargo.lock"

    original_manifest = manifest_path.read_text(encoding="utf-8")
    original_changelog = changelog_path.read_text(encoding="utf-8")
    original_lock = lock_path.read_bytes()
    updated_manifest, current_version = replace_workspace_version(
        original_manifest, args.version
    )
    if target <= validate_version(current_version):
        raise ValueError(
            f"release version {args.version} must be newer than {current_version}"
        )
    updated_changelog = archive_unreleased(
        original_changelog, args.version, args.date
    )

    try:
        manifest_path.write_text(updated_manifest, encoding="utf-8")
        changelog_path.write_text(updated_changelog, encoding="utf-8")
        run_cargo_metadata(repo_root, locked=False)
        metadata = run_cargo_metadata(repo_root, locked=True)
        verify_workspace_versions(metadata, args.version)
        changelog_section(updated_changelog, args.version)
    except BaseException:
        manifest_path.write_text(original_manifest, encoding="utf-8")
        changelog_path.write_text(original_changelog, encoding="utf-8")
        lock_path.write_bytes(original_lock)
        raise

    print(f"Prepared yttt {args.version} ({args.date}).")
    print("Review Cargo.toml, Cargo.lock, and CHANGELOG.md, then commit and push.")
    print("Create and push the annotated tag only after the three-platform build succeeds.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
