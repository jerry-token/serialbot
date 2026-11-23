# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
"""Tests of the site build: `python3 -m unittest discover site`."""

import json
import re
import tempfile
import unittest
from pathlib import Path

import build

SITE = Path(__file__).resolve().parent


class BuildTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.out = Path(cls.tmp.name) / "site"
        build.build(cls.out)
        cls.pages = {"en": cls.out / "index.html", "zh": cls.out / "zh" / "index.html"}

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_both_languages_have_the_same_strings(self):
        en = json.loads((SITE / "strings" / "en.json").read_text(encoding="utf-8"))
        zh = json.loads((SITE / "strings" / "zh.json").read_text(encoding="utf-8"))
        self.assertEqual(set(en), set(zh))
        for key, value in en.items():
            self.assertEqual(type(value), type(zh[key]), key)
            if isinstance(value, (list, dict)):
                self.assertEqual(len(value), len(zh[key]), key)

    def test_pages_are_complete(self):
        for lang, page in self.pages.items():
            text = page.read_text(encoding="utf-8")
            self.assertNotIn("{{", text, lang)
            self.assertIn("support@aiany.cc", text)
            self.assertIn("© 2026 Jerry", text)
            self.assertIn('<path class="wire"', text)

    def test_local_links_resolve(self):
        for lang, page in self.pages.items():
            text = page.read_text(encoding="utf-8")
            for ref in re.findall(r'(?:href|src)="([^"#:]+)"', text):
                target = (page.parent / ref).resolve()
                if ref.endswith("/"):
                    target = target / "index.html"
                self.assertTrue(target.exists(), f"{lang}: {ref}")

    def test_fonts_are_served_from_the_site(self):
        # No third-party font host: it is slow or blocked in some regions.
        for page in self.pages.values():
            self.assertNotIn("fonts.googleapis", page.read_text(encoding="utf-8"))
        css = (self.out / "style.css").read_text(encoding="utf-8")
        for url in re.findall(r'url\("([^"]+)"\)', css):
            self.assertTrue((self.out / url).exists(), url)
        self.assertNotIn("font-stretch", css)

    def test_the_custom_domain_is_kept(self):
        self.assertEqual((self.out / "CNAME").read_text().strip(), "serialbot.aiany.cc")

    def test_frames_are_8n1_lsb_first(self):
        # 'A' = 0x41 = 0b01000001: start 0, data 1,0,0,0,0,0,1,0, stop 1.
        self.assertEqual(build.frame_bits(0x41), [0, 1, 0, 0, 0, 0, 0, 1, 0, 1])
        svg = build.uart_svg(b"AT")
        self.assertEqual(svg.count('class="frame"'), 2)
        self.assertIn(">0x54<", svg)

    def test_missing_strings_are_reported(self):
        with self.assertRaises(KeyError):
            build.render("{{nope}}", {}, "en")


if __name__ == "__main__":
    unittest.main()
