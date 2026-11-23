# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
"""Packages a built `serialbot-cli` for the release page.

    python3 .github/scripts/package_cli.py --binary target/release/serialbot-cli \\
        --version 0.3.0-beta.1 --platform linux-x64 --out dist

Writes into `--out`:

- `serialbot-cli-<version>-<platform>.tar.gz` (`.zip` on Windows): a folder
  with the program, its shell completions (made by the program itself),
  LICENSE, NOTICE and a short README;
- on Linux also `serialbot-cli-<version>-linux-<amd64|arm64>.deb`, which
  installs the program to /usr/bin and the completions where bash, zsh and
  fish look for them.

The names end in `-<platform>.<ext>` so they never match the app's files
(`SerialBot_<version>_amd64.deb` and so on) on the site or in the notes.
"""

import argparse
import io
import os
import shutil
import subprocess
import tarfile
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

PLATFORMS = ("linux-x64", "linux-arm64", "macos-universal", "windows-x64")

# Debian architecture of each Linux platform.
DEB_ARCH = {"linux-x64": "amd64", "linux-arm64": "arm64"}

# Completion script file names, by shell.
COMPLETIONS = {
    "bash": "serialbot-cli.bash",
    "zsh": "_serialbot-cli",
    "fish": "serialbot-cli.fish",
    "powershell": "serialbot-cli.ps1",
}

README = """SerialBot CLI {version}

  serialbot-cli ports                 list the serial ports
  serialbot-cli open /dev/ttyUSB0     interactive terminal (minicom keys, Ctrl+A Z)
  serialbot-cli tui PORT PORT         full screen: several ports, waveform, AI panel
  serialbot-cli --help                everything else

Shell completions are in completions/ (bash, zsh, fish, PowerShell).

Copyright (C) 2026 Jerry <support@aiany.cc>, <strongwong003@gmail.com>
GPL-3.0-only: see LICENSE and NOTICE.
Official releases: https://github.com/jerry-token/serialbot/releases
"""


def deb_version(version: str) -> str:
    """`0.3.0-beta.1` → `0.3.0~beta.1`, so a pre-release sorts first.

    >>> deb_version("0.3.0-beta.1")
    '0.3.0~beta.1'
    >>> deb_version("0.3.0")
    '0.3.0'
    """
    return version.replace("-", "~", 1)


def completions(binary: Path) -> dict:
    """Each shell's completion script, from the program itself."""
    scripts = {}
    for shell in COMPLETIONS:
        out = subprocess.run(
            [str(binary), "completions", shell], check=True, capture_output=True
        )
        scripts[shell] = out.stdout
    return scripts


def folder(binary: Path, version: str, scripts: dict, into: Path, exe: str) -> Path:
    """The release folder: program, completions, LICENSE, NOTICE, README."""
    target = into / f"serialbot-cli-{version}"
    (target / "completions").mkdir(parents=True)
    shutil.copy2(binary, target / exe)
    os.chmod(target / exe, 0o755)
    for shell, name in COMPLETIONS.items():
        (target / "completions" / name).write_bytes(scripts[shell])
    shutil.copy2(ROOT / "LICENSE", target / "LICENSE")
    shutil.copy2(ROOT / "NOTICE", target / "NOTICE")
    (target / "README.txt").write_text(README.format(version=version), encoding="utf-8")
    return target


def archive(target: Path, out: Path, name: str, windows: bool) -> Path:
    """A `.zip` (Windows) or `.tar.gz` of `target`, keeping the program executable."""
    if windows:
        path = out / f"{name}.zip"
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
            for file in sorted(target.rglob("*")):
                z.write(file, file.relative_to(target.parent))
        return path
    path = out / f"{name}.tar.gz"
    with tarfile.open(path, "w:gz") as t:
        t.add(target, arcname=target.name)
    return path


def control(version: str, arch: str, size_kb: int) -> str:
    return (
        "Package: serialbot-cli\n"
        f"Version: {deb_version(version)}\n"
        f"Architecture: {arch}\n"
        "Maintainer: Jerry <support@aiany.cc>\n"
        f"Installed-Size: {size_kb}\n"
        "Depends: libc6, libudev1\n"
        "Section: utils\n"
        "Priority: optional\n"
        "Homepage: https://serialbot.aiany.cc\n"
        "Description: SerialBot in the terminal: serial ports for people and AI agents\n"
        " Send, monitor, an interactive terminal with minicom keys, a full-screen\n"
        " terminal with several ports and a waveform, scripts, and an MCP server.\n"
    )


def deb(binary: Path, version: str, arch: str, scripts: dict, out: Path) -> Path:
    """A `.deb` with the program, completions and the copyright file."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp) / "pkg"
        files = {
            "usr/bin/serialbot-cli": None,
            "usr/share/bash-completion/completions/serialbot-cli": scripts["bash"],
            "usr/share/zsh/vendor-completions/_serialbot-cli": scripts["zsh"],
            "usr/share/fish/vendor_completions.d/serialbot-cli.fish": scripts["fish"],
            "usr/share/doc/serialbot-cli/copyright": (
                (ROOT / "NOTICE").read_bytes()
                + b"\nThe full license is in /usr/share/common-licenses/GPL-3.\n"
            ),
        }
        for rel, data in files.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            if data is None:
                shutil.copy2(binary, path)
                os.chmod(path, 0o755)
            else:
                path.write_bytes(data)
                os.chmod(path, 0o644)
        size_kb = sum(f.stat().st_size for f in root.rglob("*") if f.is_file()) // 1024 + 1
        (root / "DEBIAN").mkdir()
        (root / "DEBIAN" / "control").write_text(control(version, arch, size_kb))
        path = out / f"serialbot-cli-{version}-linux-{arch}.deb"
        subprocess.run(
            ["dpkg-deb", "--root-owner-group", "--build", str(root), str(path)],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        return path


def package(binary: Path, version: str, platform: str, out: Path) -> list:
    """Every file for `platform`, in `out`."""
    if platform not in PLATFORMS:
        raise SystemExit(f"--platform must be one of {', '.join(PLATFORMS)}")
    out.mkdir(parents=True, exist_ok=True)
    windows = platform.startswith("windows")
    scripts = completions(binary)
    made = []
    with tempfile.TemporaryDirectory() as tmp:
        exe = "serialbot-cli.exe" if windows else "serialbot-cli"
        target = folder(binary, version, scripts, Path(tmp), exe)
        made.append(archive(target, out, f"serialbot-cli-{version}-{platform}", windows))
    if platform in DEB_ARCH:
        made.append(deb(binary, version, DEB_ARCH[platform], scripts, out))
    return made


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--version", required=True, help="without the v, e.g. 0.3.0-beta.1")
    parser.add_argument("--platform", required=True, choices=PLATFORMS)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    for path in package(args.binary, args.version.removeprefix("v"), args.platform, args.out):
        print(path)


if __name__ == "__main__":
    main()
