#!/usr/bin/env python3
"""Keeps the installers' copies of packaging/installer/ui.sh identical to it.

Both installers ship as single files, so each carries the library between
its `# >>> ui.sh` and `# <<< ui.sh` lines. Without arguments this checks the
copies; with --write it replaces them.
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "packaging/installer/ui.sh"
TARGETS = (
    ROOT / "packaging/relay/install-control.sh",
    ROOT / "crates/yttt-control/install/relay.sh",
)
BEGIN = "# >>> ui.sh\n"
END = "# <<< ui.sh\n"
# Filled in when the scripts are published or served; the library must not
# contain them, or the copies would be changed too.
PLACEHOLDERS = ("__VERSION__", "__CONTROL_URL__", "__IMAGE__")


def library() -> str:
    text = SOURCE.read_text()
    if not text.startswith(BEGIN) or not text.endswith(END):
        raise ValueError(f"{SOURCE} must start with {BEGIN!r} and end with {END!r}")
    for placeholder in PLACEHOLDERS:
        if placeholder in text:
            raise ValueError(f"{SOURCE} must not mention {placeholder}")
    return text


def synced(target: str, text: str) -> str:
    start = target.find(BEGIN)
    end = target.find(END, start)
    if start < 0 or end < 0:
        raise ValueError("no ui.sh markers")
    return target[:start] + text + target[end + len(END) :]


def stale() -> list[Path]:
    text = library()
    return [path for path in TARGETS if synced(path.read_text(), text) != path.read_text()]


def main() -> int:
    if sys.argv[1:] == ["--write"]:
        text = library()
        for path in TARGETS:
            path.write_text(synced(path.read_text(), text))
        return 0
    if sys.argv[1:]:
        print("usage: sync_installer_ui.py [--write]", file=sys.stderr)
        return 2
    out_of_date = stale()
    for path in out_of_date:
        print(
            f"{path.relative_to(ROOT)} has an old copy of ui.sh; "
            "run python3 scripts/sync_installer_ui.py --write",
            file=sys.stderr,
        )
    return 1 if out_of_date else 0


if __name__ == "__main__":
    sys.exit(main())
