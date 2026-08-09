#!/usr/bin/env python3
"""Minimal statik ELF64 (x86_64) uretici -- TCMK Faz 4 userland testi.

`gen_hello_elf.py`'nin 64-bit karsiligi. Program Ring 3'te calisir ve
sistem cagrilarini **`syscall` komutuyla** yapar (doc S.15: x86_64 -> syscall).

x86_64 Linux ABI (doc S.6):
    RAX = numara, RDI/RSI/RDX = arg1..3, donus RAX
    NOT: `syscall` RCX ve R11'i ezer.

Akis:
    sys_write(1, banner, len)         # RAX=1
    fd = sys_open("/boot/msg.txt")    # RAX=2
    n  = sys_read(fd, buf, 128)       # RAX=0
    sys_write(1, buf, n)              # RAX=1
    sys_close(fd)                     # RAX=3
    sys_brk(0)                        # RAX=12
    sys_exit(0)                       # RAX=60

Kullanim:
    python3 tools/gen_hello_elf64.py userland/hello64.elf
"""

import struct
import sys

USER_BASE = 0x0030_0000

EHDR_SIZE = 64
PHDR_SIZE = 56
CODE_OFF = EHDR_SIZE + PHDR_SIZE

# x86_64 Linux syscall numaralari
SYS_READ = 0
SYS_WRITE = 1
SYS_OPEN = 2
SYS_CLOSE = 3
SYS_BRK = 12
SYS_EXIT = 60
STDOUT = 1

BANNER = b"Hello from Ring 3 userland (x86_64, syscall)!\n"
PATH = b"/boot/msg.txt\x00"
BUF_SIZE = 128


# --- Kucuk bir x86_64 kodlayici; tum komutlar sabit uzunlukta ---

def mov_imm32(reg: str, value: int) -> bytes:
    """mov r32, imm32 -- ust 32 biti sifirlar, adreslerimiz 4 GiB altinda."""
    op = {"eax": 0xB8, "ecx": 0xB9, "edx": 0xBA, "ebx": 0xBB,
          "esi": 0xBE, "edi": 0xBF}[reg]
    return bytes([op]) + struct.pack("<I", value & 0xFFFFFFFF)


def mov_r64(dst: str, src: str) -> bytes:
    """mov r64, r64 (REX.W + 89 /r)."""
    order = {"rax": 0, "rcx": 1, "rdx": 2, "rbx": 3, "rsp": 4, "rbp": 5,
             "rsi": 6, "rdi": 7, "r8": 8, "r9": 9, "r10": 10, "r11": 11,
             "r12": 12, "r13": 13, "r14": 14, "r15": 15}
    d, s = order[dst], order[src]
    rex = 0x48 | ((s >> 3) << 2) | (d >> 3)
    modrm = 0xC0 | ((s & 7) << 3) | (d & 7)
    return bytes([rex, 0x89, modrm])


SYSCALL = b"\x0f\x05"
JMP_SELF = b"\xeb\xfe"


def build_code(banner_va: int, path_va: int, buf_va: int) -> bytes:
    return b"".join([
        # sys_write(1, banner, len)
        mov_imm32("eax", SYS_WRITE),
        mov_imm32("edi", STDOUT),
        mov_imm32("esi", banner_va),
        mov_imm32("edx", len(BANNER)),
        SYSCALL,

        # fd = sys_open(path, 0, 0)  -> r12'ye sakla (callee-saved degil ama
        # cekirdek cerceveyi koruyor; yine de rbx yerine r12 secildi)
        mov_imm32("eax", SYS_OPEN),
        mov_imm32("edi", path_va),
        mov_imm32("esi", 0),
        mov_imm32("edx", 0),
        SYSCALL,
        mov_r64("r12", "rax"),

        # n = sys_read(fd, buf, BUF_SIZE) -> r13
        mov_imm32("eax", SYS_READ),
        mov_r64("rdi", "r12"),
        mov_imm32("esi", buf_va),
        mov_imm32("edx", BUF_SIZE),
        SYSCALL,
        mov_r64("r13", "rax"),

        # sys_write(1, buf, n)
        mov_imm32("eax", SYS_WRITE),
        mov_imm32("edi", STDOUT),
        mov_imm32("esi", buf_va),
        mov_r64("rdx", "r13"),
        SYSCALL,

        # sys_close(fd)
        mov_imm32("eax", SYS_CLOSE),
        mov_r64("rdi", "r12"),
        SYSCALL,

        # sys_brk(0)
        mov_imm32("eax", SYS_BRK),
        mov_imm32("edi", 0),
        SYSCALL,

        # sys_exit(0)
        mov_imm32("eax", SYS_EXIT),
        mov_imm32("edi", 0),
        SYSCALL,

        JMP_SELF,
    ])


CODE_LEN = len(build_code(0, 0, 0))
BANNER_OFF = CODE_OFF + CODE_LEN
PATH_OFF = BANNER_OFF + len(BANNER)
FILE_SIZE = PATH_OFF + len(PATH)
BUF_OFF = (FILE_SIZE + 7) & ~7
MEM_SIZE = BUF_OFF + BUF_SIZE


def build_elf() -> bytes:
    banner_va = USER_BASE + BANNER_OFF
    path_va = USER_BASE + PATH_OFF
    buf_va = USER_BASE + BUF_OFF

    code = build_code(banner_va, path_va, buf_va)
    assert len(code) == CODE_LEN, "kod uzunlugu sabit olmali"

    entry = USER_BASE + CODE_OFF

    ehdr = struct.pack(
        "<16sHHIQQQIHHHHHH",
        b"\x7fELF\x02\x01\x01\x00" + b"\x00" * 8,  # e_ident (ELFCLASS64)
        2,           # e_type    = ET_EXEC
        62,          # e_machine = EM_X86_64
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

    # ELF64 program header: p_flags p_offset'ten HEMEN SONRA gelir
    # (ELF32'de sonda idi) -- klasik bir karistirma noktasi.
    phdr = struct.pack(
        "<IIQQQQQQ",
        1,            # p_type  = PT_LOAD
        7,            # p_flags = PF_R | PF_W | PF_X
        0,            # p_offset
        USER_BASE,    # p_vaddr
        USER_BASE,    # p_paddr
        FILE_SIZE,    # p_filesz
        MEM_SIZE,     # p_memsz
        0x1000,       # p_align
    )

    assert len(ehdr) == EHDR_SIZE, f"ehdr {len(ehdr)} != {EHDR_SIZE}"
    assert len(phdr) == PHDR_SIZE, f"phdr {len(phdr)} != {PHDR_SIZE}"

    image = ehdr + phdr + code + BANNER + PATH
    assert len(image) == FILE_SIZE
    return image


def main() -> None:
    out = sys.argv[1] if len(sys.argv) > 1 else "userland/hello64.elf"
    data = build_elf()
    with open(out, "wb") as f:
        f.write(data)
    print(
        f"{out}: filesz={FILE_SIZE} memsz={MEM_SIZE} "
        f"entry=0x{USER_BASE + CODE_OFF:08x} buf=0x{USER_BASE + BUF_OFF:08x}"
    )


if __name__ == "__main__":
    main()
