"""Builds the SerialBot site into a folder: `python3 site/build.py _site`.

One template (`template.html`), one string table per language
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
            f'<g class="frame"><rect x="{start}" y="8" width="{width}" height="60" rx="3"/>'
            f'<text class="char" x="{start + width / 2}" y="92">{html.escape(char)}</text>'
            f'<text class="hex" x="{start + width / 2}" y="108">0x{byte:02X}</text></g>'
        )
        points += [(x, HIGH), (x + GAP, HIGH)]
        x += GAP
    path = "M" + " L".join(f"{px},{py}" for px, py in points)
    return (
        f'<svg class="uart" viewBox="0 0 {x} 116" role="img" aria-labelledby="uart-title">'
        f'<title id="uart-title">{{{{uart_title}}}}</title>'
        + "".join(labels)
        + f'<path class="wire" d="{path}" pathLength="1"/></svg>'
    )


def render(template: str, strings: dict, lang: str) -> str:
    """Fills `{{key}}` placeholders; list values become <li> items."""
    values = dict(strings)
    values["lang"] = lang
    values["root"] = "./" if lang == "en" else "../"
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
    for lang in LANGS:
        strings = json.loads((SITE / "strings" / f"{lang}.json").read_text(encoding="utf-8"))
        target = out if lang == "en" else out / lang
        target.mkdir(parents=True, exist_ok=True)
        (target / "index.html").write_text(render(template, strings, lang), encoding="utf-8")
    for name in ("style.css", "download.js", "CNAME"):
        shutil.copy(SITE / name, out / name)
    for image in (REPO / "docs" / "images").iterdir():
        shutil.copy(image, out / "images" / image.name)
    shutil.copy(REPO / "app" / "src-tauri" / "icons" / "128x128.png", out / "images" / "icon.png")


if __name__ == "__main__":
    build(Path(sys.argv[1] if len(sys.argv) > 1 else "_site"))
