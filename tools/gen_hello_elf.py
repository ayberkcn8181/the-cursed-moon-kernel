#!/usr/bin/env python3
"""Minimal statik ELF32 (i386) uretici -- TCMK Faz 3 userland testi.

Uretilen program Ring 3'te calisir ve yalnizca `int 0x80` ile iki Linux
sistem cagrisi yapar:

    sys_write(1, msg, len)   # EAX=4
    sys_exit(0)              # EAX=1

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
SYS_WRITE = 4
STDOUT = 1

MESSAGE = b"Hello from Ring 3 userland!\n"


def build_code(msg_vaddr: int, msg_len: int) -> bytes:
    """i386 makine kodu. Uzunlugu sabittir (29 bayt), bkz. CODE_LEN."""
    return b"".join(
        [
            b"\xb8" + struct.pack("<I", SYS_WRITE),   # mov eax, 4
            b"\xbb" + struct.pack("<I", STDOUT),      # mov ebx, 1
            b"\xb9" + struct.pack("<I", msg_vaddr),   # mov ecx, msg
            b"\xba" + struct.pack("<I", msg_len),     # mov edx, len
            b"\xcd\x80",                              # int 0x80
            b"\xb8" + struct.pack("<I", SYS_EXIT),    # mov eax, 1
            b"\xbb" + struct.pack("<I", 0),           # mov ebx, 0
            b"\xcd\x80",                              # int 0x80
            b"\xeb\xfe",                              # jmp $  (asla ulasilmaz)
        ]
    )


# Kod uzunlugu msg adresinden bagimsizdir; once bir kukla ile olcup
# gercek adresi hesapliyoruz.
CODE_LEN = len(build_code(0, 0))
MSG_OFF = CODE_OFF + CODE_LEN
MSG_VADDR = USER_BASE + MSG_OFF


def build_elf() -> bytes:
    code = build_code(MSG_VADDR, len(MESSAGE))
    assert len(code) == CODE_LEN, "kod uzunlugu sabit olmali"

    image_size = MSG_OFF + len(MESSAGE)
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

    # Tek bir PT_LOAD: ELF basliklari dahil tum dosyayi USER_BASE'e esler.
    phdr = struct.pack(
        "<IIIIIIII",
        1,            # p_type   = PT_LOAD
        0,            # p_offset
        USER_BASE,    # p_vaddr
        USER_BASE,    # p_paddr
        image_size,   # p_filesz
        image_size,   # p_memsz
        5,            # p_flags  = PF_R | PF_X
        0x1000,       # p_align
    )

    assert len(ehdr) == EHDR_SIZE
    assert len(phdr) == PHDR_SIZE
    return ehdr + phdr + code + MESSAGE


def main() -> None:
    out = sys.argv[1] if len(sys.argv) > 1 else "userland/hello.elf"
    data = build_elf()
    with open(out, "wb") as f:
        f.write(data)
    print(f"{out}: {len(data)} bayt, entry=0x{USER_BASE + CODE_OFF:08x}, "
          f"msg=0x{MSG_VADDR:08x}")


if __name__ == "__main__":
    main()
