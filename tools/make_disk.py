#!/usr/bin/env python3
"""TCMK disk imaji ureteci.

`grub-mkrescue` bir **hibrit** imaj uretir: hem CD (ISO9660) hem sabit disk
olarak acilabilen, gecerli bir MBR'ye sahip tek dosya. Bu imajin sonuna
TCMKFS icin bos bir bolge eklenir ve MBR'nin bolum tablosuna ikinci bir
girdi yazilir:

    bolum 1  ISO hibrit -> GRUB + cekirdek (salt okunur, grub-mkrescue kurdu)
    bolum 2  0x7F       -> TCMKFS (kalici, yazilabilir veri)

Boylece **root gerekmeden** kendi kendine acilan ve kalici bir veri bolumu
olan tek bir disk imaji elde edilir; QEMU'da `-hda` ile kullanilir.

Kullanim:
    python3 tools/make_disk.py build/tcmk-i386.iso build/tcmk-disk.img 64
"""

import os
import shutil
import struct
import sys

SECTOR = 512
#: TCMKFS bolum turu -- cekirdekteki partition::TYPE_TCMKFS ile ayni olmali.
TYPE_TCMKFS = 0x7F
#: Bolum tablosunun MBR icindeki ofseti.
PARTITION_TABLE_OFFSET = 446
ENTRY_SIZE = 16


def chs_dummy():
    """LBA kullanildigi icin CHS alanlari 'buyuk disk' isaretiyle doldurulur.

    0xFE 0xFF 0xFF, "bu bolum CHS ile adreslenemez, LBA alanlarina bak"
    anlamina gelen yaygin kaliptir.
    """
    return bytes([0xFE, 0xFF, 0xFF])


def build(iso_path, out_path, data_mib):
    if not os.path.exists(iso_path):
        sys.exit("ISO bulunamadi: %s (once `make iso`)" % iso_path)

    shutil.copyfile(iso_path, out_path)
    iso_size = os.path.getsize(out_path)

    # TCMKFS bolgesi sektor sinirinda baslamali; ayrica 1 MiB'a hizalanir
    # (gelenek; 4 KiB fiziksel sektorlu disklerde de hizalama bozulmaz).
    align = (1024 * 1024) // SECTOR
    data_start = (iso_size + SECTOR - 1) // SECTOR
    data_start = ((data_start + align - 1) // align) * align
    data_sectors = data_mib * 1024 * 1024 // SECTOR

    with open(out_path, "r+b") as f:
        # Bolgeyi ayir (sparse: sifirlarla doldurmak yerine dosyayi uzat).
        f.truncate((data_start + data_sectors) * SECTOR)

        f.seek(0)
        mbr = bytearray(f.read(SECTOR))
        if mbr[510] != 0x55 or mbr[511] != 0xAA:
            sys.exit("ISO hibrit MBR icermiyor; grub-mkrescue surumunu kontrol edin.")

        # Ilk bos bolum girdisini bul (grub-mkrescue 1. girdiyi kullanir).
        slot = None
        for i in range(4):
            base = PARTITION_TABLE_OFFSET + i * ENTRY_SIZE
            if mbr[base + 4] == 0:
                slot = i
                break
        if slot is None:
            sys.exit("MBR'de bos bolum girdisi yok.")

        entry = struct.pack(
            "<B3sB3sII",
            0x00,               # acilis bayragi yok (GRUB 1. bolumden acar)
            chs_dummy(),
            TYPE_TCMKFS,
            chs_dummy(),
            data_start,
            data_sectors,
        )
        base = PARTITION_TABLE_OFFSET + slot * ENTRY_SIZE
        mbr[base:base + ENTRY_SIZE] = entry

        f.seek(0)
        f.write(mbr)

    total_mib = os.path.getsize(out_path) // (1024 * 1024)
    print("%s: %d MiB toplam" % (out_path, total_mib))
    print("  bolum 1: ISO hibrit (GRUB + cekirdek), %d KiB" % (iso_size // 1024))
    print("  bolum %d: TCMKFS  lba=%d  %d sektor (%d MiB)"
          % (slot + 1, data_start, data_sectors, data_mib))


def main():
    iso = sys.argv[1] if len(sys.argv) > 1 else "build/tcmk-i386.iso"
    out = sys.argv[2] if len(sys.argv) > 2 else "build/tcmk-disk.img"
    mib = int(sys.argv[3]) if len(sys.argv) > 3 else 64
    build(iso, out, mib)


if __name__ == "__main__":
    main()
