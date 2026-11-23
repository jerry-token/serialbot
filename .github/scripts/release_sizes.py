# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
"""Prints the download sizes section of the release notes.

Reads the release as JSON (`gh api repos/OWNER/REPO/releases/ID`) on
stdin and prints a Markdown table of each file and its size, so the notes
always state the sizes of the files actually published. Sizes use binary
megabytes with one decimal, as GitHub shows them on the release page.

>>> print(section({"assets": [
...     {"name": "serialbot_windows_x64.exe", "size": 17664512},
...     {"name": "SerialBot_0.1.0_x64-setup.exe", "size": 3857299},
...     {"name": "latest.json", "size": 900},
... ]}))
### Sizes / 大小
<BLANKLINE>
| File | Size |
|---|---|
| `SerialBot_0.1.0_x64-setup.exe` | 3.7 MB |
| `serialbot_windows_x64.exe` | 16.8 MB |
>>> section({"assets": []})
''
"""

import json
import sys

# Files people download; signatures and updater manifests are left out.
SUFFIXES = (".exe", ".dmg", ".app.tar.gz", ".AppImage", ".deb", ".rpm", ".msi")


def megabytes(size: int) -> str:
    return f"{size / (1024 * 1024):.1f} MB"


def section(release: dict) -> str:
    assets = [a for a in release.get("assets", []) if a["name"].endswith(SUFFIXES)]
    if not assets:
        return ""
    rows = [f"| `{a['name']}` | {megabytes(a['size'])} |" for a in sorted(assets, key=lambda a: a["size"])]
    return "\n".join(["### Sizes / 大小", "", "| File | Size |", "|---|---|", *rows])


if __name__ == "__main__":
    print(section(json.load(sys.stdin)))
