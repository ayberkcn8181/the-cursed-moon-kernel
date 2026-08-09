#!/usr/bin/env python3
"""Minimal statik PE32 (Windows/i386) uretici -- TCMK Faz 7 testi.

Uretilen program Ring 3'te calisir ve NT sistem cagrilarini `int 0x2E` ile
yapar (doc S.3 Windows senaryosu):

    NtWriteConsole(1, msg, len)        # EAX=0x1001
    NtCreateFile("/boot/msg.txt")      # EAX=0x1002 -> handle EDX'te
    NtReadFile(handle, buf, 128)       # EAX=0x1003 -> okunan bayt EDX'te
    NtWriteConsole(1, buf, n)          # EAX=0x1001
    NtClose(handle)                    # EAX=0x1004
    NtTerminateProcess(0, 0)           # EAX=0x1000

ImageBase **bilincli olarak 0x00400000** (klasik Windows varsayilani)
secilmistir: cekirdek imaji 0x00300000'e yukledigi icin taban farki sifir
degildir ve yukleyicinin `.reloc` yolu gercekten calisir.

Kullanim:
    python3 tools/gen_pe_hello.py userland/hello.exe
"""

import struct
import sys

IMAGE_BASE = 0x0040_0000
SECTION_ALIGN = 0x1000
FILE_ALIGN = 0x200

TEXT_RVA = 0x1000
RDATA_RVA = 0x2000
BSS_RVA = 0x3000
RELOC_RVA = 0x4000
SIZE_OF_IMAGE = 0x5000

TEXT_RAW = 0x200
RDATA_RAW = 0x400
RELOC_RAW = 0x600

NT_TERMINATE_PROCESS = 0x1000
NT_WRITE_CONSOLE = 0x1001
NT_CREATE_FILE = 0x1002
NT_READ_FILE = 0x1003
NT_CLOSE = 0x1004

CONSOLE_HANDLE = 1
BUF_SIZE = 128

BANNER = b"Hello from Ring 3 PE (Windows NT uyumluluk katmani)!\n"
PATH = b"/boot/msg.txt\x00"


# --- i386 kodlayici (gen_hello_elf.py ile ayni yaklasim) -----------------

def mov_imm(reg: str, value: int) -> bytes:
    op = {"eax": 0xB8, "ecx": 0xB9, "edx": 0xBA, "ebx": 0xBB,
          "esi": 0xBE, "edi": 0xBF}[reg]
    return bytes([op]) + struct.pack("<I", value & 0xFFFFFFFF)


def mov_reg(dst: str, src: str) -> bytes:
    order = {"eax": 0, "ecx": 1, "edx": 2, "ebx": 3, "esp": 4, "ebp": 5,
             "esi": 6, "edi": 7}
    return bytes([0x89, 0xC0 | (order[src] << 3) | order[dst]])


INT2E = b"\xcd\x2e"
JMP_SELF = b"\xeb\xfe"


def build_code(banner_va: int, path_va: int, buf_va: int):
    """Kodu ve icindeki mutlak adreslerin (reloc gereken) ofsetlerini dondurur."""
    parts = []
    reloc_offsets = []

    def emit(chunk: bytes):
        parts.append(chunk)

    def emit_mov_abs(reg: str, va: int):
        # imm32 komutun 1. baytindan sonra gelir -> reloc hedefi orasi.
        offset = sum(len(p) for p in parts) + 1
        reloc_offsets.append(offset)
        emit(mov_imm(reg, va))

    # NtWriteConsole(1, banner, len)
    emit(mov_imm("eax", NT_WRITE_CONSOLE))
    emit(mov_imm("ebx", CONSOLE_HANDLE))
    emit_mov_abs("ecx", banner_va)
    emit(mov_imm("edx", len(BANNER)))
    emit(INT2E)

    # NtCreateFile(path) -> handle EDX'te; esi'ye sakla
    emit(mov_imm("eax", NT_CREATE_FILE))
    emit_mov_abs("ebx", path_va)
    emit(mov_imm("ecx", 0))
    emit(INT2E)
    emit(mov_reg("esi", "edx"))

    # NtReadFile(handle, buf, BUF_SIZE) -> okunan bayt EDX'te; edi'ye sakla
    emit(mov_imm("eax", NT_READ_FILE))
    emit(mov_reg("ebx", "esi"))
    emit_mov_abs("ecx", buf_va)
    emit(mov_imm("edx", BUF_SIZE))
    emit(INT2E)
    emit(mov_reg("edi", "edx"))

    # NtWriteConsole(1, buf, n)
    emit(mov_imm("eax", NT_WRITE_CONSOLE))
    emit(mov_imm("ebx", CONSOLE_HANDLE))
    emit_mov_abs("ecx", buf_va)
    emit(mov_reg("edx", "edi"))
    emit(INT2E)

    # NtClose(handle)
    emit(mov_imm("eax", NT_CLOSE))
    emit(mov_reg("ebx", "esi"))
    emit(INT2E)

    # NtTerminateProcess(0, 0) -- geri donmez
    emit(mov_imm("eax", NT_TERMINATE_PROCESS))
    emit(mov_imm("ebx", 0))
    emit(mov_imm("ecx", 0))
    emit(INT2E)

    emit(JMP_SELF)
    return b"".join(parts), reloc_offsets


def build_reloc_block(page_rva: int, offsets: list) -> bytes:
    """Tek bir .reloc blogu (IMAGE_REL_BASED_HIGHLOW = 3)."""
    entries = [(3 << 12) | (off & 0x0FFF) for off in offsets]
    # Blok boyutu 4'e hizali olmali; gerekirse ABSOLUTE (tip 0) dolgu ekle.
    if len(entries) % 2 == 1:
        entries.append(0)
    size = 8 + 2 * len(entries)
    return struct.pack("<II", page_rva, size) + b"".join(
        struct.pack("<H", e) for e in entries
    )


