# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
"""Tests of the MSIX packaging (everything but makeappx, which needs Windows)."""

import doctest
import shutil
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import package_msix  # noqa: E402

NS = {
    "m": "http://schemas.microsoft.com/appx/manifest/foundation/windows10",
    "uap": "http://schemas.microsoft.com/appx/manifest/uap/windows10",
    "uap3": "http://schemas.microsoft.com/appx/manifest/uap/windows10/3",
    "desktop": "http://schemas.microsoft.com/appx/manifest/desktop/windows10",
    "rescap": "http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities",
}


class Msix(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.exe = self.tmp / "serialbot.exe"
        self.exe.write_bytes(b"MZ")

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def test_manifest_carries_the_store_identity(self):
        folder = package_msix.package(self.exe, "0.3.0-beta.2", self.tmp / "out", layout_only=True)
        root = ET.parse(folder / "AppxManifest.xml").getroot()
        identity = root.find("m:Identity", NS)
        self.assertEqual(identity.get("Name"), "Jerry-Token.SerialBot")
        self.assertEqual(identity.get("Publisher"), "CN=6D958A18-B1D4-46B1-A60A-9C25DD1E5419")
        self.assertEqual(identity.get("Version"), "0.3.22.0")
        self.assertEqual(root.find("m:Properties/m:DisplayName", NS).text, "SerialBot")
        self.assertEqual(root.find("m:Properties/m:PublisherDisplayName", NS).text, "Jerry-Token")
        app = root.find("m:Applications/m:Application", NS)
        self.assertEqual(app.get("Executable"), "serialbot.exe")
        self.assertEqual(app.get("EntryPoint"), "Windows.FullTrustApplication")
        alias = app.find(".//desktop:ExecutionAlias", NS)
        self.assertEqual(alias.get("Alias"), "serialbot.exe")
        self.assertIsNotNone(root.find("m:Capabilities/rescap:Capability[@Name='runFullTrust']", NS))

    def test_every_file_the_manifest_names_is_there(self):
        folder = package_msix.package(self.exe, "0.3.0", self.tmp / "out", layout_only=True)
        text = (folder / "AppxManifest.xml").read_text()
        for name in ["serialbot.exe", "LICENSE.txt", "NOTICE.txt"]:
            self.assertTrue((folder / name).is_file(), name)
        for asset in package_msix.ASSETS:
            self.assertIn(f"Assets\\{asset}", text)
            self.assertTrue((folder / "Assets" / asset).is_file(), asset)

    def test_store_versions_only_grow(self):
        made = ["0.3.0-alpha.1", "0.3.0-beta.1", "0.3.0-beta.2", "0.3.0-rc.1", "0.3.0", "0.3.1-beta.1", "0.4.0-beta.1"]
        numbers = [tuple(int(p) for p in package_msix.store_version(v).split(".")) for v in made]
        self.assertEqual(numbers, sorted(numbers))
        self.assertTrue(all(n[3] == 0 for n in numbers), "the Store wants revision 0")
        for bad in ["0.3", "0.3.0-beta", "0.3.0-beta.20", "0.3.700"]:
            with self.assertRaises(SystemExit, msg=bad):
                package_msix.store_version(bad)

    def test_examples(self):
        failed, _ = doctest.testmod(package_msix)
        self.assertEqual(failed, 0)


if __name__ == "__main__":
    unittest.main()
