#!/usr/bin/env python3
"""Minimal statik ELF32 (i386) uretici -- TCMK userland test programi.

Uretilen program Ring 3'te calisir ve yalnizca `int 0x80` ile Linux sistem
cagrilari yapar (doc S.6 ABI: EAX=numara, EBX/ECX/EDX=arg1..3):

    sys_write(1, banner, ...)         # EAX=4   selamlama
    fd = sys_open("/boot/msg.txt")    # EAX=5   VFS'ten dosya ac
    n  = sys_read(fd, buf, 128)       # EAX=3   icerigi oku
    sys_write(1, buf, n)              # EAX=4   stdout'a yaz
    sys_close(fd)                     # EAX=6
    sys_brk(0)                        # EAX=45  mevcut program break
    sys_exit(0)                       # EAX=1

Harici assembler/linker gerektirmemesi bilincli bir tercihtir: TCMK'nin
arac zinciri Rust + GRUB + QEMU ile sinirli tutulur (bkz. README).

Kullanim:
    python3 tools/gen_hello_elf.py userland/hello.elf
"""

import struct
import sys

# Kullanici bellek bolgesi -- kernel/src/level0a/core/mmu_i386.rs icindeki
# USER_MEM_START ile ayni olmalidir.
USER_BASE = 0x0030_0000

EHDR_SIZE = 52
PHDR_SIZE = 32
CODE_OFF = EHDR_SIZE + PHDR_SIZE  # ehdr + tek phdr'den hemen sonra

SYS_EXIT = 1
SYS_READ = 3
SYS_WRITE = 4
SYS_OPEN = 5
SYS_CLOSE = 6
SYS_BRK = 45
STDOUT = 1

BANNER = b"Hello from Ring 3 userland!\n"
PATH = b"/boot/msg.txt\x00"
BUF_SIZE = 128


# --- Kucuk bir i386 kodlayici -------------------------------------------
# Tum komutlar sabit uzunluktadir; bu sayede iki gecisli yerlesim
# (once olcum, sonra gercek adresler) guvenlidir.

def mov_imm(reg: str, value: int) -> bytes:
    opcodes = {"eax": 0xB8, "ecx": 0xB9, "edx": 0xBA, "ebx": 0xBB,
               "esi": 0xBE, "edi": 0xBF}
    return bytes([opcodes[reg]]) + struct.pack("<I", value & 0xFFFFFFFF)


def mov_reg(dst: str, src: str) -> bytes:
    # 89 /r  -> mov r/m32, r32   (mod=11)
    order = {"eax": 0, "ecx": 1, "edx": 2, "ebx": 3, "esp": 4, "ebp": 5,
             "esi": 6, "edi": 7}
    modrm = 0xC0 | (order[src] << 3) | order[dst]
    return bytes([0x89, modrm])


INT80 = b"\xcd\x80"
JMP_SELF = b"\xeb\xfe"


def build_code(banner_va: int, path_va: int, buf_va: int) -> bytes:
    return b"".join([
        # sys_write(1, banner, len(BANNER))
        mov_imm("eax", SYS_WRITE),
        mov_imm("ebx", STDOUT),
        mov_imm("ecx", banner_va),
        mov_imm("edx", len(BANNER)),
        INT80,

        # fd = sys_open(path, 0, 0);  fd -> esi
        mov_imm("eax", SYS_OPEN),
        mov_imm("ebx", path_va),
        mov_imm("ecx", 0),
        mov_imm("edx", 0),
        INT80,
        mov_reg("esi", "eax"),

        # n = sys_read(fd, buf, BUF_SIZE);  n -> edi
        mov_imm("eax", SYS_READ),
        mov_reg("ebx", "esi"),
        mov_imm("ecx", buf_va),
        mov_imm("edx", BUF_SIZE),
        INT80,
        mov_reg("edi", "eax"),

        # sys_write(1, buf, n)
        mov_imm("eax", SYS_WRITE),
        mov_imm("ebx", STDOUT),
        mov_imm("ecx", buf_va),
        mov_reg("edx", "edi"),
        INT80,

        # sys_close(fd)
        mov_imm("eax", SYS_CLOSE),
        mov_reg("ebx", "esi"),
        INT80,

        # sys_brk(0) -- mevcut program break'i sorgula
        mov_imm("eax", SYS_BRK),
        mov_imm("ebx", 0),
        INT80,

        # sys_exit(0)
        mov_imm("eax", SYS_EXIT),
        mov_imm("ebx", 0),
        INT80,

        JMP_SELF,  # asla ulasilmaz
    ])


# 1. gecis: kod uzunlugunu olc (adreslerden bagimsizdir).
CODE_LEN = len(build_code(0, 0, 0))

BANNER_OFF = CODE_OFF + CODE_LEN
PATH_OFF = BANNER_OFF + len(BANNER)
FILE_SIZE = PATH_OFF + len(PATH)
# .bss benzeri tampon: dosyada yer kaplamaz, yalnizca memsz'de.
BUF_OFF = (FILE_SIZE + 3) & ~3
MEM_SIZE = BUF_OFF + BUF_SIZE


def build_elf() -> bytes:
    banner_va = USER_BASE + BANNER_OFF
    path_va = USER_BASE + PATH_OFF
    buf_va = USER_BASE + BUF_OFF

    # 2. gecis: gercek adreslerle kodu uret.
    code = build_code(banner_va, path_va, buf_va)
    assert len(code) == CODE_LEN, "kod uzunlugu sabit olmali"

    entry = USER_BASE + CODE_OFF

    ehdr = struct.pack(
        "<16sHHIIIIIHHHHHH",
        b"\x7fELF\x01\x01\x01\x00" + b"\x00" * 8,  # e_ident
        2,           # e_type    = ET_EXEC
        3,           # e_machine = EM_386
        1,           # e_version
        entry,       # e_entry
        EHDR_SIZE,   # e_phoff
        0,           # e_shoff
        0,           # e_flags
        EHDR_SIZE,   # e_ehsize
        PHDR_SIZE,   # e_phentsize
        1,           # e_phnum
        0,           # e_shentsize
        0,           # e_shnum
        0,           # e_shstrndx
    )

    # Tek bir PT_LOAD: ELF basliklari dahil tum imaji USER_BASE'e esler.
    # memsz > filesz oldugu icin cekirdek aradaki farki sifirlar (.bss).
    phdr = struct.pack(
        "<IIIIIIII",
        1,            # p_type   = PT_LOAD
        0,            # p_offset
        USER_BASE,    # p_vaddr
        USER_BASE,    # p_paddr
        FILE_SIZE,    # p_filesz
        MEM_SIZE,     # p_memsz
        7,            # p_flags  = PF_R | PF_W | PF_X (tampon yazilabilir)
        0x1000,       # p_align
    )

    assert len(ehdr) == EHDR_SIZE
    assert len(phdr) == PHDR_SIZE

    image = bytearray(ehdr + phdr + code + BANNER + PATH)
    assert len(image) == FILE_SIZE, f"{len(image)} != {FILE_SIZE}"
    return bytes(image)


def main() -> None:
    out = sys.argv[1] if len(sys.argv) > 1 else "userland/hello.elf"
    data = build_elf()
    with open(out, "wb") as f:
        f.write(data)
    print(
        f"{out}: filesz={FILE_SIZE} memsz={MEM_SIZE} "
        f"entry=0x{USER_BASE + CODE_OFF:08x} buf=0x{USER_BASE + BUF_OFF:08x}"
    )


if __name__ == "__main__":
    main()