def build_pe() -> bytes:
    banner_va = IMAGE_BASE + RDATA_RVA
    path_va = banner_va + len(BANNER)
    buf_va = IMAGE_BASE + BSS_RVA

    code, reloc_offsets = build_code(banner_va, path_va, buf_va)
    rdata = BANNER + PATH
    reloc = build_reloc_block(TEXT_RVA, reloc_offsets)

    # --- DOS basligi + stub ---
    dos = bytearray(64)
    dos[0:2] = b"MZ"
    struct.pack_into("<I", dos, 0x3C, 64)  # e_lfanew -> PE imzasi hemen sonra

    # --- COFF basligi ---
    num_sections = 4  # .text, .rdata, .bss, .reloc
    size_of_optional = 224
    coff = struct.pack(
        "<HHIIIHH",
        0x014C,            # Machine = IMAGE_FILE_MACHINE_I386
        num_sections,
        0,                 # TimeDateStamp
        0,                 # PointerToSymbolTable
        0,                 # NumberOfSymbols
        size_of_optional,  # DIKKAT: 0 birakilirsa bolum tablosu bulunamaz!
        0x0102,            # EXECUTABLE_IMAGE | 32BIT_MACHINE
    )

    # --- Optional header (PE32, 224 bayt) ---
    opt = struct.pack(
        "<HBBIIIIIII",
        0x010B,            # Magic = PE32
        0, 0,              # linker versiyonu
        len(code),         # SizeOfCode
        len(rdata),        # SizeOfInitializedData
        BUF_SIZE,          # SizeOfUninitializedData
        TEXT_RVA,          # AddressOfEntryPoint
        TEXT_RVA,          # BaseOfCode
        RDATA_RVA,         # BaseOfData
        IMAGE_BASE,        # ImageBase
    )
    opt += struct.pack(
        "<IIHHHHHHIIIIHH",
        SECTION_ALIGN,
        FILE_ALIGN,
        4, 0,              # MajorOperatingSystemVersion, Minor
        0, 0,              # MajorImageVersion, Minor
        4, 0,              # MajorSubsystemVersion, Minor
        0,                 # Win32VersionValue
        SIZE_OF_IMAGE,
        FILE_ALIGN,        # SizeOfHeaders
        0,                 # CheckSum
        3,                 # Subsystem = WINDOWS_CUI
        0,                 # DllCharacteristics
    )
    opt += struct.pack(
        "<IIIIII",
        0x100000, 0x1000,  # SizeOfStackReserve / Commit
        0x100000, 0x1000,  # SizeOfHeapReserve / Commit
        0,                 # LoaderFlags
        16,                # NumberOfRvaAndSizes
    )
    # 16 adet veri dizini; yalnizca BASERELOC (index 5) dolu.
    dirs = []
    for i in range(16):
        if i == 5:
            dirs.append(struct.pack("<II", RELOC_RVA, len(reloc)))
        else:
            dirs.append(struct.pack("<II", 0, 0))
    opt += b"".join(dirs)
    assert len(opt) == size_of_optional, f"optional header {len(opt)} != 224"

    # --- Bolum tablosu ---
    def section(name, vsize, rva, rawsize, rawptr, chars):
        return struct.pack(
            "<8sIIIIIIHHI",
            name, vsize, rva, rawsize, rawptr, 0, 0, 0, 0, chars
        )

    CODE_CHARS = 0x60000020   # CODE | EXECUTE | READ
    RDATA_CHARS = 0x40000040  # INITIALIZED_DATA | READ
    BSS_CHARS = 0xC0000080    # UNINITIALIZED_DATA | READ | WRITE
    RELOC_CHARS = 0x42000040  # INITIALIZED_DATA | DISCARDABLE | READ

    def align_up(v, a):
        return (v + a - 1) & ~(a - 1)

    sections = b"".join([
        section(b".text\0\0\0", len(code), TEXT_RVA,
                align_up(len(code), FILE_ALIGN), TEXT_RAW, CODE_CHARS),
        section(b".rdata\0\0", len(rdata), RDATA_RVA,
                align_up(len(rdata), FILE_ALIGN), RDATA_RAW, RDATA_CHARS),
        section(b".bss\0\0\0\0", BUF_SIZE, BSS_RVA, 0, 0, BSS_CHARS),
        section(b".reloc\0\0", len(reloc), RELOC_RVA,
                align_up(len(reloc), FILE_ALIGN), RELOC_RAW, RELOC_CHARS),
    ])

    headers = bytes(dos) + b"PE\0\0" + coff + opt + sections
    assert len(headers) <= FILE_ALIGN, f"basliklar {len(headers)} > {FILE_ALIGN}"

    image = bytearray(RELOC_RAW + align_up(len(reloc), FILE_ALIGN))
    image[0:len(headers)] = headers
    image[TEXT_RAW:TEXT_RAW + len(code)] = code
    image[RDATA_RAW:RDATA_RAW + len(rdata)] = rdata
    image[RELOC_RAW:RELOC_RAW + len(reloc)] = reloc
    return bytes(image)


def main() -> None:
    out = sys.argv[1] if len(sys.argv) > 1 else "userland/hello.exe"
    data = build_pe()
    with open(out, "wb") as f:
        f.write(data)
    print(
        f"{out}: {len(data)} bayt, ImageBase=0x{IMAGE_BASE:08x}, "
        f"entry RVA=0x{TEXT_RVA:x} (yukleme 0x00300000 -> reloc delta uygulanir)"
    )


if __name__ == "__main__":
    main()
