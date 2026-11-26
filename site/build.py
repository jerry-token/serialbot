# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
"""Builds the SerialBot site into a folder: `python3 site/build.py _site`.

One template (`template.html`, and `privacy.html` for /privacy/), one string table per language
(`strings/<lang>.json`). English goes to `/`, every other language to
`/<lang>/`. Screenshots and the app icon are copied from the repository, so
the site always shows the same images as the README.

The hero is a UART timing diagram of a real command, generated here as SVG.
"""

import html
import json
import re
import shutil
import sys
from pathlib import Path

SITE = Path(__file__).resolve().parent
REPO = SITE.parent
LANGS = ("en", "zh")
SITE_URL = "https://serialbot.aiany.cc"
# Pages besides each language's home, as paths below it.
PAGES = ("", "privacy/")
# The command drawn in the hero, as it goes over the wire.
HERO_BYTES = b"AT+GMR\r\n"

BIT_W = 9  # px per bit
FRAME_BITS = 10  # start + 8 data + stop
GAP = 6  # idle px between frames
HIGH, LOW = 18, 58  # y of logic 1 / logic 0


def frame_bits(byte: int) -> list[int]:
    """Start bit, 8 data bits LSB first, stop bit (8N1)."""
    return [0, *((byte >> i) & 1 for i in range(8)), 1]


def uart_svg(data: bytes) -> str:
    """The 8N1 waveform of `data` with each frame decoded below it."""
    x = GAP
    points = [(0, HIGH), (x, HIGH)]
    labels = []
    for byte in data:
        start = x
        for bit in frame_bits(byte):
            y = HIGH if bit else LOW
            points += [(x, y), (x + BIT_W, y)]
            x += BIT_W
        width = FRAME_BITS * BIT_W
        char = {0x0D: "\\r", 0x0A: "\\n"}.get(byte, chr(byte))
        labels.append(
            f'<g class="frame" aria-hidden="true"><rect x="{start}" y="8" width="{width}" height="60" rx="3"/>'
            f'<text class="char" x="{start + width / 2}" y="92">{html.escape(char)}</text>'
            f'<text class="hex" x="{start + width / 2}" y="108">0x{byte:02X}</text></g>'
        )
        points += [(x, HIGH), (x + GAP, HIGH)]
        x += GAP
    path = "M" + " L".join(f"{px},{py}" for px, py in points)
    return (
        # role="img" with a title: screen readers announce the title, not the
        # byte labels drawn inside.
        f'<svg class="uart" viewBox="0 0 {x} 116" role="img" aria-labelledby="uart-title">'
        f'<title id="uart-title">{{{{uart_title}}}}</title>'
        + "".join(labels)
        + f'<path class="wire" d="{path}" pathLength="1"/></svg>'
    )


def page_url(lang: str, path: str = "") -> str:
    """The absolute URL of `path` (a key of PAGES) in `lang`."""
    return f"{SITE_URL}/{'' if lang == 'en' else lang + '/'}{path}"


def json_ld(strings: dict, lang: str) -> str:
    """SoftwareApplication structured data for a home page."""
    data = {
        "@context": "https://schema.org",
        "@type": "SoftwareApplication",
        "name": "SerialBot",
        "url": page_url(lang),
        "description": strings["description"],
        "inLanguage": strings["html_lang"],
        "applicationCategory": "DeveloperApplication",
        "applicationSubCategory": "Serial port assistant",
        "operatingSystem": "Windows, macOS, Linux",
        "image": f"{SITE_URL}/images/windows.png",
        "screenshot": f"{SITE_URL}/images/terminal-dark.png",
        "downloadUrl": "https://github.com/jerry-token/serialbot/releases",
        "license": "https://www.gnu.org/licenses/gpl-3.0.html",
        "isAccessibleForFree": True,
        "offers": {"@type": "Offer", "price": "0", "priceCurrency": "USD"},
        "author": {"@type": "Person", "name": "Jerry", "email": "support@aiany.cc"},
        "sameAs": ["https://github.com/jerry-token/serialbot"],
    }
    text = json.dumps(data, ensure_ascii=False).replace("</", "<\\/")
    return f'<script type="application/ld+json">{text}</script>'


