#!/usr/bin/env python3
"""8x16 bitmap font ureteci -- TCMK framebuffer konsolu ve GUI icin.

DejaVu Sans Mono'yu 8x16 hucrelere rasterlestirip Rust'a gomulebilecek
duz bir bayt dizisine cevirir. ASCII 32..126 (95 karakter) uretilir;
her karakter 16 bayt, her bayt bir satirin 8 pikselidir (MSB = soldaki
piksel).

Cikti: kernel/src/level0a/drivers/font_data.rs

Kullanim:
    python3 tools/gen_font.py
"""

import pathlib
import sys

from PIL import Image, ImageDraw, ImageFont

CELL_W = 8
CELL_H = 16
FIRST = 32
LAST = 126

FONT_CANDIDATES = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
]


def pick_font():
    for path in FONT_CANDIDATES:
        if pathlib.Path(path).exists():
            return path
    sys.exit("uygun monospace font bulunamadi")


def render_glyph(font, ch: str) -> list:
    """Tek karakteri CELL_W x CELL_H hucreye rasterlestirir."""
    img = Image.new("L", (CELL_W, CELL_H), 0)
    draw = ImageDraw.Draw(img)
    # Dikey yerlesim: taban cizgisini hucreye oturt.
    draw.text((0, -1), ch, font=font, fill=255)

    rows = []
    for y in range(CELL_H):
        bits = 0
        for x in range(CELL_W):
            # 128 esigi: gri tonlari ikili hale getir.
            if img.getpixel((x, y)) >= 128:
                bits |= 1 << (CELL_W - 1 - x)
        rows.append(bits)
    return rows


def main() -> None:
    font_path = pick_font()
    # 13 punto DejaVu Sans Mono ~8 piksel genisliginde hucreye oturur.
    font = ImageFont.truetype(font_path, 13)

    glyphs = []
    for code in range(FIRST, LAST + 1):
        glyphs.append(render_glyph(font, chr(code)))

    out = pathlib.Path("kernel/src/level0a/drivers/font_data.rs")
    lines = [
        "//! OTOMATIK URETILDI -- elle duzenlemeyin.",
        "//! Kaynak: tools/gen_font.py (%s)" % pathlib.Path(font_path).name,
        "//!",
        "//! 8x16 bitmap font, ASCII %d..%d." % (FIRST, LAST),
        "//! Her karakter %d bayt; her bayt bir satir, MSB soldaki piksel." % CELL_H,
        "",
        "pub const FONT_WIDTH: usize = %d;" % CELL_W,
        "pub const FONT_HEIGHT: usize = %d;" % CELL_H,
        "pub const FONT_FIRST: u8 = %d;" % FIRST,
        "pub const FONT_LAST: u8 = %d;" % LAST,
        "",
        "#[rustfmt::skip]",
        "pub static FONT: [[u8; FONT_HEIGHT]; %d] = [" % len(glyphs),
    ]

    for code, rows in zip(range(FIRST, LAST + 1), glyphs):
        ch = chr(code)
        label = "space" if ch == " " else ch
        body = ", ".join("0x%02X" % r for r in rows)
        lines.append("    [%s], // '%s'" % (body, label))

    lines.append("];")
    lines.append("")

    out.write_text("\n".join(lines))
    print(f"{out}: {len(glyphs)} karakter, {CELL_W}x{CELL_H}")


if __name__ == "__main__":
    main()
