#!/usr/bin/env python3
"""TCMK disk imaji ureteci.

`grub-mkrescue` bir **hibrit** imaj uretir: hem CD (ISO9660) hem sabit disk
olarak acilabilen, gecerli bir MBR'ye sahip tek dosya. Bu betik onun sonuna
TCMKFS icin bir bolum ekler, bolum tablosuna isler ve bolumun **onyukleyici
alanini** doldurur:

    bolum 1  ISO hibrit -> GRUB + cekirdek (salt okunur, grub-mkrescue kurdu)
    bolum 2  0x7F       -> TCMKFS
               sektor   40..71    2. asama (yamalanmis)
               sektor   72..      cekirdek imaji (ham bellek blogu)
               sektor 4096..      dosya sistemi veri bloklari

Boylece **root gerekmeden** uretilen imaj hem GRUB ile hem de -- kabuktan
`install` calistirildiktan sonra -- TCMK'nin kendi onyukleyicisiyle acilir.

Cekirdek ELF'i diske **cozulmus** halde yazilir: tum PT_LOAD segmentleri
bellekteki yerlerine gore tek bir ardisik bloga yerlestirilir. Boylece
2. asamanin ELF ayristirmasina ihtiyaci kalmaz.

Kullanim:
    python3 tools/make_disk.py <iso> <cikis> <mib> <stage2.bin> <kernel.elf>
"""

import os
import shutil
import struct
import sys

SECTOR = 512
#: TCMKFS bolum turu -- cekirdekteki partition::TYPE_TCMKFS ile ayni olmali.
TYPE_TCMKFS = 0x7F
PARTITION_TABLE_OFFSET = 446
ENTRY_SIZE = 16

#: Bolum baslangicina gore -- core/tcmkfs.rs ile ayni olmali.
BOOT_AREA_SECTOR = 40
STAGE2_SECTORS = 32
KERNEL_BLOB_SECTOR = BOOT_AREA_SECTOR + STAGE2_SECTORS
DATA_START_SECTOR = 8192

#: Cekirdek 1 MiB'a linklenir (boot/linker/i386.ld).
KERNEL_BASE = 0x0010_0000

#: 2. asamanin parametre bloku (boot/tcmkboot/src/bin/stage2.rs).
STAGE2_MAGIC = b"TCMKB2"
STAGE2_MAGIC_OFFSET = 3
P_BLOB_LBA = 9
P_BLOB_SECTORS = 13
P_LOAD_ADDR = 17
P_ENTRY = 21
P_BSS_START = 25
P_BSS_END = 29


def chs_dummy():
    """LBA kullanildigi icin CHS alanlari 'buyuk disk' isaretiyle doldurulur.

    0xFE 0xFF 0xFF, "bu bolum CHS ile adreslenemez, LBA alanlarina bak"
    anlamina gelen yaygin kaliptir.
    """
    return bytes([0xFE, 0xFF, 0xFF])


def read_elf_segments(path):
    """ELF32'nin PT_LOAD segmentlerini (offset, paddr, filesz, memsz) doner."""
    data = open(path, "rb").read()
    if data[:4] != b"\x7fELF" or data[4] != 1:
        sys.exit("ELF32 bekleniyordu: %s" % path)

    entry, phoff = struct.unpack_from("<II", data, 24)
    phentsize, phnum = struct.unpack_from("<HH", data, 42)

    segments = []
    for i in range(phnum):
        base = phoff + i * phentsize
        p_type, p_offset, _vaddr, p_paddr, p_filesz, p_memsz = struct.unpack_from(
            "<IIIIII", data, base)
        if p_type == 1 and p_memsz:  # PT_LOAD
            segments.append((p_offset, p_paddr, p_filesz, p_memsz))
    if not segments:
        sys.exit("PT_LOAD segmenti yok: %s" % path)
    return data, entry, segments


def build_blob(kernel_path):
    """ELF'i cozup ardisik bir bellek imaji uretir.

    Doner: (blob, entry, bss_start, bss_end)
    `bss_start`..`bss_end`, dosyada karsiligi olmayan ama sifirlanmasi
    gereken alandir (`.bss`).
    """
    data, entry, segments = read_elf_segments(kernel_path)

    file_end = max(paddr + filesz for _, paddr, filesz, _ in segments)
    mem_end = max(paddr + memsz for _, paddr, _, memsz in segments)
    lowest = min(paddr for _, paddr, _, _ in segments)
    if lowest != KERNEL_BASE:
        sys.exit("cekirdek 0x%08x'e linklenmemis (0x%08x bulundu)"
                 % (KERNEL_BASE, lowest))

    blob = bytearray(file_end - KERNEL_BASE)
    for offset, paddr, filesz, _ in segments:
        start = paddr - KERNEL_BASE
        blob[start:start + filesz] = data[offset:offset + filesz]

    return blob, entry, file_end, mem_end


