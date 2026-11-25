# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
"""Packages the built app as an MSIX for the Microsoft Store.

    python .github/scripts/package_msix.py --exe app/src-tauri/target/release/serialbot.exe \\
        --version 0.3.0-beta.2 --out dist

Writes `SerialBot_<version>_x64.msix` into `--out`: the program, its icons
and an AppxManifest.xml with the identity the Store assigned (Partner Center
→ SerialBot → Product identity). The package is not signed: the Store signs
it, so it is only for uploading to Partner Center, not for installing.

`--layout-only` writes the folder the package is made from and stops, so
everything but `makeappx` (Windows SDK) can be tried on any system.

The package declares the alias `serialbot.exe`, so `serialbot mcp` (the
stdio bridge for Claude Desktop) can be started from anywhere: files under
WindowsApps cannot be started directly.
"""

import argparse
import glob
import os
import re
import shutil
import subprocess
from pathlib import Path
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parents[2]
ICONS = ROOT / "app" / "src-tauri" / "icons"

# From Partner Center → SerialBot → Product identity.
IDENTITY_NAME = "Jerry-Token.SerialBot"
PUBLISHER = "CN=6D958A18-B1D4-46B1-A60A-9C25DD1E5419"
PUBLISHER_DISPLAY_NAME = "Jerry-Token"
# Must be the reserved product name exactly.
DISPLAY_NAME = "SerialBot"
DESCRIPTION = "Serial port assistant whose AI agents work through MCP"
EXECUTABLE = "serialbot.exe"
ALIAS = "serialbot.exe"

# Windows 10 1809, the oldest the Store still offers apps to with WebView2.
MIN_VERSION = "10.0.17763.0"
MAX_VERSION_TESTED = "10.0.26100.0"

# Icons the manifest names (tauri icon makes them).
ASSETS = ["StoreLogo.png", "Square44x44Logo.png", "Square150x150Logo.png"]

STAGES = {"alpha": 0, "beta": 20, "rc": 80}


def store_version(version: str) -> str:
    """The Store's four-part version for a release version.

    The Store wants Major.Minor.Build.0 and a higher number for every
    upload, while our pre-releases share 0.3.0. The build part is
    patch × 100 + the stage: alpha.N → N, beta.N → 20 + N, rc.N → 80 + N,
    the release itself → 99, so they sort as they were made.

    >>> store_version("0.3.0-beta.2")
    '0.3.22.0'
    >>> store_version("0.3.0")
    '0.3.99.0'
    >>> store_version("v1.2.3-rc.1")
    '1.2.381.0'
    >>> store_version("0.3.0-alpha.4")
    '0.3.4.0'
    """
    m = re.fullmatch(r"v?(\d+)\.(\d+)\.(\d+)(?:-(alpha|beta|rc)\.(\d+))?", version)
    if not m:
        raise SystemExit(f"--version {version!r}: expected 1.2.3 or 1.2.3-beta.4")
    major, minor, patch, stage, n = m.groups()
    if int(patch) > 654 or (n and int(n) > 19):
        raise SystemExit(f"--version {version!r}: too large for the Store's numbering")
    step = 99 if stage is None else STAGES[stage] + int(n)
    return f"{major}.{minor}.{int(patch) * 100 + step}.0"


def manifest(version: str) -> str:
    """AppxManifest.xml for `version` (already in the Store's form)."""
    return f"""<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:uap3="http://schemas.microsoft.com/appx/manifest/uap/windows10/3"
  xmlns:desktop="http://schemas.microsoft.com/appx/manifest/desktop/windows10"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  IgnorableNamespaces="uap uap3 desktop rescap">
  <Identity Name="{IDENTITY_NAME}" Publisher="{PUBLISHER}" Version="{version}" ProcessorArchitecture="x64" />
  <Properties>
    <DisplayName>{escape(DISPLAY_NAME)}</DisplayName>
    <PublisherDisplayName>{escape(PUBLISHER_DISPLAY_NAME)}</PublisherDisplayName>
    <Logo>Assets\\StoreLogo.png</Logo>
    <Description>{escape(DESCRIPTION)}</Description>
  </Properties>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="{MIN_VERSION}" MaxVersionTested="{MAX_VERSION_TESTED}" />
  </Dependencies>
  <Resources>
    <Resource Language="en-us" />
    <Resource Language="zh-cn" />
  </Resources>
  <Applications>
    <Application Id="SerialBot" Executable="{EXECUTABLE}" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="{escape(DISPLAY_NAME)}" Description="{escape(DESCRIPTION)}"
        BackgroundColor="transparent"
        Square150x150Logo="Assets\\Square150x150Logo.png" Square44x44Logo="Assets\\Square44x44Logo.png" />
      <Extensions>
        <uap3:Extension Category="windows.appExecutionAlias" Executable="{EXECUTABLE}" EntryPoint="Windows.FullTrustApplication">
          <uap3:AppExecutionAlias>
            <desktop:ExecutionAlias Alias="{ALIAS}" />
          </uap3:AppExecutionAlias>
        </uap3:Extension>
      </Extensions>
    </Application>
  </Applications>
  <Capabilities>
    <rescap:Capability Name="runFullTrust" />
  </Capabilities>
</Package>
"""


def layout(exe: Path, version: str, into: Path) -> Path:
    """The package's folder: program, icons, LICENSE, NOTICE, manifest."""
    if into.exists():
        shutil.rmtree(into)
    (into / "Assets").mkdir(parents=True)
    shutil.copy2(exe, into / EXECUTABLE)
    for name in ASSETS:
        shutil.copy2(ICONS / name, into / "Assets" / name)
    shutil.copy2(ROOT / "LICENSE", into / "LICENSE.txt")
    shutil.copy2(ROOT / "NOTICE", into / "NOTICE.txt")
    (into / "AppxManifest.xml").write_text(manifest(store_version(version)), encoding="utf-8")
    return into


def makeappx() -> str:
    """makeappx.exe of the newest Windows SDK installed."""
    found = shutil.which("makeappx")
    if found:
        return found
    kits = os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")
    candidates = sorted(glob.glob(rf"{kits}\Windows Kits\10\bin\10.*\x64\makeappx.exe"))
    if not candidates:
        raise SystemExit("makeappx.exe not found: install the Windows 10/11 SDK")
    return candidates[-1]


def package(exe: Path, version: str, out: Path, layout_only: bool = False) -> Path:
    """The MSIX for `exe`, or with `layout_only` its folder."""
    out.mkdir(parents=True, exist_ok=True)
    folder = layout(exe, version, out / "msix-layout")
    if layout_only:
        return folder
    path = out / f"SerialBot_{version.removeprefix('v')}_x64.msix"
    subprocess.run(
        [makeappx(), "pack", "/o", "/h", "SHA256", "/d", str(folder), "/p", str(path)],
        check=True,
    )
    shutil.rmtree(folder)
    return path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--version", required=True, help="e.g. 0.3.0-beta.2")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--layout-only", action="store_true")
    args = parser.parse_args()
    print(package(args.exe, args.version, args.out, args.layout_only))


if __name__ == "__main__":
    main()