def sitemap() -> str:
    """Every page in every language, each listing its translations."""
    urls = []
    for path in PAGES:
        links = "".join(
            f'<xhtml:link rel="alternate" hreflang="{code}" href="{page_url(lang, path)}"/>'
            for code, lang in (("en", "en"), ("zh-Hans", "zh"), ("x-default", "en"))
        )
        urls += [f"<url><loc>{page_url(lang, path)}</loc>{links}</url>" for lang in LANGS]
    return (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" '
        'xmlns:xhtml="http://www.w3.org/1999/xhtml">\n' + "\n".join(urls) + "\n</urlset>\n"
    )


ROBOTS = f"User-agent: *\nAllow: /\n\nSitemap: {SITE_URL}/sitemap.xml\n"


def render(template: str, strings: dict, lang: str, depth: int = 0, path: str = "") -> str:
    """Fills `{{key}}` placeholders; list values become <li> items. `depth`
    is how many folders below the language's home the page is, `path` the
    page's path below it (for its canonical URL and its translations)."""
    values = dict(strings)
    values["lang"] = lang
    values["site_url"] = SITE_URL
    values["canonical"] = page_url(lang, path)
    values["url_en"] = page_url("en", path)
    values["url_zh"] = page_url("zh", path)
    values["jsonld"] = json_ld(strings, lang)
    values["root"] = ("./" if lang == "en" else "../") if depth == 0 else "../" * (depth + (lang != "en"))
    values["uart"] = uart_svg(HERO_BYTES)
    out = template.replace("{{uart}}", values["uart"])

    def fill(match: re.Match) -> str:
        key = match.group(1)
        if key not in values:
            raise KeyError(f"missing string {key!r} for {lang}")
        value = values[key]
        if isinstance(value, list):
            return "".join(f"<li>{item}</li>" for item in value)
        if isinstance(value, dict):
            return "".join(f"<dt>{k}</dt><dd>{v}</dd>" for k, v in value.items())
        return str(value)

    # Twice: the hero SVG brings its own placeholder.
    for _ in range(2):
        out = re.sub(r"\{\{(\w+)\}\}", fill, out)
    return out


def build(out: Path) -> None:
    if out.exists():
        shutil.rmtree(out)
    (out / "images").mkdir(parents=True)
    template = (SITE / "template.html").read_text(encoding="utf-8")
    privacy = (SITE / "privacy.html").read_text(encoding="utf-8")
    for lang in LANGS:
        strings = json.loads((SITE / "strings" / f"{lang}.json").read_text(encoding="utf-8"))
        target = out if lang == "en" else out / lang
        target.mkdir(parents=True, exist_ok=True)
        (target / "index.html").write_text(render(template, strings, lang), encoding="utf-8")
        # The privacy policy the Microsoft Store listing links to.
        (target / "privacy").mkdir()
        page = render(privacy, strings, lang, depth=1, path="privacy/")
        (target / "privacy" / "index.html").write_text(page, encoding="utf-8")
    (out / "sitemap.xml").write_text(sitemap(), encoding="utf-8")
    (out / "robots.txt").write_text(ROBOTS, encoding="utf-8")
    for name in ("style.css", "download.js", "CNAME"):
        shutil.copy(SITE / name, out / name)
    shutil.copytree(SITE / "fonts", out / "fonts")
    for image in (REPO / "docs" / "images").iterdir():
        shutil.copy(image, out / "images" / image.name)
    shutil.copy(REPO / "app" / "src-tauri" / "icons" / "128x128.png", out / "images" / "icon.png")


if __name__ == "__main__":
    build(Path(sys.argv[1] if len(sys.argv) > 1 else "_site"))
