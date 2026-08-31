#!/usr/bin/env python3
"""Generate release notes and the client update manifest from a release checkout."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path

VERSION_PATTERN = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")
RELEASE_HEADING_PATTERN = re.compile(
    r"^## (?P<version>(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)) - \d{4}-\d{2}-\d{2}$"
)
ASSET_FILENAMES = {
    "macos-aarch64": "yttt-{version}-macos-aarch64.dmg",
    "windows-x86_64": "yttt-{version}-windows-x86_64-setup.exe",
    "linux-x86_64": "yttt-{version}-linux-x86_64.tar.gz",
}


def validate_version(version: str) -> tuple[int, int, int]:
    match = VERSION_PATTERN.fullmatch(version)
    if match is None:
        raise ValueError(f"version must be stable SemVer without a v prefix: {version!r}")
    return tuple(int(part) for part in match.groups())


def changelog_section(changelog: str, version: str) -> str:
    validate_version(version)
    lines = changelog.splitlines()
    starts = [
        index + 1
        for index, line in enumerate(lines)
        if (match := RELEASE_HEADING_PATTERN.fullmatch(line)) is not None
        and match.group("version") == version
    ]
    if not starts:
        raise ValueError(f"CHANGELOG.md has no release section for {version}")
    if len(starts) > 1:
        raise ValueError(f"CHANGELOG.md contains duplicate release sections for {version}")

    start = starts[0]
    end = len(lines)
    for index in range(start, len(lines)):
        if lines[index].startswith("## "):
            end = index
            break
    notes = "\n".join(lines[start:end]).strip()
    if not notes:
        raise ValueError(f"CHANGELOG.md release section for {version} is empty")
    return f"{notes}\n"


def validate_release_changelog(changelog: str, version: str) -> str:
    notes = changelog_section(changelog, version)
    lines = changelog.splitlines()
    unreleased_headings = [
        index for index, line in enumerate(lines) if line == "## Unreleased"
    ]
    if len(unreleased_headings) != 1:
        raise ValueError("CHANGELOG.md must contain exactly one Unreleased section")

    unreleased_heading = unreleased_headings[0]
    next_heading = next(
        (
            index
            for index in range(unreleased_heading + 1, len(lines))
            if lines[index].startswith("## ")
        ),
        None,
    )
    if next_heading is None:
        raise ValueError(f"CHANGELOG.md has no release after Unreleased for {version}")
    unreleased_body = "\n".join(
        lines[unreleased_heading + 1 : next_heading]
    ).strip()
    if unreleased_body:
        raise ValueError(
            "CHANGELOG.md Unreleased section is not empty; "
            f"archive it before publishing {version}"
        )
    match = RELEASE_HEADING_PATTERN.fullmatch(lines[next_heading])
    if match is None or match.group("version") != version:
        raise ValueError(
            f"CHANGELOG.md release {version} must be the first release after Unreleased"
        )
    return notes


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def build_update_manifest(
    version: str,
    tag: str,
    repository: str,
    dist_dir: Path,
    notes: str,
) -> dict[str, object]:
    validate_version(version)
    if tag != f"v{version}":
        raise ValueError(f"release tag {tag!r} does not match version {version!r}")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError(f"invalid GitHub repository name: {repository!r}")

    release_url = f"https://github.com/{repository}/releases/tag/{tag}"
    download_base = f"https://github.com/{repository}/releases/download/{tag}"
    assets: dict[str, dict[str, str]] = {}
    for platform, template in ASSET_FILENAMES.items():
        filename = template.format(version=version)
        path = dist_dir / filename
        if not path.is_file():
            raise ValueError(f"missing release asset: {path}")
        assets[platform] = {
            "url": f"{download_base}/{filename}",
            "sha256": sha256_file(path),
        }

    return {
        "schema": 1,
        "version": version,
        "releaseUrl": release_url,
        "notes": notes.rstrip(),
        "assets": assets,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--tag")
    parser.add_argument("--repository")
    parser.add_argument("--dist", type=Path)
    parser.add_argument("--changelog", type=Path, default=Path("CHANGELOG.md"))
    parser.add_argument("--notes-output", type=Path)
    parser.add_argument("--manifest-output", type=Path)
    parser.add_argument("--validate-changelog-only", action="store_true")
    args = parser.parse_args()

    changelog = args.changelog.read_text(encoding="utf-8")
    notes = validate_release_changelog(changelog, args.version)
    if args.validate_changelog_only:
        print(f"Validated CHANGELOG.md release section for {args.version}.")
        return 0

    required = {
        "--tag": args.tag,
        "--repository": args.repository,
        "--dist": args.dist,
        "--notes-output": args.notes_output,
        "--manifest-output": args.manifest_output,
    }
    for option, value in required.items():
        if value is None:
            parser.error(f"{option} is required unless --validate-changelog-only is used")

    manifest = build_update_manifest(
        args.version,
        args.tag,
        args.repository,
        args.dist,
        notes,
    )

    args.notes_output.write_text(notes, encoding="utf-8")
    args.manifest_output.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
