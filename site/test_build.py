# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
"""Tests of the site build: `python3 -m unittest discover site`."""

import json
import re
import xml.etree.ElementTree as ET
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
        cls.privacy = {"en": cls.out / "privacy" / "index.html", "zh": cls.out / "zh" / "privacy" / "index.html"}

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
            self.assertIn('data-asset="-linux-amd64.deb"', text)
            self.assertIn("serialbot-cli tui", text)
            self.assertIn('<path class="wire"', text)

    def test_privacy_policy_pages(self):
        for lang, page in self.privacy.items():
            text = page.read_text(encoding="utf-8")
            self.assertNotIn("{{", text, lang)
            self.assertIn("strongwong003@gmail.com", text)
            self.assertIn("127.0.0.1", text)
            self.assertIn('href="../style.css"' if lang == "en" else 'href="../../style.css"', text)
        for page in self.pages.values():
            self.assertIn('privacy/"', page.read_text(encoding="utf-8"))

    def test_local_links_resolve(self):
        for lang, page in [*self.pages.items(), *self.privacy.items()]:
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

    def test_each_page_names_itself_canonical_and_its_translations(self):
        expected = {
            self.pages["en"]: "https://serialbot.aiany.cc/",
            self.pages["zh"]: "https://serialbot.aiany.cc/zh/",
            self.privacy["en"]: "https://serialbot.aiany.cc/privacy/",
            self.privacy["zh"]: "https://serialbot.aiany.cc/zh/privacy/",
        }
        for page, url in expected.items():
            text = page.read_text(encoding="utf-8")
            self.assertEqual(re.findall(r'rel="canonical" href="([^"]+)"', text), [url])
            self.assertIn(f'<meta property="og:url" content="{url}">', text)
            langs = dict(
                (code, href) for code, href in re.findall(r'hreflang="([^"]+)" href="([^"]+)"', text)
            )
            self.assertEqual(set(langs), {"en", "zh-Hans", "x-default"})
            self.assertEqual(langs["x-default"], langs["en"])
            self.assertIn(url, langs.values())

    def test_titles_and_descriptions_carry_the_search_terms(self):
        for lang, page in self.pages.items():
            text = page.read_text(encoding="utf-8")
            title = re.search(r"<title>(.*?)</title>", text).group(1)
            description = re.search(r'<meta name="description" content="([^"]+)"', text).group(1)
            self.assertIn("MCP", title)
            self.assertIn("UART", title)
            for word in ("MCP", "ESP32", "STM32"):
                self.assertIn(word, description, lang)
            self.assertLessEqual(len(description), 160 if lang == "en" else 100, lang)
            self.assertIn('name="twitter:card" content="summary_large_image"', text)
            image = re.search(r'property="og:image" content="([^"]+)"', text).group(1)
            self.assertTrue((self.out / image.removeprefix("https://serialbot.aiany.cc/")).exists())

    def test_home_pages_describe_the_app_as_structured_data(self):
        for lang, page in self.pages.items():
            text = page.read_text(encoding="utf-8")
            blocks = re.findall(r'<script type="application/ld\+json">(.*?)</script>', text, re.S)
            self.assertEqual(len(blocks), 1, lang)
            data = json.loads(blocks[0])
            self.assertEqual(data["@type"], "SoftwareApplication")
            self.assertEqual(data["name"], "SerialBot")
            self.assertEqual(data["offers"]["price"], "0")
            self.assertIn("Linux", data["operatingSystem"])

    def test_json_ld_cannot_close_its_script_tag(self):
        text = build.json_ld({"description": "</script><b>", "html_lang": "en"}, "en")
        self.assertEqual(text.count("</script>"), 1)
        self.assertEqual(json.loads(text.split(">", 1)[1].rsplit("<", 1)[0])["description"], "</script><b>")

    def test_sitemap_and_robots(self):
        ns = {"s": "http://www.sitemaps.org/schemas/sitemap/0.9", "x": "http://www.w3.org/1999/xhtml"}
        root = ET.parse(self.out / "sitemap.xml").getroot()
        locs = [url.find("s:loc", ns).text for url in root.findall("s:url", ns)]
        self.assertEqual(
            sorted(locs),
            sorted(
                f"https://serialbot.aiany.cc/{p}"
                for p in ("", "zh/", "privacy/", "zh/privacy/")
            ),
        )
        for url in root.findall("s:url", ns):
            self.assertEqual(len(url.findall("x:link", ns)), 3)
        for loc in locs:
            path = loc.removeprefix("https://serialbot.aiany.cc/")
            self.assertTrue((self.out / path / "index.html").exists(), loc)
        robots = (self.out / "robots.txt").read_text()
        self.assertIn("Sitemap: https://serialbot.aiany.cc/sitemap.xml", robots)
        self.assertNotIn("Disallow: /\n", robots)

    def test_screenshots_come_as_webp_with_a_png_fallback_and_alt_text(self):
        for lang, page in self.pages.items():
            text = page.read_text(encoding="utf-8")
            pictures = re.findall(r"<picture>(.*?)</picture>", text)
            self.assertEqual(len(pictures), 4, lang)
            for picture in pictures:
                webp = re.search(r'srcset="([^"]+\.webp)"', picture).group(1)
                png = re.search(r'<img src="([^"]+\.png)"', picture).group(1)
                self.assertEqual(webp.removesuffix(".webp"), png.removesuffix(".png"))
                self.assertTrue((page.parent / webp).resolve().exists(), webp)
                self.assertRegex(picture, r'alt="[^"]{10,}"')

    def test_hero_byte_labels_are_hidden_from_screen_readers(self):
        svg = build.uart_svg(b"AT")
        self.assertIn('role="img"', svg)
        self.assertEqual(svg.count('<g class="frame" aria-hidden="true">'), 2)

    def test_missing_strings_are_reported(self):
        with self.assertRaises(KeyError):
            build.render("{{nope}}", {}, "en")


if __name__ == "__main__":
    unittest.main()
