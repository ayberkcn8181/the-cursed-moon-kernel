#!/usr/bin/env python3
"""TCMK GUI uygulamasi ureteci (ELF32, i386).

Ring 3'te calisan gercek bir grafik uygulamasi uretir. Uygulama:

    id  = win_create(title, (x<<16)|y, (w<<16)|h)   # 0x500
    buf = win_buffer(id)                            # 0x501
    dongu:
        pencereyi bir desenle boya (dogrudan buf'a yazarak)
        k = win_poll_key(id)                        # 0x504
        win_flush(id)                               # 0x503 -> CPU'yu birak

Cizim cekirdekten gecmez: uygulama kendi piksel tamponuna dogrudan yazar.
Bu, "uygulama gercekten calisiyor" iddiasinin somut kanitidir.

Kullanim:
    python3 tools/gen_gui_app.py userland/paint.elf paint
"""

import struct
import sys

# Kullanici bolgesi (2 MiB) esit "slot"lara bolunur; her uygulama kendi
# slotuna yuklenir. Ayni anda birden fazla Ring 3 uygulamasi calistigi ve
# henuz surec basina adres uzayi olmadigi icin bu sart: ayni tabana
# yuklenen iki uygulama birbirinin kodunu ezerdi.
USER_REGION = 0x00C0_0000
SLOT_SIZE = 0x0004_0000  # 256 KiB

EHDR_SIZE = 52
PHDR_SIZE = 32
CODE_OFF = EHDR_SIZE + PHDR_SIZE

SYS_EXIT = 1
SYS_WIN_CREATE = 0x500
SYS_WIN_BUFFER = 0x501
SYS_WIN_POLL_KEY = 0x504
SYS_MOUSE_STATE = 0x505
SYS_WIN_FLUSH = 0x503


def mov_imm(reg, value):
    op = {"eax": 0xB8, "ecx": 0xB9, "edx": 0xBA, "ebx": 0xBB,
          "esi": 0xBE, "edi": 0xBF, "ebp": 0xBD}[reg]
    return bytes([op]) + struct.pack("<I", value & 0xFFFFFFFF)


def mov_reg(dst, src):
    order = {"eax": 0, "ecx": 1, "edx": 2, "ebx": 3, "esp": 4, "ebp": 5,
             "esi": 6, "edi": 7}
    return bytes([0x89, 0xC0 | (order[src] << 3) | order[dst]])


INT80 = b"\xcd\x80"


APPS = {
    # ad: (baslik, x, y, w, h, desen, slot)
    "paint":  ("TCMK Paint",   30,  60, 320, 200, 0, 1),
    "plasma": ("TCMK Plasma", 700, 300, 300, 200, 1, 2),
}


