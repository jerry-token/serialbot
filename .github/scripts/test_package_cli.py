# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
"""Tests of the CLI packaging: `python3 -m unittest discover -s .github/scripts`."""

import doctest
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import package_cli  # noqa: E402
import release_sizes  # noqa: E402

FAKE = """#!/bin/sh
# Prints a completion script, as `serialbot-cli completions <shell>` does.
echo "complete for $2"
"""


class Package(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.binary = self.tmp / "serialbot-cli"
        self.binary.write_text(FAKE)
        self.binary.chmod(0o755)
        self.out = self.tmp / "dist"

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def test_linux_gets_a_tarball_and_a_deb(self):
        if not shutil.which("dpkg-deb"):
            self.skipTest("dpkg-deb not installed")
        made = package_cli.package(self.binary, "0.3.0-beta.1", "linux-x64", self.out)
        names = [p.name for p in made]
        self.assertEqual(
            names,
            ["serialbot-cli-0.3.0-beta.1-linux-x64.tar.gz", "serialbot-cli-0.3.0-beta.1-linux-amd64.deb"],
        )
        with tarfile.open(made[0]) as t:
            members = {m.name: m for m in t.getmembers()}
            program = members["serialbot-cli-0.3.0-beta.1/serialbot-cli"]
            self.assertTrue(program.mode & 0o111, "executable")
            for name in ["LICENSE", "NOTICE", "README.txt", "completions/_serialbot-cli"]:
                self.assertIn(f"serialbot-cli-0.3.0-beta.1/{name}", members)
            bash = t.extractfile("serialbot-cli-0.3.0-beta.1/completions/serialbot-cli.bash")
            self.assertEqual(bash.read(), b"complete for bash\n")
        info = subprocess.run(["dpkg-deb", "-f", str(made[1])], capture_output=True, text=True).stdout
        self.assertIn("Version: 0.3.0~beta.1", info)
        self.assertIn("Architecture: amd64", info)
        self.assertIn("Maintainer: Jerry <support@aiany.cc>", info)
        files = subprocess.run(["dpkg-deb", "-c", str(made[1])], capture_output=True, text=True).stdout
        for path in [
            "./usr/bin/serialbot-cli",
            "./usr/share/bash-completion/completions/serialbot-cli",
            "./usr/share/zsh/vendor-completions/_serialbot-cli",
            "./usr/share/fish/vendor_completions.d/serialbot-cli.fish",
            "./usr/share/doc/serialbot-cli/copyright",
        ]:
            self.assertIn(path, files)
        self.assertIn("root/root", files)

    def test_windows_gets_a_zip_and_macos_a_tarball(self):
        made = package_cli.package(self.binary, "0.3.0", "windows-x64", self.out)
        self.assertEqual([p.name for p in made], ["serialbot-cli-0.3.0-windows-x64.zip"])
        with zipfile.ZipFile(made[0]) as z:
            self.assertIn("serialbot-cli-0.3.0/serialbot-cli.exe", z.namelist())
            self.assertIn("serialbot-cli-0.3.0/completions/serialbot-cli.ps1", z.namelist())
            readme = z.read("serialbot-cli-0.3.0/README.txt").decode()
            self.assertIn("Copyright (C) 2026 Jerry", readme)
        made = package_cli.package(self.binary, "0.3.0", "macos-universal", self.out)
        self.assertEqual([p.name for p in made], ["serialbot-cli-0.3.0-macos-universal.tar.gz"])

    def test_unknown_platforms_are_refused(self):
        with self.assertRaises(SystemExit):
            package_cli.package(self.binary, "0.3.0", "amiga", self.out)

    def test_cli_files_do_not_look_like_the_apps(self):
        # The site and the notes pick files by these endings.
        app_endings = ["_amd64.deb", "_arm64.deb", "_amd64.AppImage", ".app.tar.gz", "_x64-setup.exe"]
        for platform in package_cli.PLATFORMS:
            name = f"serialbot-cli-0.3.0-{platform}.tar.gz"
            self.assertFalse(any(name.endswith(e) for e in app_endings), name)
        self.assertFalse("serialbot-cli-0.3.0-linux-amd64.deb".endswith("_amd64.deb"))


class Docs(unittest.TestCase):
    def test_examples(self):
        for module in (package_cli, release_sizes):
            failed, _ = doctest.testmod(module)
            self.assertEqual(failed, 0, module.__name__)


if __name__ == "__main__":
    unittest.main()
