#!/usr/bin/env python3
"""Cekirdek fontunu userland kutuphanesine kopyalar.

Cekirdek ve Ring 3 uygulamalari **ayni fonta** sahip olsun diye; kabuktaki
metinle uygulamalarin cizdigi metin ayni gorunur. Iki crate ayri hedeflere
linklendigi ve aralarinda paylasilan bir kutuphane olmadigi icin kaynak
dosya paylasilamiyor, kopyalaniyor.

Kullanim:  python3 tools/sync_font.py
"""
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "kernel/src/level0a/drivers/font_data.rs")
DST = os.path.join(ROOT, "userland-rs/src/font.rs")

HEADER = """//! OTOMATIK URETILDI -- elle duzenlemeyin.
//! Kaynak: `kernel/src/level0a/drivers/font_data.rs` (tools/sync_font.py)
//!
//! Cekirdek ve userland **ayni fonta** sahiptir: kabuktaki metinle
//! uygulamalarin cizdigi metin ayni gorunsun diye. Ayni dosyayi iki
//! yerden derlemek yerine kopyalanir, cunku iki crate ayri hedeflere
//! (cekirdek / Ring 3 uygulamasi) linklenir ve aralarinda paylasilan bir
//! kutuphane yoktur.
"""


def main():
    body = open(SRC).read().split("pub const FONT_WIDTH", 1)[1]
    open(DST, "w").write(HEADER + "\npub const FONT_WIDTH" + body)
    print("%s -> %s" % (SRC, DST))


if __name__ == "__main__":
    main()