def build_code(app, title_va, win_geom, win_size):
    """Uygulamanin makine kodu.

    Register kullanimi:
        esi = pencere kimligi
        edi = piksel tamponu adresi
        ebp = kare sayaci (animasyon icin)
    """
    _, _, _, w, h, pattern, _slot = app
    parts = []

    def emit(b):
        parts.append(b)

    # id = win_create(title, geom, size)
    emit(mov_imm("eax", SYS_WIN_CREATE))
    emit(mov_imm("ebx", title_va))
    emit(mov_imm("ecx", win_geom))
    emit(mov_imm("edx", win_size))
    emit(INT80)
    emit(mov_reg("esi", "eax"))

    # buf = win_buffer(id)
    emit(mov_imm("eax", SYS_WIN_BUFFER))
    emit(mov_reg("ebx", "esi"))
    emit(INT80)
    emit(mov_reg("edi", "eax"))

    # Tampon alinamadiysa (0) cik.
    # DIKKAT: atlama, cikis dizisinin TAMAMINI (mov 5 + mov 5 + int80 2 = 12)
    # gecmelidir; eksik hesaplarsa int 0x80'in ortasina dusulur.
    emit(b"\x85\xff")                    # test edi, edi
    emit(b"\x75\x0c")                    # jnz +12
    emit(mov_imm("eax", SYS_EXIT))       # 5
    emit(mov_imm("ebx", 1))              # 5  (cikis kodu 1 = pencere yok)
    emit(INT80)                          # 2

    emit(mov_imm("ebp", 0))              # kare sayaci

    # --- ana dongu ---
    loop_start = sum(len(p) for p in parts)

    # ecx = piksel indeksi = 0
    emit(mov_imm("ecx", 0))

    pixel_loop = sum(len(p) for p in parts)

    # Deseni hesapla: edx = renk
    if pattern == 0:
        # Yatay gradyan + kare sayaci kaymasi:
        # renk = ((ecx + ebp) & 0xFF) << 8 | 0x40
        emit(mov_reg("edx", "ecx"))
        emit(b"\x01\xea")                # add edx, ebp
        emit(b"\x81\xe2\xff\x00\x00\x00")  # and edx, 0xFF
        emit(b"\xc1\xe2\x08")            # shl edx, 8
        emit(b"\x81\xca\x40\x00\x00\x40")  # or edx, 0x40000040
    else:
        # XOR deseni: renk = ((ecx ^ (ecx>>7) ^ ebp) & 0xFF) * 0x010101
        emit(mov_reg("edx", "ecx"))
        emit(b"\x89\xc8")                # mov eax, ecx
        emit(b"\xc1\xe8\x07")            # shr eax, 7
        emit(b"\x31\xc2")                # xor edx, eax
        emit(b"\x31\xea")                # xor edx, ebp
        emit(b"\x81\xe2\xff\x00\x00\x00")  # and edx, 0xFF
        emit(b"\x69\xd2\x01\x01\x01\x00")  # imul edx, edx, 0x00010101

    # buf[ecx] = edx   ->  mov [edi + ecx*4], edx
    emit(b"\x89\x14\x8f")

    # ecx++ ; ecx < w*h ?
    emit(b"\x41")                        # inc ecx
    emit(b"\x81\xf9" + struct.pack("<I", w * h))  # cmp ecx, w*h
    rel = pixel_loop - (sum(len(p) for p in parts) + 6)
    emit(b"\x0f\x82" + struct.pack("<i", rel))    # jb pixel_loop

    # k = win_poll_key(id) -- 'q' ise cik
    emit(mov_imm("eax", SYS_WIN_POLL_KEY))
    emit(mov_reg("ebx", "esi"))
    emit(INT80)
    emit(b"\x3c\x71")                    # cmp al, 'q'
    emit(b"\x75\x0c")                    # jne +12 (cikis dizisini atla)
    emit(mov_imm("eax", SYS_EXIT))       # 5
    emit(mov_imm("ebx", 0))              # 5
    emit(INT80)                          # 2

    # win_flush(id) -> CPU'yu birak
    emit(mov_imm("eax", SYS_WIN_FLUSH))
    emit(mov_reg("ebx", "esi"))
    emit(INT80)

    # ebp += 3 (animasyon hizi)
    emit(b"\x83\xc5\x03")                # add ebp, 3

    rel = loop_start - (sum(len(p) for p in parts) + 5)
    emit(b"\xe9" + struct.pack("<i", rel))  # jmp loop_start

    return b"".join(parts)


def build_elf(name):
    app = APPS[name]
    title = app[0].encode() + b"\x00"
    x, y, w, h = app[1], app[2], app[3], app[4]
    user_base = USER_REGION + app[6] * SLOT_SIZE
    win_geom = (x << 16) | y
    win_size = (w << 16) | h

    code_len = len(build_code(app, 0, win_geom, win_size))
    title_off = CODE_OFF + code_len
    title_va = user_base + title_off

    code = build_code(app, title_va, win_geom, win_size)
    assert len(code) == code_len, "kod uzunlugu sabit olmali"

    file_size = title_off + len(title)
    entry = user_base + CODE_OFF

    ehdr = struct.pack(
        "<16sHHIIIIIHHHHHH",
        b"\x7fELF\x01\x01\x01\x00" + b"\x00" * 8,
        2, 3, 1, entry, EHDR_SIZE, 0, 0,
        EHDR_SIZE, PHDR_SIZE, 1, 0, 0, 0,
    )
    phdr = struct.pack(
        "<IIIIIIII",
        1, 0, user_base, user_base, file_size, file_size, 7, 0x1000,
    )
    return ehdr + phdr + code + title


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "userland/paint.elf"
    name = sys.argv[2] if len(sys.argv) > 2 else "paint"
    data = build_elf(name)
    with open(out, "wb") as f:
        f.write(data)
    base = USER_REGION + APPS[name][6] * SLOT_SIZE
    print(f"{out}: {len(data)} bayt, uygulama='{name}', taban=0x{base:08x}")


if __name__ == "__main__":
    main()