def patch_stage2(stage2, blob_lba, blob_sectors, entry, bss_start, bss_end):
    s = bytearray(stage2)
    if s[STAGE2_MAGIC_OFFSET:STAGE2_MAGIC_OFFSET + 6] != STAGE2_MAGIC:
        sys.exit("2. asama imzasi bulunamadi (yanlis ikili?)")
    struct.pack_into("<I", s, P_BLOB_LBA, blob_lba)
    struct.pack_into("<I", s, P_BLOB_SECTORS, blob_sectors)
    struct.pack_into("<I", s, P_LOAD_ADDR, KERNEL_BASE)
    struct.pack_into("<I", s, P_ENTRY, entry)
    struct.pack_into("<I", s, P_BSS_START, bss_start)
    struct.pack_into("<I", s, P_BSS_END, bss_end)
    return bytes(s)


def build(iso_path, out_path, data_mib, stage2_path, kernel_path):
    if not os.path.exists(iso_path):
        sys.exit("ISO bulunamadi: %s (once `make iso`)" % iso_path)

    stage2 = open(stage2_path, "rb").read()
    if len(stage2) > STAGE2_SECTORS * SECTOR:
        sys.exit("2. asama %d sektore sigmiyor" % STAGE2_SECTORS)

    blob, entry, bss_start, bss_end = build_blob(kernel_path)
    blob_sectors = (len(blob) + SECTOR - 1) // SECTOR
    if KERNEL_BLOB_SECTOR + blob_sectors > DATA_START_SECTOR:
        sys.exit("cekirdek blogu onyukleyici alanina sigmiyor (%d sektor)"
                 % blob_sectors)

    shutil.copyfile(iso_path, out_path)
    iso_size = os.path.getsize(out_path)

    # TCMKFS bolgesi 1 MiB sinirina hizalanir.
    align = (1024 * 1024) // SECTOR
    part_start = (iso_size + SECTOR - 1) // SECTOR
    part_start = ((part_start + align - 1) // align) * align
    part_sectors = data_mib * 1024 * 1024 // SECTOR

    stage2 = patch_stage2(
        stage2,
        part_start + KERNEL_BLOB_SECTOR,
        blob_sectors,
        entry,
        bss_start,
        bss_end,
    )

    with open(out_path, "r+b") as f:
        f.truncate((part_start + part_sectors) * SECTOR)

        f.seek(0)
        mbr = bytearray(f.read(SECTOR))
        if mbr[510] != 0x55 or mbr[511] != 0xAA:
            sys.exit("ISO hibrit MBR icermiyor; grub-mkrescue surumunu kontrol edin.")

        slot = None
        for i in range(4):
            base = PARTITION_TABLE_OFFSET + i * ENTRY_SIZE
            if mbr[base + 4] == 0:
                slot = i
                break
        if slot is None:
            sys.exit("MBR'de bos bolum girdisi yok.")

        entry_bytes = struct.pack(
            "<B3sB3sII",
            0x00,               # acilis bayragi yok (GRUB 1. bolumden acar)
            chs_dummy(),
            TYPE_TCMKFS,
            chs_dummy(),
            part_start,
            part_sectors,
        )
        base = PARTITION_TABLE_OFFSET + slot * ENTRY_SIZE
        mbr[base:base + ENTRY_SIZE] = entry_bytes
        f.seek(0)
        f.write(mbr)

        # Onyukleyici alani: 2. asama ve cekirdek blogu.
        f.seek((part_start + BOOT_AREA_SECTOR) * SECTOR)
        f.write(stage2)
        f.seek((part_start + KERNEL_BLOB_SECTOR) * SECTOR)
        f.write(blob)

    total_mib = os.path.getsize(out_path) // (1024 * 1024)
    print("%s: %d MiB toplam" % (out_path, total_mib))
    print("  bolum 1: ISO hibrit (GRUB + cekirdek), %d KiB" % (iso_size // 1024))
    print("  bolum %d: TCMKFS  lba=%d  %d MiB" % (slot + 1, part_start, data_mib))
    print("    2. asama     : lba=%d  %d bayt"
          % (part_start + BOOT_AREA_SECTOR, len(stage2)))
    print("    cekirdek blob: lba=%d  %d sektor (%d KiB)"
          % (part_start + KERNEL_BLOB_SECTOR, blob_sectors, len(blob) // 1024))
    print("    entry=0x%08x  bss=0x%08x..0x%08x" % (entry, bss_start, bss_end))


def main():
    if len(sys.argv) < 6:
        sys.exit(__doc__)
    build(sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4], sys.argv[5])


if __name__ == "__main__":
    main()
